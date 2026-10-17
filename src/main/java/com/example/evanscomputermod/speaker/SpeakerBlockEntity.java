package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.EcmConfig;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.network.PacketDistributor;
//? if >=26.1 {
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
//?}

import java.util.List;

/**
 * Server side of a Speaker: owns its {@link SpeakerAudio} stream, sends what
 * programs wrote to nearby players (IMA ADPCM), and shows the loudness on the
 * cone. The volume is saved with the block.
 *
 * <p>Sending runs on the display pump every {@link #CADENCE_MS} ms, not on the
 * server tick (DISPLAY_PIPELINE_SPEC.md Part A4), so a tick hitch doesn't
 * stall the sound; each block carries the time its first sample plays
 * (Part E2). Everything here happens downstream of {@link SpeakerAudio#drain()},
 * the single output of the speaker's card. The listeners come from the last
 * tick (a snapshot); the microphone sound field and the cone stay on the tick.
 */
public class SpeakerBlockEntity extends BlockEntity implements SpeakerPeripheral.Owner {

    /** Samples per network block (a little over one tick at 48 kHz). */
    private static final int CHUNK = 2600;

    private final SpeakerAudio audio = new SpeakerAudio();
    private final SpeakerPeripheral peripheral = new SpeakerPeripheral(this);
    private final ImaAdpcm encoder = new ImaAdpcm();
    private int seq;
    private int quietTicks;
    private int levelCooldown;
    private volatile boolean volumeDirty;

    /** How often the pump sends what was written (ms). */
    static final int CADENCE_MS = 20;
    /** Players in range at the last tick (the pump sends to these). */
    private volatile List<ServerPlayer> listeners = List.of();
    /** Audio the pump drained, for the tick to put into the air for microphones. */
    private final java.util.concurrent.ConcurrentLinkedQueue<short[]> forField = new java.util.concurrent.ConcurrentLinkedQueue<>();
    /** Loudest sample since the tick last looked, and whether anything played. */
    private volatile int pumpPeak;
    private volatile boolean pumpPlayed;
    private java.util.concurrent.ScheduledFuture<?> pumpTask;
    /** Test listeners (the in-process client harnesses): they get every block a nearby player would. */
    private final java.util.Map<java.util.UUID, java.util.function.Consumer<SpeakerAudioPacket>> testListeners =
            new java.util.concurrent.ConcurrentHashMap<>();

    public void addTestListener(java.util.UUID id, java.util.function.Consumer<SpeakerAudioPacket> sink) {
        testListeners.put(id, sink);
    }

    public void removeTestListener(java.util.UUID id) {
        testListeners.remove(id);
    }

    public SpeakerBlockEntity(BlockPos pos, BlockState state) {
        super(SpeakerContent.SPEAKER_BE.get(), pos, state);
    }

    @Override
    public SpeakerAudio audio() {
        return audio;
    }

    public SpeakerPeripheral getPeripheral() {
        return peripheral;
    }

    @Override
    public void markVolumeChanged() {
        volumeDirty = true;
    }

    /** Play a Minecraft sound at the speaker (server thread). */
    @Override
    public boolean playSound(String id, float volume, float pitch) {
        Identifier rl = Identifier.tryParse(id);
        if (rl == null || !(level instanceof ServerLevel sl)) return false;
        var event = net.minecraft.core.registries.BuiltInRegistries.SOUND_EVENT.getOptional(rl);
        if (event.isEmpty()) return false;
        sl.playSound((net.minecraft.world.entity.Entity) null, worldPosition, event.get(), SoundSource.RECORDS,
                volume * audio.volume() / 100f, pitch);
        return true;
    }

    void serverTick() {
        if (!(level instanceof ServerLevel sl)) return;
        if (volumeDirty) {
            volumeDirty = false;
            setChanged();
        }
        listeners = listeners(sl);
        if (pumpTask == null && !isRemoved()) {
            pumpTask = com.example.evanscomputermod.computer.pipeline.DisplayPump.scheduler().scheduleAtFixedRate(
                    this::pumpAudio, CADENCE_MS, CADENCE_MS, java.util.concurrent.TimeUnit.MILLISECONDS);
        }
        short[] pcm;
        //? if <=1.21.1 {
        // Into the air too, for microphones nearby (on the tick, as before).
        boolean mics = com.example.evanscomputermod.voice.VoiceServer.anyMic();
        while ((pcm = forField.poll()) != null) {
            if (!mics) continue;
            Vec3 c = Vec3.atCenterOf(worldPosition);
            com.example.evanscomputermod.voice.SoundField.get().emit("speaker:" + sl.dimension().location() + ":" + worldPosition.asLong(),
                    sl.dimension().location().toString(), c.x, c.y, c.z, pcm, 0, pcm.length, audio.rate(), 1f);
        }
        //?} else {
        /*forField.clear();*/
        //?}
        boolean played = pumpPlayed;
        int peak = pumpPeak;
        pumpPlayed = false;
        pumpPeak = 0;
        if (played) quietTicks = 0;
        else quietTicks++;
        updateCone(played ? levelOf(peak) : (quietTicks > 4 ? 0 : -1));
    }

    /**
     * Display pump, every {@link #CADENCE_MS} ms: send what the card mixed
     * since the last run. The block's first sample plays when the queue ahead
     * of it has played: {@code now + buffered - this block}.
     */
    private void pumpAudio() {
        if (isRemoved()) return;
        short[] pcm = audio.drain();
        if (pcm.length == 0) return;
        int rate = Math.max(1, audio.rate());
        long now = com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs();
        // Live audio (a call) plays at once on the client; everything else is stamped with when it plays.
        boolean live = audio.lastDrainLive();
        long pts = live ? now : now + audio.bufferedMs() * 1000L - pcm.length * 1_000_000L / rate;
        if (pts < now) pts = now;
        int flags = live ? SpeakerAudioPacket.FLAG_LIVE : 0;
        java.util.UUID computer = peripheral.computer();
        int peak = 0;
        List<ServerPlayer> to = listeners;
        for (int off = 0; off < pcm.length; off += CHUNK) {
            int n = Math.min(CHUNK, pcm.length - off);
            for (int i = off; i < off + n; i++) peak = Math.max(peak, Math.abs(pcm[i]));
            if (to.isEmpty() && testListeners.isEmpty()) continue;
            byte[] block = encoder.encode(pcm, off, n);
            SpeakerAudioPacket packet = new SpeakerAudioPacket(worldPosition, seq++, rate, n, block,
                    live ? now : pts + off * 1_000_000L / rate, computer, flags);
            for (ServerPlayer p : to) {
                if (!p.hasDisconnected()) PacketDistributor.sendToPlayer(p, packet);
            }
            for (var l : testListeners.values()) l.accept(packet);
        }
        pumpPeak = Math.max(pumpPeak, peak);
        pumpPlayed = true;
        if (forField.size() < 50) forField.add(pcm);
    }

    private void stopPump() {
        var t = pumpTask;
        pumpTask = null;
        if (t != null) t.cancel(false);
    }

    private static int levelOf(int peak) {
        if (peak < 600) return 0;
        if (peak < 6000) return 1;
        if (peak < 18000) return 2;
        return 3;
    }

    /** Move the cone (at most every 3 ticks, so a block update isn't sent every tick). */
    private void updateCone(int target) {
        if (levelCooldown > 0) levelCooldown--;
        if (target < 0 || levelCooldown > 0 || level == null) return;
        BlockState state = getBlockState();
        if (!(state.getBlock() instanceof SpeakerBlock) || state.getValue(SpeakerBlock.LEVEL) == target) return;
        level.setBlock(worldPosition, state.setValue(SpeakerBlock.LEVEL, target), Block.UPDATE_CLIENTS);
        levelCooldown = 3;
    }

    /** Players close enough to hear it (on a Sable structure: the players tracking it). */
    private List<ServerPlayer> listeners(ServerLevel sl) {
        //? if <=1.21.1 {
        List<ServerPlayer> plot = com.example.evanscomputermod.sable.SableCompat.getPlotTrackingPlayers(sl, worldPosition);
        if (plot != null) return plot;
        //?}
        double range = EcmConfig.speakerRange();
        Vec3 c = Vec3.atCenterOf(worldPosition);
        return sl.getPlayers(p -> p.distanceToSqr(c) < range * range);
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        stopPump();
        audio.close();
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
        stopPump();
        audio.close();
    }

    // --- NBT: the volume ---

    //? if >=26.1 {
    @Override
    protected void saveAdditional(ValueOutput output) {
        super.saveAdditional(output);
        output.putInt("volume", audio.volume());
    }

    @Override
    protected void loadAdditional(ValueInput input) {
        super.loadAdditional(input);
        audio.setVolume(Math.max(0, Math.min(100, input.getIntOr("volume", 100))));
    }
    //?} else {
    /*@Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putInt("volume", audio.volume());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        audio.setVolume(tag.contains("volume") ? Math.max(0, Math.min(100, tag.getInt("volume"))) : 100);
    }*/
    //?}
}
