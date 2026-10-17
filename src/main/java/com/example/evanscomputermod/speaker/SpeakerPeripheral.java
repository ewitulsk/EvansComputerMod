package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * A Speaker as a peripheral (type {@code speaker}). The sound itself goes
 * through the device files: {@code /dev/audio.<name>} for PCM and
 * {@code /dev/audioctl.<name>} for settings ({@code /dev/audio} is the first
 * speaker). These methods are for finding the device and for one-off sounds.
 */
public final class SpeakerPeripheral extends AnnotatedPeripheral {

    public static final String TYPE = "speaker";

    /** What the peripheral needs from the speaker that owns it. */
    public interface Owner {
        SpeakerAudio audio();

        /** The volume changed: save it. */
        void markVolumeChanged();

        /** Play a Minecraft sound once; false if there's no such sound. Server thread. */
        boolean playSound(String id, float volume, float pitch);
    }

    private final Owner speaker;

    public SpeakerPeripheral(Owner speaker) {
        this.speaker = speaker;
    }

    public SpeakerAudio audio() {
        return speaker.audio();
    }

    /** The volume was changed elsewhere (the Volume Mixer): let the owner save it. */
    public void volumeChanged() {
        speaker.markVolumeChanged();
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @PeripheralMethod(mainThread = false,
            description = "Format and state: {device, ctl, rate, bits, channels, volume, latency, buffered_ms, underruns}")
    public Map<String, Object> getInfo(IComputerAccess computer) {
        SpeakerAudio a = speaker.audio();
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("device", "/dev/audio." + computer.getAttachmentName());
        out.put("ctl", "/dev/audioctl." + computer.getAttachmentName());
        out.put("rate", a.rate());
        out.put("bits", a.bits());
        out.put("channels", a.channels());
        out.put("volume", a.volume());
        out.put("latency", a.latencyMs());
        out.put("buffered_ms", a.bufferedMs());
        out.put("underruns", a.underruns());
        return out;
    }

    @PeripheralMethod(mainThread = false, description = "Set the output volume, 0-100")
    public void setVolume(int volume) throws PeripheralException {
        try {
            speaker.audio().setVolume(volume);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        speaker.markVolumeChanged();
    }

    @PeripheralMethod(mainThread = false, description = "Output volume, 0-100")
    public int getVolume() {
        return speaker.audio().volume();
    }

    @PeripheralMethod(mainThread = false, description = "Stop: drop the audio that is queued")
    public void stop() {
        speaker.audio().flush();
    }

    @PeripheralMethod(description = "Play a Minecraft sound once, e.g. play_sound('minecraft:block.note_block.harp', 1.0, 1.0)")
    public void playSound(String id, Double volume, Double pitch) throws PeripheralException {
        float v = volume == null ? 1f : (float) Math.max(0, Math.min(3, volume));
        float p = pitch == null ? 1f : (float) Math.max(0.5, Math.min(2, pitch));
        if (!speaker.playSound(id, v, p)) throw new PeripheralException("unknown sound '" + id + "'");
    }

    /** The computer this speaker is attached to (the last one, if several), or null. */
    private volatile java.util.UUID computer;

    @Override
    public void attach(IComputerAccess computer) {
        this.computer = computer.getComputerId();
    }

    @Override
    public void detach(IComputerAccess computer) {
        if (computer.getComputerId().equals(this.computer)) this.computer = null;
    }

    /**
     * The computer this speaker plays for: its playout delay applies to the
     * speaker's audio while it streams timed frames (DISPLAY_PIPELINE_SPEC.md
     * Part E2). Null when attached to nothing.
     */
    public java.util.UUID computer() {
        return computer;
    }
}
