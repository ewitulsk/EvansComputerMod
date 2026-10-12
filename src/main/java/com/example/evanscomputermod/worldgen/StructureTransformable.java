package com.example.evanscomputermod.worldgen;

import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.Rotation;

/**
 * A block entity whose saved data holds world directions (a dish's aim, a conductor's cut
 * faces). Vanilla structure placement turns the block state ({@code mirror} then
 * {@code rotate}) but loads the block entity's NBT unchanged; the structure-template mixin
 * calls this right after that load so the block entity turns with its block.
 */
public interface StructureTransformable {
    /** Apply {@code mirror} and then {@code rotation}, as structure placement does to the block state. */
    void transformForStructure(Mirror mirror, Rotation rotation);
}
