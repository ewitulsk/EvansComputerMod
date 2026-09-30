package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

/**
 * Item Decoder: turns ledger entries back into items when a program asks.
 * Pushes them into the inventory in front of it, or holds them in a nine-slot
 * buffer that hoppers and funnels can pull from.
 */
public class DecoderBlock extends StorageDeviceBlock {
    public static final MapCodec<DecoderBlock> CODEC = simpleCodec(DecoderBlock::new);

    public DecoderBlock(Properties properties) {
        super(properties);
    }

    @Override
    protected MapCodec<? extends HorizontalDirectionalBlock> codec() {
        return CODEC;
    }

    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new DecoderBlockEntity(pos, state);
    }
}
//?}
