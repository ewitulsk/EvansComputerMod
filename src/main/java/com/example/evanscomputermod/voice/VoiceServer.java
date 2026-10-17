package com.example.evanscomputermod.voice;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.radio.handheld.HandheldServer;
import com.example.evanscomputermod.radio.handheld.HandheldSettings;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.common.util.FakePlayer;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.network.PacketDistributor;

import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Supplier;

/**
 * The server side of players' voices: puts each player's microphone into the
 * {@link SoundField} at their mouth, remembers who holds the Talk key (a
 * handheld radio's push-to-talk), and tells each client what wants its voice so
 * the client opens the microphone only then.
 */
public final class VoiceServer {

    /** Where a microphone is, for working out whose voice it wants. */
    public record Spot(String dimension, double x, double y, double z) {}

    private static final Map<MicAudio, Supplier<Spot>> MICS = new ConcurrentHashMap<>();
    private static final Map<UUID, Integer> SENT_DEMAND = new ConcurrentHashMap<>();
    private static final Set<UUID> TALKING = ConcurrentHashMap.newKeySet();
    private static final Map<UUID, Long> LAST_VOICE = new ConcurrentHashMap<>();
    private static final Map<UUID, Long> VOICE_SAMPLES = new ConcurrentHashMap<>();

    private VoiceServer() {}

    /** A microphone exists at a (possibly moving) place: players near it are asked for their voice while it records. */
    public static void addMic(MicAudio mic, Supplier<Spot> where) {
        MICS.put(mic, where);
    }

    public static void removeMic(MicAudio mic) {
        MICS.remove(mic);
    }

    /** Some microphone exists (sources skip putting sound in the air otherwise). */
    public static boolean anyMic() {
        return !MICS.isEmpty();
    }

    /** The air track id of a player's voice. */
    public static String trackId(UUID player) {
        return "player:" + player;
    }

    /** Where a player's mouth is. */
    public static Spot mouth(ServerPlayer p) {
        Vec3 eye = p.getEyePosition();
        return new Spot(p.level().dimension().location().toString(), eye.x, eye.y - 0.1, eye.z);
    }

    /** A block of the player's microphone arrived (the client's packet, or a scenario's stand-in for one). */
    public static void onVoice(ServerPlayer player, VoicePackets.Voice v) {
        setTalking(player.getUUID(), v.talk());
        if (!EcmConfig.micPlayerVoices()) return;
        if (v.samples() <= 0 || v.samples() > VoicePackets.MAX_SAMPLES || v.rate() < 8000 || v.rate() > 48000) return;
        short[] pcm = ImaAdpcm.decode(v.data(), v.samples());
        if (pcm == null) return;
        Spot m = mouth(player);
        SoundField.get().setRange(EcmConfig.micRange());
        SoundField.get().emit(trackId(player.getUUID()), m.dimension(), m.x(), m.y(), m.z(), pcm, 0, pcm.length, v.rate(), 1f);
        LAST_VOICE.put(player.getUUID(), System.nanoTime());
        VOICE_SAMPLES.merge(player.getUUID(), (long) pcm.length, Long::sum);
    }

    public static void onTalk(ServerPlayer player, boolean down) {
        setTalking(player.getUUID(), down);
    }

    private static void setTalking(UUID id, boolean down) {
        if (down) TALKING.add(id);
        else TALKING.remove(id);
    }

    /** The player holds their radio's push-to-talk key. */
    public static boolean talking(UUID player) {
        return TALKING.contains(player);
    }

    /** Samples of this player's voice received so far (tests, debug). */
    public static long voiceSamples(UUID player) {
        return VOICE_SAMPLES.getOrDefault(player, 0L);
    }

    /** What wants this player's voice now ({@code DEMAND_*} bits). */
    public static int demandFor(ServerPlayer p) {
        int flags = 0;
        Spot m = mouth(p);
        double range = EcmConfig.micRange();
        for (Map.Entry<MicAudio, Supplier<Spot>> e : MICS.entrySet()) {
            if (!e.getKey().listening()) continue;
            Spot s = e.getValue().get();
            if (s == null || !s.dimension().equals(m.dimension())) continue;
            double dx = s.x() - m.x(), dy = s.y() - m.y(), dz = s.z() - m.z();
            if (dx * dx + dy * dy + dz * dz <= range * range) {
                flags |= VoicePackets.DEMAND_MIC;
                break;
            }
        }
        var stack = HandheldServer.held(p);
        if (!stack.isEmpty()) {
            HandheldSettings s = HandheldSettings.read(stack);
            if (s.on() && HandheldServer.canTransmit(stack, s)) {
                flags |= VoicePackets.DEMAND_RADIO;
                if (talking(p.getUUID())) flags |= VoicePackets.DEMAND_ON_AIR;
            }
        }
        return flags;
    }

    public static void onServerTick(ServerTickEvent.Post e) {
        MinecraftServer server = e.getServer();
        // Keep each talker's voice where they are now.
        for (ServerPlayer p : server.getPlayerList().getPlayers()) {
            AirTrack t = SoundField.get().find(trackId(p.getUUID()));
            if (t != null) {
                Spot m = mouth(p);
                t.moveTo(m.dimension(), m.x(), m.y(), m.z());
            }
            if (p instanceof FakePlayer) continue;
            int flags = demandFor(p);
            Integer was = SENT_DEMAND.put(p.getUUID(), flags);
            if (was == null || was != flags) PacketDistributor.sendToPlayer(p, new VoicePackets.Demand(flags));
        }
    }

    public static void onLogout(PlayerEvent.PlayerLoggedOutEvent e) {
        UUID id = e.getEntity().getUUID();
        SENT_DEMAND.remove(id);
        TALKING.remove(id);
        LAST_VOICE.remove(id);
        SoundField.get().remove(trackId(id));
    }
}
//?}
