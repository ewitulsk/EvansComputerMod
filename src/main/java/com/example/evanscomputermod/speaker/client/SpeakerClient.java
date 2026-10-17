package com.example.evanscomputermod.speaker.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import com.example.evanscomputermod.speaker.SpeakerAudioPacket;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientTickEvent;

import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;

/** Plays the audio speakers send: one streaming sound per speaker while it has something to play. */
@EventBusSubscriber(modid = EvansComputerMod.MODID, value = Dist.CLIENT)
public final class SpeakerClient {

    /** A speaker that sent nothing for this long stops its sound. */
    private static final long IDLE_STOP_MS = 3_000;

    private static final class Playing {
        final PcmStream stream;
        final SpeakerSoundInstance sound;
        final long startedMs = System.currentTimeMillis();
        long lastAudioMs;
        final com.example.evanscomputermod.speaker.SpeakerSchedule schedule = new com.example.evanscomputermod.speaker.SpeakerSchedule();

        Playing(PcmStream stream, SpeakerSoundInstance sound) {
            this.stream = stream;
            this.sound = sound;
        }
    }

    private static final Map<BlockPos, Playing> PLAYING = new HashMap<>();
    private static Level lastLevel;

    private SpeakerClient() {}

    /** Network thread: onto the render thread with its receive time (decoding is cheap). */
    public static void onAudioPacket(SpeakerAudioPacket p, net.neoforged.neoforge.network.handling.IPayloadContext ctx) {
        com.example.evanscomputermod.client.DisplayClientHooks.toMain(() -> onAudio(p));
    }

    public static void onAudio(SpeakerAudioPacket p) {
        if (p.samples() <= 0 || p.samples() > SpeakerAudioPacket.MAX_SAMPLES) return;
        short[] pcm = ImaAdpcm.decode(p.data(), p.samples());
        if (pcm == null) return;
        Minecraft mc = Minecraft.getInstance();
        Playing playing = PLAYING.get(p.pos());
        boolean ended = playing != null && !mc.getSoundManager().isActive(playing.sound)
                && System.currentTimeMillis() - playing.startedMs > 1_000; // don't retry every packet
        if (playing == null || playing.stream.rate() != p.rate() || ended) {
            if (playing != null) stop(mc, playing);
            PcmStream stream = new PcmStream(p.rate());
            playing = new Playing(stream, new SpeakerSoundInstance(p.pos(), stream));
            PLAYING.put(p.pos().immutable(), playing);
            mc.getSoundManager().play(playing.sound);
        }
        // Lip sync (DISPLAY_PIPELINE_SPEC.md Part E2): while the speaker's computer streams timed
        // frames, its audio plays with the same playout delay as the picture; otherwise at once.
        // A live block (a call) plays at once and drops delayed audio still queued (SpeakerSchedule).
        long now = com.example.evanscomputermod.network.ClockSync.clientNowUs();
        var clock = com.example.evanscomputermod.network.ClockSync.INSTANCE;
        long delay = p.ptsUs() != 0 && clock.known()
                ? com.example.evanscomputermod.network.ClientPlayout.audioDelayUs(p.computer(), now) : -1;
        long lead = delay >= 0 ? clock.toClient(p.ptsUs()) + delay - now : -1;
        var plan = playing.schedule.next(p.live(), delay, lead);
        if (plan.dropQueued()) playing.stream.dropQueued();
        if (plan.now()) playing.stream.pushNow(pcm);
        else playing.stream.pushAt(pcm, plan.leadUs());
        playing.lastAudioMs = System.currentTimeMillis();
    }

    private static void stop(Minecraft mc, Playing playing) {
        playing.sound.stopPlaying();
        playing.stream.close();
        mc.getSoundManager().stop(playing.sound);
    }

    @SubscribeEvent
    public static void onClientTick(ClientTickEvent.Post event) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.level != lastLevel) {
            // Changed world or dimension: nothing of the old one keeps playing.
            for (Playing playing : PLAYING.values()) stop(mc, playing);
            PLAYING.clear();
            lastLevel = mc.level;
        }
        long now = System.currentTimeMillis();
        for (Iterator<Playing> it = PLAYING.values().iterator(); it.hasNext(); ) {
            Playing playing = it.next();
            if (now - playing.lastAudioMs > IDLE_STOP_MS) {
                stop(mc, playing);
                it.remove();
            }
        }
    }
}
