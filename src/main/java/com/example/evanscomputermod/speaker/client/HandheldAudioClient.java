package com.example.evanscomputermod.speaker.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.handheld.HandheldPackets;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import net.minecraft.client.Minecraft;
import net.minecraft.client.resources.sounds.AbstractSoundInstance;
import net.minecraft.client.resources.sounds.Sound;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.resources.sounds.TickableSoundInstance;
import net.minecraft.client.sounds.AudioStream;
import net.minecraft.client.sounds.SoundBufferLibrary;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.client.sounds.WeighedSoundEvents;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.valueproviders.ConstantFloat;

import java.util.concurrent.CompletableFuture;

/**
 * Plays the handheld radio's audio: one non-positional streaming sound (it is in
 * your hand), fed by the server's audio blocks. Also keeps the last meter
 * reading for the HUD.
 */
public final class HandheldAudioClient {

    private static PcmStream stream;
    private static Instance sound;
    private static long lastAudioMs;
    public static volatile float signalDbm = -200;
    public static volatile double freqHz;

    private HandheldAudioClient() {}

    public static void onAudio(HandheldPackets.Audio p) {
        short[] pcm = ImaAdpcm.decode(p.data(), p.samples());
        if (pcm == null) return;
        Minecraft mc = Minecraft.getInstance();
        if (stream == null || stream.rate() != p.rate() || sound == null || sound.stopped) {
            stop();
            stream = new PcmStream(p.rate());
            sound = new Instance(stream);
            mc.getSoundManager().play(sound);
        }
        stream.push(pcm);
        lastAudioMs = System.currentTimeMillis();
        signalDbm = p.signalDbm();
        freqHz = p.freqHz();
    }

    /** Called every client tick: stop when the server stopped sending (radio off / put away). */
    public static void tick() {
        if (stream != null && System.currentTimeMillis() - lastAudioMs > 1000) stop();
    }

    public static boolean playing() {
        return stream != null;
    }

    private static void stop() {
        if (sound != null) {
            sound.stopped = true;
            Minecraft.getInstance().getSoundManager().stop(sound);
        }
        if (stream != null) stream.close();
        sound = null;
        stream = null;
        signalDbm = -200;
    }

    private static final class Instance extends AbstractSoundInstance implements TickableSoundInstance {
        private static final ResourceLocation ID = ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "handheld_radio");
        private final PcmStream pcm;
        boolean stopped;

        Instance(PcmStream pcm) {
            super(ID, SoundSource.RECORDS, SoundInstance.createUnseededRandom());
            this.pcm = pcm;
            this.relative = true;
            this.attenuation = Attenuation.NONE;
            this.volume = 1f;
            this.pitch = 1f;
        }

        @Override
        public WeighedSoundEvents resolve(SoundManager manager) {
            this.sound = new Sound(ID, ConstantFloat.of(1f), ConstantFloat.of(1f), 1, Sound.Type.FILE, true, false, 16);
            return new WeighedSoundEvents(ID, null);
        }

        @Override
        public CompletableFuture<AudioStream> getStream(SoundBufferLibrary buffers, Sound s, boolean looping) {
            return CompletableFuture.completedFuture(pcm);
        }

        @Override public boolean canStartSilent() { return true; }
        @Override public boolean isStopped() { return stopped; }
        @Override public void tick() {}
    }
}
//?}
