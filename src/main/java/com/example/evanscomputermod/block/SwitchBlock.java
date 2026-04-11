package com.example.evanscomputermod.block;

import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.BaseEntityBlock;
import org.jspecify.annotations.Nullable;

/**
 * Switch block — a network-switch variant of {@link TerminalBlock}. Uses the
 * same block state, textures, facing, placement, redstone, cable/interface
 * hookup and screen as the terminal; only the block entity it spawns is
 * different. Since `SwitchBlock extends TerminalBlock`, existing
 * `instanceof TerminalBlock` checks in InterfaceBlock and NetworkCableBlock
 * treat switches as terminals automatically.
 */
public class SwitchBlock extends TerminalBlock {

    public static final MapCodec<SwitchBlock> CODEC = simpleCodec(SwitchBlock::new);

    public SwitchBlock(BlockBehaviour.Properties properties) {
        super(properties);
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new SwitchBlockEntity(pos, state);
    }
}
