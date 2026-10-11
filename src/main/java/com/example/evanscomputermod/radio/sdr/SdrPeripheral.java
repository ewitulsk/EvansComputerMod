package com.example.evanscomputermod.radio.sdr;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.radio.api.RadioMedium;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;
import java.util.function.Supplier;

/**
 * An SDR as a peripheral (type {@code sdr}). Samples flow through the device
 * files {@code /dev/sdr<N>} (read = receive, write = transmit; interleaved
 * {@code cs16} or {@code cf32}) and {@code /dev/sdrctl<N>}; these methods tune
 * and inspect it. Events: {@code sdr_overflow}, {@code sdr_underflow}.
 */
public final class SdrPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "sdr";

    private final SdrRadio radio;
    private final Supplier<RadioMedium> medium;
    private final Runnable onChange;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();

    public SdrPeripheral(SdrRadio radio, Supplier<RadioMedium> medium, Runnable onChange) {
        this.radio = radio;
        this.medium = medium;
        this.onChange = onChange;
    }

    public SdrRadio radio() {
        return radio;
    }

    public RadioMedium medium() {
        return medium.get();
    }

    /** Forward an SDR event to every attached computer (the hub adds the attachment name). */
    public void event(String name) {
        for (IComputerAccess c : computers) c.queueEvent(name);
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
    }

    private static PeripheralException wrap(RuntimeException e) {
        return new PeripheralException(e.getMessage());
    }

    @PeripheralMethod(mainThread = false, description = "Tune the centre frequency, Hz")
    public void set_frequency(double hz) throws PeripheralException {
        try { radio.setFrequency(hz); } catch (RuntimeException e) { throw wrap(e); }
        onChange.run();
    }

    @PeripheralMethod(mainThread = false, description = "Set the sample rate, samples/s (tier and server caps apply)")
    public void set_sample_rate(int rate) throws PeripheralException {
        try { radio.setSampleRate(rate); } catch (RuntimeException e) { throw wrap(e); }
        onChange.run();
    }

    @PeripheralMethod(mainThread = false, description = "Manual RF gain, 0-60 dB (turns AGC off)")
    public void set_gain(double db) throws PeripheralException {
        try { radio.setGain(db); } catch (RuntimeException e) { throw wrap(e); }
        onChange.run();
    }

    @PeripheralMethod(mainThread = false, description = "Automatic gain control on/off")
    public void set_agc(boolean on) {
        radio.setAgc(on);
        onChange.run();
    }

    @PeripheralMethod(mainThread = false, description = "Receive channel filter bandwidth, Hz (0 = the sample rate): the samples are low-pass filtered to it")
    public void set_bandwidth(double hz) throws PeripheralException {
        try { radio.setBandwidth(hz); } catch (RuntimeException e) { throw wrap(e); }
    }

    @PeripheralMethod(mainThread = false, description = "Sample format of /dev/sdr: 'cs16' or 'cf32' (anything else is an error)")
    public void set_format(String format) throws PeripheralException {
        try { radio.setFormat(SdrRadio.parseFormat(format)); } catch (RuntimeException e) { throw wrap(e); }
    }

    @PeripheralMethod(mainThread = false, description = "Enable/disable transmit at a power in dBm (exciter caps at 5 W = 37 dBm)")
    public void tx_enable(boolean on, Double powerDbm) throws PeripheralException {
        try { radio.setTx(on, powerDbm == null ? radio.tier().maxTxDbm : powerDbm); } catch (RuntimeException e) { throw wrap(e); }
    }

    @PeripheralMethod(mainThread = false, description = "Current sample counter on the world sample clock (sync rx and tx)")
    public long timestamp() {
        return radio.timestamp();
    }

    @PeripheralMethod(mainThread = false, description = "Settings and counters: {tier, freq, rate, max_rate, bw, gain, agc, format, tx, tx_power_dbm, adc_bits, timestamp, read, written, overflows, underflows, device, ctl} (numbers and booleans as such)")
    public Map<String, Object> info(IComputerAccess computer) {
        Map<String, Object> m = new LinkedHashMap<>(radio.settings());
        m.put("device", "/dev/sdr." + computer.getAttachmentName());
        m.put("ctl", "/dev/sdrctl." + computer.getAttachmentName());
        return m;
    }
}
