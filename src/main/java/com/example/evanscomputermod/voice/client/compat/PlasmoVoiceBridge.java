package com.example.evanscomputermod.voice.client.compat;

//? if <=1.21.1 {
import com.example.evanscomputermod.voice.client.CaptureBuffer;
import su.plo.voice.api.addon.AddonLoaderScope;
import su.plo.voice.api.addon.InjectPlasmoVoice;
import su.plo.voice.api.addon.annotation.Addon;
import su.plo.voice.api.client.PlasmoVoiceClient;
import su.plo.voice.api.client.audio.capture.ClientActivation;
import su.plo.voice.api.client.event.audio.capture.AudioCaptureProcessedEvent;
import su.plo.voice.api.event.EventSubscribe;

/**
 * Plasmo Voice client add-on (loaded only when Plasmo Voice is installed).
 * Plasmo Voice owns the microphone and its settings: its input device, its
 * gain and filters, and its activation (push-to-talk or voice activation).
 * So this mod takes each frame after Plasmo Voice has processed it, and only
 * while Plasmo Voice's own activation is on (the player is talking in Plasmo
 * Voice); it never opens the microphone or applies settings of its own. The
 * frame itself is left as it is.
 */
@Addon(id = "evanscomputermod", name = "Evans Computer Mod", version = "1.0.0", authors = {"Evan"}, scope = AddonLoaderScope.CLIENT)
public final class PlasmoVoiceBridge {

    private static PlasmoVoiceBridge instance;

    @InjectPlasmoVoice
    private PlasmoVoiceClient voiceClient;

    /** Register with Plasmo Voice (client mod setup). */
    public static void install() {
        instance = new PlasmoVoiceBridge();
        PlasmoVoiceClient.getAddonsLoader().load(instance);
    }

    private ClientActivation parent() {
        return voiceClient == null ? null : voiceClient.getActivationManager().getParentActivation().orElse(null);
    }

    /** Plasmo Voice's push-to-talk key is held (a Walkie-Talkie with no key of its own uses it). */
    public static boolean pttPressed() {
        ClientActivation a = instance == null ? null : instance.parent();
        return a != null && a.getPttKey() != null && a.getPttKey().isPressed();
    }

    @EventSubscribe
    public void onCapture(AudioCaptureProcessedEvent e) {
        if (!CaptureBuffer.accepting()) return;
        ClientActivation a = parent();
        if (a == null || !a.isActive()) return;   // not talking in Plasmo Voice
        short[] mono = e.getProcessed().getMono();
        if (mono == null) return;
        CaptureBuffer.offer(mono, (int) e.getDevice().getFormat().getSampleRate(), CaptureBuffer.Source.PLASMO_VOICE);
    }
}
//?}
