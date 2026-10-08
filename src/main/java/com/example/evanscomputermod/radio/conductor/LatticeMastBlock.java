package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * The lattice mast: a see-through 15 px tower section that is itself the
 * radiator of a broadcast antenna. Only its four corner legs collide, so a
 * player can step inside and climb it (it is in {@code #minecraft:climbable}).
 */
public class LatticeMastBlock extends ConductorBlock {
    private static final VoxelShape LEGS = Shapes.or(
            Block.box(0.5, 0, 0.5, 2.5, 16, 2.5), Block.box(13.5, 0, 0.5, 15.5, 16, 2.5),
            Block.box(0.5, 0, 13.5, 2.5, 16, 15.5), Block.box(13.5, 0, 13.5, 15.5, 16, 15.5));

    public LatticeMastBlock(Properties properties) {
        super(properties, Role.CONDUCTOR, 15, true);
    }

    @Override
    protected VoxelShape getCollisionShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        return LEGS;
    }
}
//?}
