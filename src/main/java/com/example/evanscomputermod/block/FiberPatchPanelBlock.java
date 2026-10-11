package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.EnumProperty;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;

/** A physical rack enclosure, rather than an invisible cube with a cable outline. */
public final class FiberPatchPanelBlock extends NetworkCableBlock {
  public static final EnumProperty<Direction> FACING =
      net.minecraft.world.level.block.state.properties.BlockStateProperties.HORIZONTAL_FACING;

  public FiberPatchPanelBlock(Properties properties) {
    super(properties);
    registerDefaultState(defaultBlockState().setValue(FACING, Direction.NORTH));
  }

  @Override
  protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
    super.createBlockStateDefinition(builder);
    builder.add(FACING);
  }

  @Override
  public BlockState getStateForPlacement(
      net.minecraft.world.item.context.BlockPlaceContext context) {
    return super.getStateForPlacement(context)
        .setValue(FACING, context.getHorizontalDirection().getOpposite());
  }

  @Override
  protected VoxelShape getShape(
      BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
    return state.getValue(FACING).getAxis() == Direction.Axis.Z
        ? Block.box(0, 2, 2, 16, 14.2, 14)
        : Block.box(2, 2, 0, 14, 14.2, 16);
  }
}
