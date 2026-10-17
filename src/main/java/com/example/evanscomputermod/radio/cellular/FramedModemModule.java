package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.IModuleItemTarget;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.card.SimPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.framed.FramedPeripheral;
import com.example.evanscomputermod.radio.framed.FramedRadio;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;

import java.util.Map;
import java.util.UUID;

/**
 * An LTE / mmWave Modem module: a bare framed-PHY radio (Cellular v2), the same
 * {@link FramedRadio} an Advanced SDR runs in framed mode, built into a module, with
 * no protocol of its own. Software ({@code celld}) drives it through the
 * {@link FramedPeripheral} methods and {@code /dev/frames.<bay>}.
 *
 * <p>Hardware: the profile is the modem's technology (fixed: an LTE modem has no
 * 28 GHz front end), role UE by default, transmit power capped at the profile's
 * device power (23 dBm), the device antenna of {@link CellAntennas#modem} at the
 * host's pose (a block computer, a Sable ship, a held ePhone).
 *
 * <p>SIM: on a Terminal the modem has its own SIM slot (right-click its bay with a
 * written SIM; sneak-right-click the loose modem item to take it out), exposed to the
 * computer as the companion peripheral {@code <bay>_sim} (type {@code sim},
 * {@link SimPeripheral}). The card itself is kept on the server ({@link ModemSimStore});
 * the module's state, which clients see, holds only its identity and slot id. On a host with a SIM tray (the ePhone,
 * {@link #setSimProvider}) the tray is the SIM: the phone exposes it as {@code sim}
 * and the modem has no slot of its own.
 */
public final class FramedModemModule extends FramedPeripheral implements IComputerModule, IModuleItemTarget {
    public static final String TYPE = "framed_radio";

    private final IModuleHost host;
    private final PhyProfile tech;
    private final UUID id = UUID.randomUUID();
    /** Made in the constructor, after {@link #tech} (its antenna depends on it). */
    private final Endpoint endpoint;
    private final FramedRadio radio;
    private volatile Pose pose;
    private RadioMedium registeredWith;
    /** The SIM in the modem's own slot: its slot id in {@link ModemSimStore} and the card (EMPTY with none). */
    private volatile String simSlot;
    private volatile ItemStack ownSim = ItemStack.EMPTY;
    private volatile SimProvider provider;
    private final SimPeripheral simPeripheral = new SimPeripheral(SimPeripheral.ofCard(this::currentSim));

    public FramedModemModule(IModuleHost host, PhyProfile tech, CompoundTag saved) {
        this.host = host;
        this.simSlot = ModemSimStore.slotOf(saved.getCompound("sim"));
        this.ownSim = ModemSimStore.peek(simSlot);
        if (ownSim.isEmpty()) simSlot = null;   // given back from a copy of this modem, or on a client
        if (host instanceof SimProvider p) provider = p;
        this.tech = tech;
        this.endpoint = new Endpoint();
        this.radio = new FramedRadio(new Host(), id);
        radio.setProfile(tech);
        CompoundTag f = saved.getCompound("framed");
        try {
            if (f.contains("Role")) radio.setRole(FramedRadio.parseRole(f.getString("Role")));
            if (f.contains("Channel")) radio.setChannel(f.getInt("Channel"));
            if (f.contains("Bw")) radio.setBandwidth(f.getDouble("Bw"));
            if (f.contains("Power")) radio.setPower(f.getDouble("Power"));
            if (f.getInt("Address") != 0) radio.setAddress(f.getInt("Address"));
        } catch (RuntimeException ignored) {
            // keep defaults for the rest
        }
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public FramedRadio framed() {
        return radio;
    }

    public PhyProfile tech() {
        return tech;
    }

    /** Where the modem's antenna is now (the host's pose), or null before it loaded. */
    public Pose pose() {
        return pose;
    }

    // ------------------------------------------------------------ SIM

    /** Use {@code p}'s SIM (a phone's tray) instead of the modem's own slot; null = the own slot. Before the module is attached. */
    public void setSimProvider(SimProvider p) {
        provider = p;
    }

    /** The SIM in use: the provider's, else the modem's own slot (server side: it holds the key). */
    public SimCard currentSim() {
        SimProvider p = provider;
        return p != null ? p.sim() : SimCardItem.sim(ownSim);
    }

    /** The SIM card in the modem's own slot (EMPTY with none); do not change it. */
    public ItemStack ownSim() {
        return ownSim;
    }

    /** The SIM access peripheral of this modem's own slot. */
    public SimPeripheral simPeripheral() {
        return simPeripheral;
    }

    @Override
    public Map<String, IPeripheral> companions() {
        return provider == null ? Map.of("sim", simPeripheral) : Map.of();
    }

    @Override
    public boolean acceptsItem(ItemStack stack) {
        return SimCardItem.isIssued(stack) && provider == null;
    }

    @Override
    public Component useItem(ItemStack stack, Player player) {
        com.example.evanscomputermod.card.SimIdentity in = SimCardItem.identity(stack);
        if (in == null) return Component.translatable("message.evanscomputermod.modem.blank_sim");
        String slot = ModemSimStore.put(stack);
        if (slot == null) return Component.translatable("message.evanscomputermod.no_module_takes_item");
        String oldSlot = simSlot;
        simSlot = slot;
        ownSim = ModemSimStore.peek(slot);
        if (!player.getAbilities().instabuild) stack.shrink(1);
        ItemStack back = ModemSimStore.take(oldSlot);
        if (!back.isEmpty() && !player.getInventory().add(back)) player.drop(back, false);
        host.markDirty();
        return Component.translatable("message.evanscomputermod.modem.sim_in", in.carrier(), in.imsiText());
    }

    @Override
    protected void framedChanged() {
        if (radio.profile() != tech) radio.setProfile(tech);   // the module's front end is fixed
        host.markDirty();
    }

    @Override
    public void set_phy(String profile) throws com.example.evanscomputermod.api.peripheral.PeripheralException {
        if (PhyProfile.byId(profile) != tech)
            throw new com.example.evanscomputermod.api.peripheral.PeripheralException("this modem is " + tech.id + " only");
    }

    @Override
    protected String framesPath(IComputerAccess computer) {
        return "/dev/frames." + computer.getAttachmentName();
    }

    // ------------------------------------------------------------ module lifecycle

    @Override
    public void onLoad() {
        radio.setActive(true);
        pose = ModemPose.of(host);
    }

    @Override
    public void onUnload() {
        radio.setActive(false);
        if (registeredWith != null) registeredWith.unregister(endpoint);
        registeredWith = null;
    }

    @Override
    public void tick() {
        if (!host.isAlive()) return;
        pose = ModemPose.of(host);
        RadioMedium m = RadioMediumHooks.medium();
        if (m != registeredWith) {
            if (registeredWith != null) registeredWith.unregister(endpoint);
            registeredWith = m;
            if (m != null) m.register(endpoint);
        }
        radio.pump();
    }

    @Override
    public void saveState(CompoundTag tag) {
        String slot = simSlot;
        com.example.evanscomputermod.card.SimIdentity sim = SimCardItem.identity(ownSim);
        if (slot != null && sim != null) tag.put("sim", ModemSimStore.writeRef(sim, slot));
        CompoundTag f = new CompoundTag();
        f.putString("Role", radio.role().name());
        f.putInt("Channel", radio.channel());
        f.putDouble("Bw", radio.bandwidthHz());
        if (!Double.isNaN(radio.requestedPowerDbm())) f.putDouble("Power", radio.requestedPowerDbm());
        f.putInt("Address", radio.address());
        tag.put("framed", f);
    }

    private final class Host implements FramedRadio.Host {
        @Override public RadioEndpoint endpoint() { return endpoint; }
        @Override public RadioMedium medium() { return RadioMediumHooks.medium(); }
        @Override public double maxTxDbm() { return tech.ueTxDbm; }
        @Override public double noiseFigureDb() { return tech.ueNoiseFigureDb; }
        @Override public void event(String name) { FramedModemModule.this.event(name); }
        @Override public void page() { host.wakeForPage(); }
    }

    private final class Endpoint implements RadioEndpoint {
        private final AntennaPattern antenna = CellAntennas.modem(tech);
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return antenna; }
        @Override public Channel tunedChannel() { return pose == null || !radio.active() ? null : radio.rxChannel(); }
        @Override public double noiseFigureDb() { return tech.ueNoiseFigureDb; }
        @Override public double sensitivityDbm() { return radio.sensitivityDbm(); }
        @Override public double maxTxPowerDbm() { return tech.ueTxDbm; }
        @Override public void onReceive(Reception reception) { radio.onReceive(reception); }
    }
}
//?}
