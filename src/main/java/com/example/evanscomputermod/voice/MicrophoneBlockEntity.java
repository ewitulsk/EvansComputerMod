package com.example.evanscomputermod.voice;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Server side of a Microphone: owns its {@link MicAudio} (the capture card
 * programs read through {@code /dev/mic}), which hears the {@link SoundField}
 * at the capsule, and lights the tally lamp while something records. The gain
 * is saved with the block.
 */
public class MicrophoneBlockEntity extends BlockEntity implements MicrophonePeripheral.Owner {

    private final MicAudio mic;
    private final MicrophonePeripheral peripheral = new MicrophonePeripheral(this);
    private volatile String dimension = "";
    private boolean gainDirty;
    private boolean registered;

    public MicrophoneBlockEntity(BlockPos pos, BlockState state) {
        super(VoiceContent.MICROPHONE_BE.get(), pos, state);
        // The capsule sits at the top of the stand.
        double x = pos.getX() + 0.5, y = pos.getY() + 0.75, z = pos.getZ() + 0.5;
        this.mic = new MicAudio((start, n, rate, out) -> SoundField.get().render(dimension, x, y, z, start, n, rate, out));
    }

    @Override
    public MicAudio mic() {
        return mic;
    }

    public MicrophonePeripheral getPeripheral() {
        return peripheral;
    }

    @Override
    public void markGainChanged() {
        gainDirty = true;
    }

    void serverTick() {
        if (level == null) return;
        if (!registered) {
            dimension = level.dimension().location().toString();
            double x = worldPosition.getX() + 0.5, y = worldPosition.getY() + 0.75, z = worldPosition.getZ() + 0.5;
            VoiceServer.Spot spot = new VoiceServer.Spot(dimension, x, y, z);
            VoiceServer.addMic(mic, () -> spot);
            SoundField.get().setRange(EcmConfig.micRange());
            registered = true;
        }
        if (gainDirty) {
            gainDirty = false;
            setChanged();
        }
        boolean live = mic.listening();
        BlockState state = getBlockState();
        if (state.getBlock() instanceof MicrophoneBlock && state.getValue(MicrophoneBlock.LIVE) != live)
            level.setBlock(worldPosition, state.setValue(MicrophoneBlock.LIVE, live), Block.UPDATE_CLIENTS);
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        mic.close();
        VoiceServer.removeMic(mic);
    }

    @Override
    public void onChunkUnloaded() {
        super.onChunkUnloaded();
        mic.close();
        VoiceServer.removeMic(mic);
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putInt("gain", mic.gain());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        mic.setGain(tag.contains("gain") ? Math.max(0, Math.min(100, tag.getInt("gain"))) : MicAudio.DEFAULT_GAIN);
    }
}
//?}
