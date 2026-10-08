package com.example.evanscomputermod.radio.handheld;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.sdr.IqSynthesizer;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;
import net.neoforged.neoforge.event.tick.PlayerTickEvent;
import net.neoforged.neoforge.network.PacketDistributor;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Runs every switched-on handheld: each tick it synthesises the IQ the radio
 * hears at the player's position (link budget, interference, noise — the same
 * medium as everything else), demodulates AM or FM and streams the audio to that
 * player only, so weak stations hiss and fade. Nothing runs for radios that are
 * off or not held.
 */
public final class HandheldServer {

    public static final int AUDIO_RATE = 24_000;

    private static final Map<UUID, Session> sessions = new ConcurrentHashMap<>();

    private HandheldServer() {}

    /** Held handheld (main hand first) or empty. */
    public static ItemStack held(ServerPlayer p) {
        for (InteractionHand h : InteractionHand.values()) {
            ItemStack s = p.getItemInHand(h);
            if (s.getItem() instanceof HandheldRadioItem) return s;
        }
        return ItemStack.EMPTY;
    }

    public static void onPlayerTick(PlayerTickEvent.Post e) {
        if (!(e.getEntity() instanceof ServerPlayer player)) return;
        ItemStack stack = held(player);
        HandheldSettings s = stack.isEmpty() ? null : HandheldSettings.read(stack);
        RadioMedium medium = RadioMediumHooks.medium();
        if (s == null || !s.on() || medium == null) {
            stop(player.getUUID());
            return;
        }
        sessions.computeIfAbsent(player.getUUID(), id -> new Session(id)).tick(player, s, medium);
    }

    public static void onLogout(PlayerEvent.PlayerLoggedOutEvent e) {
        stop(e.getEntity().getUUID());
    }

    static void stop(UUID player) {
        Session s = sessions.remove(player);
        RadioMedium m = RadioMediumHooks.medium();
        if (s != null && m != null) m.unregister(s.endpoint);
    }

    /** One player's radio: its endpoint on the medium, demodulator state and audio sequence. */
    public static final class Session {
        final UUID id;
        final Endpoint endpoint = new Endpoint();
        final HandheldDemod demod = new HandheldDemod();
        final ImaAdpcm adpcm = new ImaAdpcm();
        long cursor = Long.MIN_VALUE;
        int iqRate;
        double tunedHz;
        int seq;
        RadioMedium registered;

        public float lastSignalDbm = -200;

        public Session(UUID player) {
            this.id = new UUID(player.getMostSignificantBits() ^ 0x48616E64L, player.getLeastSignificantBits());
        }

        void tick(ServerPlayer player, HandheldSettings s, RadioMedium medium) {
            var eye = player.getEyePosition();
            float[] audio = receive(Pose.at(player.level().dimension().location().toString(), eye.x, eye.y - 0.3, eye.z), s, medium);
            if (audio == null) return;
            short[] pcm = HandheldDemod.toPcm(audio, s.volume());
            for (int off = 0; off < pcm.length; off += 4096) {
                int count = Math.min(4096, pcm.length - off);
                byte[] data = adpcm.encode(pcm, off, count);
                PacketDistributor.sendToPlayer(player, new HandheldPackets.Audio(seq++, AUDIO_RATE, count, data, lastSignalDbm, s.freqHz()));
            }
        }

        /**
         * Hear and demodulate everything since the last call at {@code pose}
         * (at most 0.25 s); returns audio at {@link #AUDIO_RATE}, or null if no time passed.
         */
        public float[] receive(Pose pose, HandheldSettings s, RadioMedium medium) {
            if (registered != medium) {
                medium.register(endpoint);
                registered = medium;
            }
            endpoint.pose = pose;
            endpoint.band = s.band();
            boolean wide = s.band().wide(s.freqHz());
            int rate = wide ? 240_000 : 48_000;
            endpoint.channel = new Channel(s.freqHz(), s.band().bandwidth(s.freqHz()));
            if (rate != iqRate || s.freqHz() != tunedHz) {
                iqRate = rate;
                tunedHz = s.freqHz();
                cursor = Long.MIN_VALUE;
            }
            long nowMicros = RadioMediumHooks.clockMicros();
            long now = IqSynthesizer.sampleAt(nowMicros, rate);
            if (cursor == Long.MIN_VALUE || now - cursor > rate / 4) cursor = now - rate / 20;
            int n = (int) Math.min(now - cursor, rate / 4);
            int decim = rate / AUDIO_RATE;
            n -= n % decim;
            if (n <= 0) return null;
            long from = (long) IqSynthesizer.microsAt(cursor, rate), to = (long) Math.ceil(IqSynthesizer.microsAt(cursor + n, rate));
            List<RadioMedium.Heard> heard = new ArrayList<>();
            medium.forEachHeard(endpoint, endpoint.channel, from - 2000, to, heard::add);
            // A fixed receiver gain (radios like this have AGC in the IF; the demodulators normalise).
            float[] iq = new float[2 * n];
            new IqSynthesizer(rate, s.freqHz(), 16, 7, id.getLeastSignificantBits() ^ cursor).synthesize(cursor, n, heard, 40, iq);
            cursor += n;
            double meter = demod.meterDbfs(iq, n);
            float signalDbm = (float) (meter + IqSynthesizer.FULL_SCALE_DBM_AT_0DB - 40);
            lastSignalDbm = signalDbm;
            float[] audio = s.band().mode == HandheldBand.Mode.FM
                    ? demod.fm(iq, n, decim, rate, wide ? 75_000 : 5_000)
                    : demod.am(iq, n, decim, 1.5);
            // Squelch: 0 = always open; 100 = needs a strong signal (−60 dBm).
            double threshold = -130 + 0.7 * s.squelch();
            if (s.squelch() > 0 && signalDbm < threshold) java.util.Arrays.fill(audio, 0);
            return audio;
        }

        /** Leave the medium. */
        public void close(RadioMedium medium) {
            if (medium != null) medium.unregister(endpoint);
        }
    }

    static final class Endpoint implements RadioEndpoint {
        final UUID id = UUID.randomUUID();
        volatile Pose pose = Pose.at("minecraft:overworld", 0, -10_000, 0);
        volatile Channel channel = new Channel(146.52e6, 12.5e3);
        volatile HandheldBand band = HandheldBand.VHF;

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() {
            double loss = -band.whipLossDb;
            return new AntennaPattern() {
                @Override public double gainDbi(double lx, double ly, double lz) { return AntennaPattern.VERTICAL_DIPOLE.gainDbi(lx, ly, lz); }
                @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {0, 1, 0}; }
                @Override public double peakGainDbi() { return 2.15; }
                @Override public double feedLossDb() { return loss; }
            };
        }
        @Override public Channel tunedChannel() { return channel; }
        @Override public double maxTxPowerDbm() { return -100; }
        @Override public double noiseFigureDb() { return 7; }
        @Override public boolean listening() { return false; }   // audio is synthesised, not frame-delivered
        @Override public void onReceive(Reception reception) {}
    }

    /** Active handheld count (tests, debug). */
    public static int activeSessions() {
        return sessions.size();
    }
}
//?}
