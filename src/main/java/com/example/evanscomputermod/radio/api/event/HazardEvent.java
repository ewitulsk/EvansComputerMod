package com.example.evanscomputermod.radio.api.event;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.neoforged.bus.api.Event;
import net.neoforged.bus.api.ICancellableEvent;
import org.jetbrains.annotations.Nullable;

/**
 * Fired before a radio hazard changes the world or hurts someone: a part
 * melting, an arc starting a fire, RF exposure damage, a lightning strike
 * destroying a radio down the coax. Cancel it to prevent the consequence.
 * World edits additionally go through BlockEvents with a fake player owned by
 * the antenna's placer, so claim mods can veto them.
 */
public class HazardEvent extends Event implements ICancellableEvent {
    public enum Kind { MELT, ARC_FIRE, RF_EXPOSURE, LIGHTNING, AMPLIFIER_BURNOUT }

    private final Level level;
    private final BlockPos pos;
    private final Kind kind;
    @Nullable private final Entity victim;
    private final String detail;

    public HazardEvent(Level level, BlockPos pos, Kind kind, @Nullable Entity victim, String detail) {
        this.level = level;
        this.pos = pos;
        this.kind = kind;
        this.victim = victim;
        this.detail = detail;
    }

    public Level level() { return level; }
    public BlockPos pos() { return pos; }
    public Kind kind() { return kind; }
    @Nullable public Entity victim() { return victim; }
    public String detail() { return detail; }
}
//?}
