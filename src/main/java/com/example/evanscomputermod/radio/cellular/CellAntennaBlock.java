package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;

import java.util.Map;

/**
 * A cell tower antenna: the LTE sector panel or the mmWave array. Mounted on a
 * mast above a Cell Tower base station (on the side of a mast block, or on top
 * of the mast or the base station), it points out along its {@link #FACING}.
 * The base station finds its antennas itself; their height, how many face which
 * way and the feeder down the mast set the tower's coverage
 * ({@link CellAntennas}).
 */
public class CellAntennaBlock extends HorizontalDirectionalBlock {
    private final CellTech tech;
    private final MapCodec<CellAntennaBlock> codec;
    /** North-facing panel, its back on the south edge (where the mast is). */
    private static final Map<Direction, VoxelShape> PANEL = com.example.evanscomputermod.radio.FacingShapes.of(
            com.example.evanscomputermod.radio.FacingShapes.box(3, 0, 10, 13, 16, 14),
            com.example.evanscomputermod.radio.FacingShapes.box(6, 4, 14, 10, 12, 16));
    private static final Map<Direction, VoxelShape> ARRAY = com.example.evanscomputermod.radio.FacingShapes.of(
            com.example.evanscomputermod.radio.FacingShapes.box(2, 2, 12, 14, 14, 15),
            com.example.evanscomputermod.radio.FacingShapes.box(6, 6, 15, 10, 10, 16));

    public CellAntennaBlock(CellTech tech, Properties properties) {
        super(properties);
        this.tech = tech;
        this.codec = simpleCodec(p -> new CellAntennaBlock(tech, p));
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH));
    }

    public CellTech tech() {
        return tech;
    }

    @Override
    protected MapCodec<? extends HorizontalDirectionalBlock> codec() {
        return codec;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING);
    }

    /** Clicked on a mast's side: point away from it. On top of something: point at the player. */
    @Override
    public BlockState getStateForPlacement(BlockPlaceContext ctx) {
        Direction face = ctx.getClickedFace();
        Direction f = face.getAxis().isHorizontal() ? face : ctx.getHorizontalDirection().getOpposite();
        return defaultBlockState().setValue(FACING, f);
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext ctx) {
        return (tech == CellTech.LTE ? PANEL : ARRAY).get(state.getValue(FACING));
    }
}
//?}
