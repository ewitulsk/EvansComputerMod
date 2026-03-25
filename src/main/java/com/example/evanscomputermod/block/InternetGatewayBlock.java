package com.example.evanscomputermod.block;

import com.mojang.serialization.MapCodec;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;

/**
 * Unbreakable block placed at (0, 0, 0) that serves as the physical connection
 * point to the TAP bridge (real internet). Cables connecting to this block
 * grant internet access to all computers on that cable network.
 */
public class InternetGatewayBlock extends Block {

    public static final MapCodec<InternetGatewayBlock> CODEC = simpleCodec(InternetGatewayBlock::new);

    public InternetGatewayBlock(BlockBehaviour.Properties properties) {
        super(properties);
    }

    @Override
    protected MapCodec<? extends Block> codec() {
        return CODEC;
    }
}
