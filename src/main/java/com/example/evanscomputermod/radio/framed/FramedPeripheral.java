package com.example.evanscomputermod.radio.framed;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * The peripheral methods of a framed radio ({@link FramedRadio}), shared by every
 * host: the SDR blocks ({@code sdr}, framed mode on the Advanced SDR) and the modem
 * modules in framed-radio form ({@code framed_radio}). Frames themselves go through
 * the device file ({@link #framesPath}); events: {@code framed_rx} when records
 * start waiting in it.
 */
public abstract class FramedPeripheral extends AnnotatedPeripheral {

    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();

    /** The framed radio (configured even while a host such as an SDR is in another mode). */
    public abstract FramedRadio framed();

    /** The frames device a computer opens for this attachment. */
    protected String framesPath(IComputerAccess computer) {
        return "/dev/frames." + computer.getAttachmentName();
    }

    /** Called after any framed setting changed (hosts save it). */
    protected void framedChanged() {}

    /** Forward an event to every attached computer (the hub adds the attachment name). */
    public void event(String name) {
        for (IComputerAccess c : computers) c.queueEvent(name);
    }

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
    }

    private interface Change {
        void run();
    }

    private void change(Change c) throws PeripheralException {
        try {
            c.run();
        } catch (RuntimeException e) {
            throw new PeripheralException(e.getMessage());
        }
        framedChanged();
    }

    @PeripheralMethod(mainThread = false, description = "Framed PHY profile: 'lte' or 'mmwave' (channel 1, full bandwidth, default power)")
    public void set_phy(String profile) throws PeripheralException {
        change(() -> framed().setProfile(PhyProfile.byId(profile)));
    }

    @PeripheralMethod(mainThread = false, description = "Framed role: 'bs' (transmit downlink, hear uplink) or 'ue' (transmit uplink, hear downlink)")
    public void set_role(String role) throws PeripheralException {
        change(() -> framed().setRole(FramedRadio.parseRole(role)));
    }

    @PeripheralMethod(mainThread = false, description = "Framed channel number in the profile's plan (LTE 1-6, mmWave 1-4)")
    public void set_channel(int channel) throws PeripheralException {
        change(() -> framed().setChannel(channel));
    }

    @PeripheralMethod(mainThread = false, description = "Framed carrier bandwidth, Hz (LTE 1.4/3/5/10 MHz, mmWave 50/100 MHz)")
    public void set_phy_bandwidth(double hz) throws PeripheralException {
        change(() -> framed().setBandwidth(hz));
    }

    @PeripheralMethod(mainThread = false, description = "Framed transmit power, dBm (capped at what the hardware and its amplifier give)")
    public void set_power(double dbm) throws PeripheralException {
        change(() -> framed().setPower(dbm));
    }

    @PeripheralMethod(mainThread = false, description = "This radio's 32-bit frame address, hex (frames to it or to ffffffff are delivered)")
    public void set_address(String hex) throws PeripheralException {
        change(() -> framed().setAddress(FramedRadio.parseAddress(hex)));
    }

    @PeripheralMethod(mainThread = false, description = "Deliver frames addressed to anyone (true) or only to this radio and broadcast (false)")
    public void set_promiscuous(boolean on) throws PeripheralException {
        change(() -> framed().setPromiscuous(on));
    }

    @PeripheralMethod(mainThread = false, description = "Repeat a frame every period_ms on the airtime clock (index 0-3; period 0 stops it)")
    public void set_beacon(int index, int periodMs, byte[] payload) throws PeripheralException {
        FramedRadio r = framed();
        if (!r.active()) throw new PeripheralException("framed mode is off");
        try {
            r.beacon(new FramedRecords.BeaconRequest(index, periodMs, 0, 0, payload == null ? new byte[0] : payload));
        } catch (RuntimeException e) {
            throw new PeripheralException(e.getMessage());
        }
    }

    @PeripheralMethod(mainThread = false, description = "Framed settings and counters: {framed, phy, role, channel, phy_bw, tx_freq, rx_freq, power_dbm, max_power_dbm, address, promisc, slot_us, slot, ta_us, tx_frames, tx_bytes, rx_frames, rx_bytes, rx_collisions, rx_dropped, harq_acked, harq_retx, harq_failed, beacons_sent, blocked, beacons, frames}")
    public Map<String, Object> framed_status(IComputerAccess computer) {
        framed().pump();
        Map<String, Object> m = new LinkedHashMap<>(framed().status());
        m.put("frames", framesPath(computer));
        return m;
    }

    @PeripheralMethod(mainThread = false, description = "Peers heard: {address: {rssi_dbm, sinr_db, cqi, reported_cqi, age_ms, delay_us, frames}}")
    public Map<String, Object> peers() {
        framed().pump();
        return framed().peers();
    }

    @PeripheralMethod(mainThread = false, description = "The PHY profiles: [{id, name, channels, plan, bandwidths, subchannels, slot_us, bs_power_dbm, ue_power_dbm, max_cqi}]")
    public List<Map<String, Object>> profiles() {
        return FramedRadio.profiles();
    }

    @PeripheralMethod(mainThread = false, description = "The current slot number on the airtime clock (schedule TX records for a later one)")
    public long slot() {
        return framed().slot();
    }
}
