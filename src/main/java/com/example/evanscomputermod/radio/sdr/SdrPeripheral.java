package com.example.evanscomputermod.radio.sdr;

import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.framed.FramedPeripheral;
import com.example.evanscomputermod.radio.framed.FramedRadio;

import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.function.Supplier;

/**
 * An SDR as a peripheral (type {@code sdr}). Samples flow through the device
 * files {@code /dev/sdr<N>} (read = receive, write = transmit; interleaved
 * {@code cs16} or {@code cf32}) and {@code /dev/sdrctl<N>}; these methods tune
 * and inspect it. Events: {@code sdr_overflow}, {@code sdr_underflow}.
 *
 * <p><b>Framed PHY mode</b> (Advanced SDR only): {@code set_mode("framed")} turns the
 * SDR into a {@link FramedRadio} (LTE / mmWave frames with measurements through
 * {@code /dev/sdr<N>.frames}; the {@link FramedPeripheral} methods configure it) and
 * {@code set_mode("iq")} back into a sample radio. One mode at a time: in framed mode
 * {@code /dev/sdr<N>} fails with EIO, in IQ mode the frames file does.
 */
public final class SdrPeripheral extends FramedPeripheral {
    public static final String TYPE = "sdr";
    public enum Mode { IQ, FRAMED }

    private final SdrRadio radio;
    private final FramedRadio framed;
    private final Supplier<RadioMedium> medium;
    private final Runnable onChange;
    private volatile Mode mode = Mode.IQ;

    public SdrPeripheral(SdrRadio radio, Supplier<RadioMedium> medium, Runnable onChange) {
        this(radio, null, medium, onChange);
    }

    /** {@code framed} is the SDR's framed radio, or null if this SDR has no framed mode. */
    public SdrPeripheral(SdrRadio radio, FramedRadio framed, Supplier<RadioMedium> medium, Runnable onChange) {
        this.radio = radio;
        this.framed = framed;
        this.medium = medium;
        this.onChange = onChange;
    }

    /** True if this SDR's tier has framed PHY mode (the Advanced SDR). */
    public static boolean framedCapable(SdrTier tier) {
        return tier == SdrTier.ADVANCED;
    }

    @Override
    public FramedRadio framed() {
        if (framed == null) throw new IllegalStateException(radio.tier().id() + " has no framed PHY mode (Advanced SDR only)");
        return framed;
    }

    /** The framed radio if this SDR is in framed mode now, else null. */
    public FramedRadio framedIfActive() {
        return mode == Mode.FRAMED ? framed : null;
    }

    public Mode mode() {
        return mode;
    }

    /** Switch between IQ and framed mode ("iq" / "framed"). */
    public void setMode(String name) {
        Mode m = switch (name == null ? "" : name.trim().toLowerCase(Locale.ROOT)) {
            case "iq", "samples" -> Mode.IQ;
            case "framed", "frames" -> Mode.FRAMED;
            default -> throw new IllegalArgumentException("mode must be 'iq' or 'framed', got '" + name + "'");
        };
        if (m == Mode.FRAMED && framed == null)
            throw new IllegalArgumentException(radio.tier().id() + " has no framed PHY mode (Advanced SDR only)");
        if (m == Mode.FRAMED) radio.setTx(false, radio.tier().maxTxDbm);
        mode = m;
        if (framed != null) framed.setActive(m == Mode.FRAMED);
        onChange.run();
    }

    /**
     * One {@code /dev/sdrctl} line: {@code mode iq|framed} and the framed lines
     * ({@link FramedRadio#control}) here, everything else is the IQ radio's.
     */
    public void control(String line) {
        String[] p = line.trim().split("\\s+");
        if (p.length == 0 || p[0].isEmpty()) return;
        if (p[0].equals("mode")) {
            if (p.length < 2) throw new IllegalArgumentException("mode needs iq or framed");
            setMode(p[1]);
            return;
        }
        if (framed != null && framed.control(p)) {
            onChange.run();
            return;
        }
        radio.control(line);
    }

    /** The {@code /dev/sdrctl} text: the IQ settings, the mode, and in framed mode the framed settings. */
    public String describe() {
        String s = radio.describe() + "mode " + mode.name().toLowerCase(Locale.ROOT) + "\n";
        return mode == Mode.FRAMED ? s + framed.describe() : s;
    }

    @Override
    protected void framedChanged() {
        onChange.run();
    }

    @Override
    protected String framesPath(IComputerAccess computer) {
        return "/dev/sdr." + computer.getAttachmentName() + ".frames";
    }

    public SdrRadio radio() {
        return radio;
    }

    public RadioMedium medium() {
        return medium.get();
    }

    @Override
    public String getType() {
        return TYPE;
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

    @PeripheralMethod(mainThread = false, description = "Mode: 'iq' (raw samples through /dev/sdr: you are the modem) or 'framed' (Advanced SDR: LTE / mmWave frames"
            + " through /dev/sdr<N>.frames, the radio does the PHY). See docs/radio/SDR_PROGRAMS.md 'When to use which mode'")
    public void set_mode(String mode) throws PeripheralException {
        try { setMode(mode); } catch (RuntimeException e) { throw wrap(e); }
    }

    @PeripheralMethod(mainThread = false, description = "The current mode: 'iq' or 'framed'")
    public String get_mode() {
        return mode.name().toLowerCase(Locale.ROOT);
    }

    @PeripheralMethod(mainThread = false, description = "Current sample counter on the world sample clock (sync rx and tx)")
    public long timestamp() {
        return radio.timestamp();
    }

    @PeripheralMethod(mainThread = false, description = "Settings and counters: {tier, freq, rate, max_rate, bw, gain, agc, format, tx, tx_power_dbm, adc_bits, timestamp, read, written, overflows, underflows, device, ctl, mode, framed_capable} (numbers and booleans as such)")
    public Map<String, Object> info(IComputerAccess computer) {
        Map<String, Object> m = new LinkedHashMap<>(radio.settings());
        m.put("device", "/dev/sdr." + computer.getAttachmentName());
        m.put("ctl", "/dev/sdrctl." + computer.getAttachmentName());
        m.put("mode", mode.name().toLowerCase(Locale.ROOT));
        m.put("framed_capable", framed != null);
        return m;
    }
}
