package com.example.evanscomputermod.module;

import com.example.evanscomputermod.radio.api.Pose;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;

/** World poses of block-hosted computers, for {@link com.example.evanscomputermod.api.module.IModuleHost#getPose()}. */
public final class BlockPoses {

    /**
     * The block's centre in world space. On a Sable sub-level: the projected
     * position and the sub-level's orientation (so module antennas turn with
     * the ship); elsewhere the block centre with identity orientation.
     */
    public static Pose of(Level level, BlockPos pos) {
        Vec3 c = Vec3.atCenterOf(pos);
        //? if >=26.1 {
        return Pose.at(level.dimension().identifier().toString(), c.x, c.y, c.z);
        //?} else {
        /*String dim = level.dimension().location().toString();
        var f = com.example.evanscomputermod.sensor.SensorSable.frame(level, c);
        if (!f.onStructure()) return Pose.at(dim, c.x, c.y, c.z);
        Vec3 w = f.position(c);
        float[] q = com.example.evanscomputermod.radio.antenna.Antenna.quaternion(
                f.normal(new Vec3(1, 0, 0)), f.normal(new Vec3(0, 1, 0)), f.normal(new Vec3(0, 0, 1)));
        return new Pose(dim, w.x, w.y, w.z, q[0], q[1], q[2], q[3]);*/
        //?}
    }

    private BlockPoses() {}
}
