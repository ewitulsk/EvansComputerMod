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
 * programs wrote to nearby players every tick (IMA ADPCM), and shows the
 * loudness on the cone. The volume is saved with the block.
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
    private boolean volumeDirty;

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
        short[] pcm = audio.drain();
        int peak = 0;
        if (pcm.length > 0) {
            List<ServerPlayer> listeners = listeners(sl);
            for (int off = 0; off < pcm.length; off += CHUNK) {
                int n = Math.min(CHUNK, pcm.length - off);
                for (int i = off; i < off + n; i++) peak = Math.max(peak, Math.abs(pcm[i]));
                if (listeners.isEmpty()) continue;
                byte[] block = encoder.encode(pcm, off, n);
                SpeakerAudioPacket packet = new SpeakerAudioPacket(worldPosition, seq++, audio.rate(), n, block);
                for (ServerPlayer p : listeners) PacketDistributor.sendToPlayer(p, packet);
            }
            quietTicks = 0;
        } else {
            quietTicks++;
        }
        updateCone(pcm.length > 0 ? levelOf(peak) : (quietTicks > 4 ? 0 : -1));
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
        audio.close();
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
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
