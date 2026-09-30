package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.StorageRules;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import org.jetbrains.annotations.Nullable;

/** Item Encoder: feed it with hoppers or funnels, or right-click it with a stack. */
public class EncoderBlock extends StorageDeviceBlock {
    public static final MapCodec<EncoderBlock> CODEC = simpleCodec(EncoderBlock::new);

    public EncoderBlock(Properties properties) {
        super(properties);
    }

    @Override
    protected MapCodec<? extends HorizontalDirectionalBlock> codec() {
        return CODEC;
    }

    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new EncoderBlockEntity(pos, state);
    }

    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (stack.isEmpty()) return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (level.isClientSide()) return ItemInteractionResult.SUCCESS;
        if (!(level.getBlockEntity(pos) instanceof EncoderBlockEntity enc) || !(level instanceof ServerLevel sl)) {
            return ItemInteractionResult.FAIL;
        }
        String why = StorageRules.whyNotEncodable(stack, StorageLedger.get(sl.getServer()));
        ItemStack left = why == null ? enc.encode(stack.copy(), false) : stack;
        int done = stack.getCount() - left.getCount();
        if (done > 0) {
            if (!player.getAbilities().instabuild) player.setItemInHand(hand, left);
            level.playSound(null, pos, SoundEvents.ENDERMAN_TELEPORT, SoundSource.BLOCKS, 0.4f, 1.6f);
            player.displayClientMessage(Component.translatable("message.evanscomputermod.encoded", done,
                    stack.getHoverName()), true);
        } else {
            player.displayClientMessage(why != null ? Component.literal(why)
                    : Component.translatable("message.evanscomputermod.encoder_no_room"), true);
        }
        return ItemInteractionResult.CONSUME;
    }
}
//?}
