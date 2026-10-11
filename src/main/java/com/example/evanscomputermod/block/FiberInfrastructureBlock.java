package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;

/** Generated infrastructure changes the saved edge only on an actual block mutation. */
public final class FiberInfrastructureBlock extends Block {
  public static final net.minecraft.world.level.block.state.properties.BooleanProperty TOP =
      net.minecraft.world.level.block.state.properties.BooleanProperty.create("top");

  public FiberInfrastructureBlock(Properties properties) {
    super(properties);
    var state = defaultBlockState().setValue(TOP, true);
    for (var d : net.minecraft.core.Direction.values())
      state = state.setValue(NetworkCableBlock.getPropertyForDirection(d), false);
    registerDefaultState(state);
  }

  @Override
  protected void createBlockStateDefinition(
      net.minecraft.world.level.block.state.StateDefinition.Builder<Block, BlockState> builder) {
    builder.add(
        TOP,
        NetworkCableBlock.NORTH,
        NetworkCableBlock.SOUTH,
        NetworkCableBlock.EAST,
        NetworkCableBlock.WEST,
        NetworkCableBlock.UP,
        NetworkCableBlock.DOWN);
  }

  public static boolean connects(BlockState state) {
    return state.getBlock() instanceof FiberInfrastructureBlock
        || state.is(ModBlocks.FIBER_PATCH_PANEL.get());
  }

  private BlockState connections(
      BlockState state, net.minecraft.world.level.BlockGetter level, BlockPos pos) {
    for (var d : net.minecraft.core.Direction.values())
      state =
          state.setValue(
              NetworkCableBlock.getPropertyForDirection(d),
              connects(level.getBlockState(pos.relative(d))));
    return state.setValue(TOP, !level.getBlockState(pos.above()).is(ModBlocks.UTILITY_POLE.get()));
  }

  @Override
  public BlockState getStateForPlacement(
      net.minecraft.world.item.context.BlockPlaceContext context) {
    return connections(defaultBlockState(), context.getLevel(), context.getClickedPos());
  }

  //? if >=26.1 {
  @Override
  protected BlockState updateShape(
      BlockState state,
      net.minecraft.world.level.LevelReader level,
      net.minecraft.world.level.ScheduledTickAccess ticks,
      BlockPos pos,
      net.minecraft.core.Direction direction,
      BlockPos neighborPos,
      BlockState neighborState,
      net.minecraft.util.RandomSource random) {
    return connections(state, level, pos);
  }

  //?} else {
  /*@Override
  protected BlockState updateShape(BlockState state, net.minecraft.core.Direction direction, BlockState neighborState,
          net.minecraft.world.level.LevelAccessor level, BlockPos pos, BlockPos neighborPos) {
      return connections(state, level, pos);
  }*/
  //?}

  @Override
  protected net.minecraft.world.phys.shapes.VoxelShape getShape(
      BlockState state,
      net.minecraft.world.level.BlockGetter level,
      BlockPos pos,
      net.minecraft.world.phys.shapes.CollisionContext context) {
    if (state.is(ModBlocks.UTILITY_POLE.get())) {
      var shaft = Block.box(5, 0, 5, 11, 16, 11);
      return state.getValue(TOP)
          ? net.minecraft.world.phys.shapes.Shapes.or(shaft, Block.box(1, 12, 6, 15, 16, 10))
          : shaft;
    }
    var shape = Block.box(5.3, 5.3, 5.3, 10.7, 10.7, 10.7);
    for (var d : net.minecraft.core.Direction.values())
      if (state.getValue(NetworkCableBlock.getPropertyForDirection(d))) {
        double x0 = 6.5, y0 = 6.5, z0 = 6.5, x1 = 9.5, y1 = 9.5, z1 = 9.5;
        switch (d) {
          case NORTH -> z0 = 0;
          case SOUTH -> z1 = 16;
          case WEST -> x0 = 0;
          case EAST -> x1 = 16;
          case DOWN -> y0 = 0;
          case UP -> y1 = 16;
        }
        shape = net.minecraft.world.phys.shapes.Shapes.or(shape, Block.box(x0, y0, z0, x1, y1, z1));
      }
    return shape;
  }

  @Override
  protected void onPlace(
      BlockState state, Level level, BlockPos pos, BlockState old, boolean moved) {
    super.onPlace(state, level, pos, old, moved);
    if (!old.is(state.getBlock())) {
      var connected = connections(state, level, pos);
      if (connected != state) level.setBlock(pos, connected, 2);
    }
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
