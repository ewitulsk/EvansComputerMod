package com.example.evanscomputermod.voice.client.compat;

//? if <=1.21.1 {
import com.example.evanscomputermod.voice.client.CaptureBuffer;
import de.maxhenkel.voicechat.api.ForgeVoicechatPlugin;
import de.maxhenkel.voicechat.api.VoicechatPlugin;
import de.maxhenkel.voicechat.api.events.ClientSoundEvent;
import de.maxhenkel.voicechat.api.events.EventRegistration;

/**
 * Simple Voice Chat plugin (found by Simple Voice Chat through the annotation;
 * never loaded without it). When the player talks in Simple Voice Chat (its
 * own push-to-talk or voice activation), each captured 20 ms frame is also
 * offered to this mod's microphones, instead of this mod opening the
 * microphone a second time. The frame itself is left as it is.
 */
@ForgeVoicechatPlugin
public final class SimpleVoiceChatBridge implements VoicechatPlugin {

    /** Simple Voice Chat captures at 48 kHz mono. */
    private static final int RATE = 48_000;

    @Override
    public String getPluginId() {
        return "evanscomputermod";
    }

    @Override
    public void registerEvents(EventRegistration registration) {
        registration.registerEvent(ClientSoundEvent.class, e -> CaptureBuffer.offer(e.getRawAudio(), RATE, CaptureBuffer.Source.SIMPLE_VOICE_CHAT));
    }
}
//?}
