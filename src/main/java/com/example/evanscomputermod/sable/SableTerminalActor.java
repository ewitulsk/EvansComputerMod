package com.example.evanscomputermod.sable;

//? if <=1.21.1 {
import dev.ryanhcode.sable.api.block.BlockEntitySubLevelActor;

/**
 * Placeholder {@link BlockEntitySubLevelActor} marker for Terminal block
 * entities. Currently we don't need per-physics-tick work — the interface
 * is used so sable knows to carry the BE across its move flow.
 */
public interface SableTerminalActor extends BlockEntitySubLevelActor {
    // Intentionally empty: inherits the default no-op implementations of
    // sable$tick, sable$physicsTick, sable$getLoadingDependencies, and
    // sable$getConnectionDependencies.
}
//?}
