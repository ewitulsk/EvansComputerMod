package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
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

import java.util.UUID;

/**
 * An LTE / mmWave Modem module as a bare framed-PHY radio (Cellular v2): the same
 * {@link FramedRadio} an Advanced SDR runs in framed mode, built into a module, with
 * no protocol of its own. It loads instead of {@link ModemModule} while
 * {@code RadioConfig.modemFramedRadio()} is set (server config
 * {@code modem.framedRadio}); software (the coming {@code celld}) drives it through
 * the {@link FramedPeripheral} methods and {@code /dev/frames.<bay>}.
 *
 * <p>Hardware: the profile is the modem's technology (fixed: an LTE modem has no
 * 28 GHz front end), role UE by default, transmit power capped at the profile's
 * device power (23 dBm), the device antenna of {@link CellAntennas#modem} at the
 * host's pose (a block computer, a Sable ship, a held ePhone).
 */
public final class FramedModemModule extends FramedPeripheral implements IComputerModule {
    public static final String TYPE = "framed_radio";

    private final IModuleHost host;
    private final PhyProfile tech;
    private final UUID id = UUID.randomUUID();
    private final Endpoint endpoint = new Endpoint();
    private final FramedRadio radio;
    private volatile Pose pose;
    private RadioMedium registeredWith;
    /** The v1 modem's saved state (MAC, SIM), kept untouched so turning the flag off gives the modem back as it was. */
    private final CompoundTag v1State;

    public FramedModemModule(IModuleHost host, PhyProfile tech, CompoundTag saved) {
        this.host = host;
        this.v1State = saved.copy();
        v1State.remove("framed");
        this.tech = tech;
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
        for (String k : v1State.getAllKeys()) tag.put(k, v1State.get(k).copy());
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
