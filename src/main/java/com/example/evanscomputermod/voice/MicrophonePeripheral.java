package com.example.evanscomputermod.voice;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * A microphone as a peripheral (type {@code microphone}): the Microphone block,
 * or the ePhone's built-in one. The sound itself goes through the device files:
 * {@code /dev/mic.<name>} for PCM and {@code /dev/micctl.<name>} for settings
 * ({@code /dev/mic} is the first microphone). These methods are for finding the
 * device and setting the input volume.
 */
public final class MicrophonePeripheral extends AnnotatedPeripheral {

    public static final String TYPE = "microphone";

    /** What the peripheral needs from the microphone that owns it. */
    public interface Owner {
        MicAudio mic();

        /** The gain changed: save it. */
        void markGainChanged();
    }

    private final Owner owner;

    public MicrophonePeripheral(Owner owner) {
        this.owner = owner;
    }

    public MicAudio mic() {
        return owner.mic();
    }

    /** The gain changed through /dev/micctl. */
    public void gainChanged() {
        owner.markGainChanged();
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @PeripheralMethod(mainThread = false,
            description = "Format and state: {device, ctl, rate, bits, channels, gain, buffered_ms, overruns, level_db, listening}")
    public Map<String, Object> getInfo(IComputerAccess computer) {
        MicAudio m = owner.mic();
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("device", "/dev/mic." + computer.getAttachmentName());
        out.put("ctl", "/dev/micctl." + computer.getAttachmentName());
        out.put("rate", m.rate());
        out.put("bits", m.bits());
        out.put("channels", m.channels());
        out.put("gain", m.gain());
        out.put("buffered_ms", m.bufferedMs());
        out.put("overruns", m.overruns());
        out.put("level_db", m.levelDb());
        out.put("listening", m.listening());
        return out;
    }

    @PeripheralMethod(mainThread = false, description = "Set the input volume, 0-100 (50 = the capsule's level, 0 = mute)")
    public void setGain(int gain) throws PeripheralException {
        try {
            owner.mic().setGain(gain);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        owner.markGainChanged();
    }

    @PeripheralMethod(mainThread = false, description = "Input volume, 0-100")
    public int getGain() {
        return owner.mic().gain();
    }
}
