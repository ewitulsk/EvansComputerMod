package com.example.evanscomputermod.radio.api.event;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.neoforged.bus.api.Event;
import net.neoforged.bus.api.ICancellableEvent;

/**
 * Fired when power into an antenna exceeds its rating (wire current, insulator
 * voltage, coax heat) and a failure is about to be applied. Cancel it to keep
 * the hardware intact (the warning stage still shows).
 */
public class AntennaOverloadEvent extends Event implements ICancellableEvent {
    private final Level level;
    private final BlockPos feedPoint;
    private final double powerWatts;
    private final double ratedWatts;
    private final BlockPos weakestLink;
    private final String cause;

    public AntennaOverloadEvent(Level level, BlockPos feedPoint, double powerWatts, double ratedWatts, BlockPos weakestLink, String cause) {
        this.level = level;
        this.feedPoint = feedPoint;
        this.powerWatts = powerWatts;
        this.ratedWatts = ratedWatts;
        this.weakestLink = weakestLink;
        this.cause = cause;
    }

    public Level level() { return level; }
    public BlockPos feedPoint() { return feedPoint; }
    public double powerWatts() { return powerWatts; }
    public double ratedWatts() { return ratedWatts; }
    public BlockPos weakestLink() { return weakestLink; }
    /** "wire_current", "insulator_voltage", "coax_heat", "swr", "tuner_mismatch" */
    public String cause() { return cause; }
}
//?}
