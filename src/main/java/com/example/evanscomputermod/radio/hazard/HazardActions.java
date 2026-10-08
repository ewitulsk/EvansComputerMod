package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.api.event.AntennaOverloadEvent;
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import com.mojang.authlib.GameProfile;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.block.BaseFireBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.common.util.BlockSnapshot;
import net.neoforged.neoforge.common.util.FakePlayer;
import net.neoforged.neoforge.common.util.FakePlayerFactory;
import net.neoforged.neoforge.event.level.BlockEvent;
import org.jetbrains.annotations.Nullable;

import java.util.UUID;

/**
 * How hazards touch the world. Every edit is announced first: a cancellable
 * {@link HazardEvent}, then a vanilla-style {@code BlockEvent} (break or
 * place) from a {@link FakePlayer} owned by the antenna's builder, so claim
 * mods can veto it. Nothing happens at hazard level {@code off}.
 */
public final class HazardActions {
    /** Owner used when nobody is known to have built the part. */
    public static final UUID ANONYMOUS = UUID.fromString("6c2f2f3e-7a51-4d7b-9e2a-ec0dd10a0001");

    private HazardActions() {}

    public static FakePlayer fakePlayer(ServerLevel level, @Nullable UUID owner) {
        UUID id = owner == null ? ANONYMOUS : owner;
        return FakePlayerFactory.get(level, new GameProfile(id, "[ECM Radio]"));
    }

    public static boolean equipmentHazards(ServerLevel level) {
        return RadioGameRules.level(level) != RadioConfig.HazardLevel.OFF;
    }

    public static boolean fullHazards(ServerLevel level) {
        return RadioGameRules.level(level) == RadioConfig.HazardLevel.FULL;
    }

    /** Posts an AntennaOverloadEvent; true if nobody cancelled it. */
    public static boolean overload(ServerLevel level, BlockPos feed, double powerW, double ratedW, BlockPos at, String cause) {
        return !NeoForge.EVENT_BUS.post(new AntennaOverloadEvent(level, feed, powerW, ratedW, at, cause)).isCanceled();
    }

    /** Posts a HazardEvent; true if nobody cancelled it. */
    public static boolean announce(ServerLevel level, BlockPos pos, HazardEvent.Kind kind, @Nullable net.minecraft.world.entity.Entity victim, String detail) {
        return !NeoForge.EVENT_BUS.post(new HazardEvent(level, pos, kind, victim, detail)).isCanceled();
    }

    /**
     * Destroys a radio part (melt, burnout, lightning): HazardEvent, then a
     * BreakEvent from the owner's fake player, then the block goes (no normal
     * drops) leaving {@code scrap}. Returns true if it went.
     */
    public static boolean destroy(ServerLevel level, BlockPos pos, @Nullable UUID owner, HazardEvent.Kind kind, String detail,
                                  ItemStack scrap) {
        if (!equipmentHazards(level)) return false;
        BlockState state = level.getBlockState(pos);
        if (state.isAir()) return false;
        if (!announce(level, pos, kind, null, detail)) return false;
        if (NeoForge.EVENT_BUS.post(new BlockEvent.BreakEvent(level, pos, state, fakePlayer(level, owner))).isCanceled()) return false;
        level.removeBlock(pos, false);
        burst(level, pos, ParticleTypes.LARGE_SMOKE, 12);
        level.playSound(null, pos, SoundEvents.LAVA_EXTINGUISH, SoundSource.BLOCKS, 0.8f, 1.0f);
        if (!scrap.isEmpty()) {
            ItemEntity item = new ItemEntity(level, pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5, scrap);
            item.setDefaultPickUpDelay();
            level.addFreshEntity(item);
        }
        EvansComputerMod.LOGGER.info("[radio hazard] {} at {}: {}", kind, pos, detail);
        return true;
    }

    /**
     * An arc at {@code at}: sparks and a crack. At hazard level full with
     * {@code doFireTick} on, the first flammable block within {@code radius}
     * catches fire (HazardEvent ARC_FIRE, then a place event for the fire from
     * the owner's fake player). Returns true if a fire started.
     */
    public static boolean arc(ServerLevel level, BlockPos at, int radius, @Nullable UUID owner, String detail) {
        burst(level, at, ParticleTypes.ELECTRIC_SPARK, 30);
        level.playSound(null, at, SoundEvents.LIGHTNING_BOLT_IMPACT, SoundSource.BLOCKS, 0.5f, 1.8f);
        if (!fullHazards(level) || !level.getGameRules().getBoolean(GameRules.RULE_DOFIRETICK)) return false;
        for (BlockPos p : BlockPos.withinManhattan(at, radius, radius, radius)) {
            BlockState s = level.getBlockState(p);
            if (s.isAir()) continue;
            for (Direction d : Direction.values()) {
                if (!s.isFlammable(level, p, d)) continue;
                BlockPos fire = p.relative(d);
                if (!level.getBlockState(fire).isAir()) continue;
                BlockState fireState = BaseFireBlock.getState(level, fire);
                if (!fireState.canSurvive(level, fire)) continue;
                if (!announce(level, fire, HazardEvent.Kind.ARC_FIRE, null, detail)) return false;
                BlockSnapshot snap = BlockSnapshot.create(level.dimension(), level, fire);
                level.setBlock(fire, fireState, Block.UPDATE_ALL);
                if (NeoForge.EVENT_BUS.post(new BlockEvent.EntityPlaceEvent(snap, s, fakePlayer(level, owner))).isCanceled()) {
                    snap.restore(Block.UPDATE_ALL);
                    return false;
                }
                EvansComputerMod.LOGGER.info("[radio hazard] arc fire at {}: {}", fire, detail);
                return true;
            }
        }
        return false;
    }

    public static void burst(ServerLevel level, BlockPos pos, ParticleOptions particle, int count) {
        level.sendParticles(particle, pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5, count, 0.3, 0.3, 0.3, 0.02);
    }

    public static void sound(ServerLevel level, BlockPos pos, SoundEvent sound, float volume, float pitch) {
        level.playSound(null, pos, sound, SoundSource.BLOCKS, volume, pitch);
    }
}
//?}
