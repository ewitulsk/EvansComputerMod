package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;

/** Generated infrastructure changes the saved edge only on an actual block mutation. */
public final class FiberInfrastructureBlock extends Block {
    public FiberInfrastructureBlock(Properties properties) {
        super(properties);
    }

    @Override
    protected void onPlace(
            BlockState state, Level level, BlockPos pos, BlockState old, boolean moved) {
        super.onPlace(state, level, pos, old, moved);
        //? if <=1.21.1 {
        if (level instanceof net.minecraft.server.level.ServerLevel server) {
            var d = com.example.evanscomputermod.computer.WorldNetwork.get(server);
            if (d.brokenFiber.remove(pos.asLong())) {
                d.setDirty();
                d.applyLinks(server);
            }
        }
        //?}
    }

    //? if <=1.21.1 {
    @Override
    protected void onRemove(
            BlockState state, Level level, BlockPos pos, BlockState next, boolean moved) {
        if (!state.is(next.getBlock())
                && level instanceof net.minecraft.server.level.ServerLevel server) {
            var d = com.example.evanscomputermod.computer.WorldNetwork.get(server);
            if (d.generatedFiber.contains(pos.asLong())) {
                d.brokenFiber.add(pos.asLong());
                d.setDirty();
                d.applyLinks(server);
            }
        }
        super.onRemove(state, level, pos, next, moved);
    }
    //?}
}
