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
  protected BlockState rotate(BlockState state, net.minecraft.world.level.block.Rotation rotation) {
    return super.rotate(state, rotation).setValue(FACING, rotation.rotate(state.getValue(FACING)));
  }

  @Override
  protected BlockState mirror(BlockState state, net.minecraft.world.level.block.Mirror mirror) {
    return super.mirror(state, mirror).setValue(FACING, mirror.mirror(state.getValue(FACING)));
  }

  /**
   * A panel placed against a span of the generated ring taps that piece of the chord
   * (WorldNetwork keeps an index of such panels for unloaded chunks).
   */
  @Override
  protected void onPlace(
      BlockState state,
      net.minecraft.world.level.Level level,
      BlockPos pos,
      BlockState oldState,
      boolean movedByPiston) {
    super.onPlace(state, level, pos, oldState, movedByPiston);
    //? if <=1.21.1 {
    if (!oldState.is(this) && level instanceof net.minecraft.server.level.ServerLevel server)
      com.example.evanscomputermod.computer.WorldNetwork.get(server).attachmentChanged(server, pos, true);
    //?}
  }

  //? if <=1.21.1 {
  @Override
  protected void onRemove(
      BlockState state,
      net.minecraft.world.level.Level level,
      BlockPos pos,
      BlockState newState,
      boolean movedByPiston) {
    if (!state.is(newState.getBlock()) && level instanceof net.minecraft.server.level.ServerLevel server)
      com.example.evanscomputermod.computer.WorldNetwork.get(server).attachmentChanged(server, pos, false);
    super.onRemove(state, level, pos, newState, movedByPiston);
  }
  //?}

  @Override
  protected VoxelShape getShape(
      BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
    return net.minecraft.world.phys.shapes.Shapes.block();
  }
}
