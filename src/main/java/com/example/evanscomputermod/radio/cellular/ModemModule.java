package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.IModuleItemTarget;
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * An LTE or mmWave modem in a module bay: the {@link ModemLink} engine on the
 * shared radio medium, presented to the kernel as {@code wwan0} through
 * {@link CellRadio} (the {@code cell_*} host functions), and to programs as a
 * peripheral ({@code status}, {@code cells}, {@code get_sim}; the {@code cell}
 * command uses them).
 *
 * <p><b>SIM.</b> In a Terminal the modem holds the SIM itself: right-click its
 * bay with a SIM card to put one in (the old one pops out), and it travels with
 * the module (ejecting the module or breaking the computer keeps it). A host
 * with its own SIM tray implements {@link SimProvider} on its module host, or
 * calls {@link #setSimProvider}; the modem then uses the tray's SIM and ignores
 * its own slot. Sneak-right-click the air holding a loose modem to take its SIM
 * out ({@link ModemModuleItem}).
 *
 * <p>The antenna follows the host every tick ({@link ModemPose}), so a modem on a
 * moving Sable ship (or, later, in a held phone) hands over between towers.
 */
public final class ModemModule extends AnnotatedPeripheral implements IComputerModule, CellRadio, IModuleItemTarget {
    public static final String TYPE_LTE = "lte_modem", TYPE_MMWAVE = "mmwave_modem";

    private final IModuleHost host;
    private final CellTech tech;
    private final ModemLink link;
    private final byte[] mac;
    private volatile SimCard ownSim;
    private volatile SimProvider provider;
    private volatile boolean live;

    public ModemModule(IModuleHost host, CellTech tech, CompoundTag saved) {
        this.host = host;
        this.tech = tech;
        long stored = saved.getLong("mac");
        if (stored == 0) {
            UUID u = UUID.randomUUID();
            stored = u.getMostSignificantBits() ^ u.getLeastSignificantBits();
        }
        mac = new byte[6];
        for (int i = 0; i < 6; i++) mac[i] = (byte) (stored >>> (8 * (5 - i)));
        mac[0] = (byte) ((mac[0] & 0xfc) | 0x02);   // locally administered, unicast
        this.ownSim = CellNbt.readSim(saved.getCompound("sim"));
        this.link = new ModemLink(UUID.randomUUID(), tech, mac, RadioMediumHooks::medium);
        if (host instanceof SimProvider p) provider = p;
    }

    public static String typeOf(CellTech tech) {
        return tech == CellTech.LTE ? TYPE_LTE : TYPE_MMWAVE;
    }

    @Override
    public String getType() {
        return typeOf(tech);
    }

    public CellTech tech() {
        return tech;
    }

    /** The engine (status, tests). */
    public ModemLink link() {
        return link;
    }

    // ------------------------------------------------------------ SIM

    /** Use {@code p}'s SIM instead of the modem's own slot (null = back to the own slot). */
    public void setSimProvider(SimProvider p) {
        provider = p;
    }

    /** The SIM in use now: the provider's, else the modem's own. */
    public SimCard currentSim() {
        SimProvider p = provider;
        return p != null ? p.sim() : ownSim;
    }

    /** The SIM held in the modem's own slot. */
    public SimCard ownSim() {
        return ownSim;
    }

    @Override
    public boolean acceptsItem(ItemStack stack) {
        return SimCardItem.isIssued(stack) && provider == null;
    }

    @Override
    public Component useItem(ItemStack stack, Player player) {
        SimCard in = SimCardItem.sim(stack);
        if (in == null) return Component.translatable("message.evanscomputermod.modem.blank_sim");
        SimCard old = ownSim;
        ownSim = in;
        if (!player.getAbilities().instabuild) stack.shrink(1);
        if (old != null && !old.equals(in)) {
            ItemStack back = SimCardItem.issued(old);
            if (!player.getInventory().add(back)) player.drop(back, false);
        }
        host.markDirty();
        return Component.translatable("message.evanscomputermod.modem.sim_in", in.carrier(), in.imsiText());
    }

    // ------------------------------------------------------------ CellRadio (kernel)

    @Override
    public boolean cellActive() {
        return live;
    }

    @Override
    public ModemLink modem() {
        return link;
    }

    // ------------------------------------------------------------ module lifecycle

    @Override
    public void onLoad() {
        live = true;
        link.start();
        link.setPose(ModemPose.of(host));
    }

    @Override
    public void onUnload() {
        live = false;
        link.stop();
    }

    @Override
    public void tick() {
        live = host.isAlive();
        if (!live) return;
        link.setSim(currentSim());
        link.setPose(ModemPose.of(host));
        link.tick();
    }

    @Override
    public void saveState(CompoundTag tag) {
        long v = 0;
        for (byte b : mac) v = v << 8 | (b & 0xff);
        tag.putLong("mac", v);
        SimCard s = ownSim;
        if (s != null) tag.put("sim", CellNbt.writeSim(s));
    }

    // ------------------------------------------------------------ peripheral API

    @PeripheralMethod(description = "{tech, state, message, sim_carrier, imsi, mac, carrier, tower, channel, frequency_mhz, rssi_dbm, sinr_db, bars, dl_cqi, dl_mbps, ul_cqi, ul_mbps, rnti, slice, tx_bytes, rx_bytes, handovers}", mainThread = false)
    public Map<String, Object> status() {
        return link.status();
    }

    @PeripheralMethod(description = "Towers heard recently, strongest first: [{carrier, tech, channel, rssi_dbm, sinr_db, load, cell, tower}]", mainThread = false)
    public List<Map<String, Object>> cells() {
        List<Map<String, Object>> out = new ArrayList<>();
        for (ModemLink.Cell c : link.cells()) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("carrier", c.carrier());
            m.put("tech", c.tech().displayName);
            m.put("channel", c.channel());
            m.put("rssi_dbm", Math.round(c.rssiDbm() * 10) / 10.0);
            m.put("sinr_db", Double.isNaN(c.sinrDb()) ? null : Math.round(c.sinrDb() * 10) / 10.0);
            m.put("load", c.load());
            m.put("cell", String.format("%08x", c.cellKey()));
            m.put("tower", c.towerId().toString());
            out.add(m);
        }
        return out;
    }

    @PeripheralMethod(description = "The SIM in use: {carrier, imsi}, or nil", mainThread = false)
    public Map<String, Object> get_sim() {
        SimCard s = currentSim();
        if (s == null) return null;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("carrier", s.carrier());
        m.put("imsi", s.imsiText());
        return m;
    }

    @PeripheralMethod(description = "The modem's MAC address (wwan0)", mainThread = false)
    public String get_mac() {
        return TowerLink.macText(mac);
    }

    @PeripheralMethod(description = "Recent attach / handover / loss events", mainThread = false)
    public List<String> events() {
        return link.events();
    }
}
//?}
