package com.example.customworld.block;

import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;

import javax.annotation.Nullable;

/**
 * Terminal Block - A computer terminal that opens a custom UI.
 * Uses gold block textures (defined in blockstate JSON).
 */
public class TerminalBlock extends BaseEntityBlock {
    
    public static final MapCodec<TerminalBlock> CODEC = simpleCodec(TerminalBlock::new);
    
    public TerminalBlock(BlockBehaviour.Properties properties) {
        super(properties);
    }
    
    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }
    
    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new TerminalBlockEntity(pos, state);
    }
    
    @Override
    protected RenderShape getRenderShape(BlockState state) {
        // Use the model defined in blockstate JSON (gold block)
        return RenderShape.MODEL;
    }
    
    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, 
            BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide && level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            // Initialize WASM when terminal is first opened
            te.initializeWasm();
            player.openMenu(te, pos);
        }
        return InteractionResult.sidedSuccess(level.isClientSide);
    }
}
