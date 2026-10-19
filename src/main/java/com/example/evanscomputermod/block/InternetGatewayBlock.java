package com.example.evanscomputermod.block;

import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BooleanProperty;

/**
 * The physical connection point to the host's network (the internet add-on's
 * uplink). Cables connecting to this block grant internet access to all
 * computers on that cable network. The one generated at (0, 0, 0) is
 * unbreakable; a gateway a player places ({@link #CRAFTED}, made with the
 * recipe that {@code internet.gatewayRecipe} turns on) breaks with a pickaxe
 * and drops itself.
 */
public class InternetGatewayBlock extends Block {

    public static final MapCodec<InternetGatewayBlock> CODEC = simpleCodec(InternetGatewayBlock::new);

    /** Placed by a player: breakable, drops itself. False for the world's generated gateway. */
    public static final BooleanProperty CRAFTED = BooleanProperty.create("crafted");

    public InternetGatewayBlock(BlockBehaviour.Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(CRAFTED, false));
    }

    @Override
    protected MapCodec<? extends Block> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(CRAFTED);
    }

    @Override
    public BlockState getStateForPlacement(BlockPlaceContext context) {
        return defaultBlockState().setValue(CRAFTED, true);
    }

    @Override
    protected float getDestroyProgress(BlockState state, Player player, BlockGetter level, BlockPos pos) {
        return state.getValue(CRAFTED) ? super.getDestroyProgress(state, player, level, pos) : 0.0f;
    }
}
