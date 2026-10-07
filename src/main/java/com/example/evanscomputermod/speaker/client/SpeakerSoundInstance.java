package com.example.evanscomputermod.speaker.client;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.client.Minecraft;
import net.minecraft.client.resources.sounds.AbstractSoundInstance;
import net.minecraft.client.resources.sounds.Sound;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.resources.sounds.TickableSoundInstance;
import net.minecraft.client.sounds.AudioStream;
import net.minecraft.client.sounds.SoundBufferLibrary;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.client.sounds.WeighedSoundEvents;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.valueproviders.ConstantFloat;
import net.minecraft.world.phys.Vec3;

import java.util.concurrent.CompletableFuture;

/**
 * A speaker playing: a positional, streaming sound whose samples come from a
 * {@link PcmStream}. It resolves to its own streaming {@link Sound}, so no
 * sound file or sounds.json entry is involved. Volume slider: Jukebox/Note Blocks.
 */
final class SpeakerSoundInstance extends AbstractSoundInstance implements TickableSoundInstance {

    private static final Identifier ID = Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "speaker");
    /** Distance over which the sound fades out (blocks). */
    static final int RANGE = 48;

    private final BlockPos pos;
    private final PcmStream stream;
    private boolean stopped;

    SpeakerSoundInstance(BlockPos pos, PcmStream stream) {
        super(ID, SoundSource.RECORDS, SoundInstance.createUnseededRandom());
        this.pos = pos;
        this.stream = stream;
        this.volume = 1.0f;
        this.pitch = 1.0f;
        this.looping = false;
        this.relative = false;
        this.attenuation = Attenuation.LINEAR;
        place();
    }

    @Override
    public WeighedSoundEvents resolve(SoundManager manager) {
        this.sound = new Sound(ID, ConstantFloat.of(1.0f), ConstantFloat.of(1.0f), 1,
                Sound.Type.FILE, true, false, RANGE);
        return new WeighedSoundEvents(ID, null);
    }

    @Override
    public CompletableFuture<AudioStream> getStream(SoundBufferLibrary buffers, Sound sound, boolean looping) {
        return CompletableFuture.completedFuture(stream);
    }

    @Override
    public boolean canStartSilent() {
        return true;
    }

    @Override
    public boolean isStopped() {
        return stopped;
    }

    void stopPlaying() {
        stopped = true;
    }

    @Override
    public void tick() {
        place();
    }

    /** Follow the speaker (it moves on a Sable structure). */
    private void place() {
        Vec3 c = Vec3.atCenterOf(pos);
        //? if <=1.21.1 {
        var level = Minecraft.getInstance().level;
        if (level != null) c = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, c);
        //?}
        this.x = c.x;
        this.y = c.y;
        this.z = c.z;
    }
}
