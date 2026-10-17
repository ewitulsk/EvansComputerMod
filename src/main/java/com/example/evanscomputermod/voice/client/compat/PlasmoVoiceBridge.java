package com.example.evanscomputermod.voice.client.compat;

//? if <=1.21.1 {
import com.example.evanscomputermod.voice.client.CaptureBuffer;
import su.plo.voice.api.addon.AddonLoaderScope;
import su.plo.voice.api.addon.annotation.Addon;
import su.plo.voice.api.client.PlasmoVoiceClient;
import su.plo.voice.api.client.event.audio.capture.AudioCaptureEvent;
import su.plo.voice.api.event.EventSubscribe;

/**
 * Plasmo Voice client add-on (loaded only when Plasmo Voice is installed):
 * every frame Plasmo Voice captures from the microphone is also offered to
 * this mod's microphones, instead of this mod opening the microphone a second
 * time. Plasmo Voice keeps the frame; this mod's own push-to-talk / voice
 * activation decides whether it is sent.
 */
@Addon(id = "evanscomputermod", name = "Evans Computer Mod", version = "1.0.0", authors = {"Evan"}, scope = AddonLoaderScope.CLIENT)
public final class PlasmoVoiceBridge {

    /** Register with Plasmo Voice (client mod setup). */
    public static void install() {
        PlasmoVoiceClient.getAddonsLoader().load(new PlasmoVoiceBridge());
    }

    @EventSubscribe
    public void onCapture(AudioCaptureEvent e) {
        if (!CaptureBuffer.accepting()) return;
        var format = e.getDevice().getFormat();
        short[] mono = CaptureBuffer.mono(e.getSamples(), format.getChannels());
        CaptureBuffer.offer(mono, (int) format.getSampleRate(), CaptureBuffer.Source.PLASMO_VOICE);
    }
}
//?}
