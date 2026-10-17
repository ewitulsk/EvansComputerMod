package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.phys.BlockHitResult;
import org.jetbrains.annotations.Nullable;

/**
 * Card Reader/Writer: a one-card slot a computer next to it reads and writes as the
 * {@code card_reader} peripheral. Right-click with a card to put it in; right-click with
 * an empty hand to take it out (or, when it is empty, or while sneaking, to open its
 * screen: the "Auto Eject Sim" setting and the card's state). Hoppers and pipes insert
 * and extract through the item handler capability on every side. Comparator: 15 while
 * a card is in.
 */
public class CardReaderBlock extends BaseEntityBlock {
    public static final MapCodec<CardReaderBlock> CODEC = simpleCodec(CardReaderBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    /** A card is in the slot (drawn sticking out of the front). */
    public static final BooleanProperty CARD = BooleanProperty.create("card");
    /** The base plate and the body (models/card_reader.bbmodel), without the card sticking out of the front. */
    private static final net.minecraft.world.phys.shapes.VoxelShape SHAPE = net.minecraft.world.phys.shapes.Shapes.or(
            Block.box(0, 0, 0, 16, 2, 16), Block.box(1, 2, 1, 15, 14, 15));

    public CardReaderBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH).setValue(CARD, false));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    /** Structure templates (villages, structure blocks) turn the block with its ports. */
    @Override
    protected BlockState rotate(BlockState state, net.minecraft.world.level.block.Rotation rotation) {
        return state.setValue(FACING, rotation.rotate(state.getValue(FACING)));
    }

    @Override
    protected BlockState mirror(BlockState state, net.minecraft.world.level.block.Mirror mirror) {
        return state.rotate(mirror.getRotation(state.getValue(FACING)));
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, CARD);
    }

    @Override
    public BlockState getStateForPlacement(BlockPlaceContext ctx) {
        return defaultBlockState().setValue(FACING, ctx.getHorizontalDirection().getOpposite());
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    protected net.minecraft.world.phys.shapes.VoxelShape getShape(BlockState state, net.minecraft.world.level.BlockGetter level, BlockPos pos,
                                                                  net.minecraft.world.phys.shapes.CollisionContext ctx) {
        return SHAPE;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new CardReaderBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null : createTickerHelper(type, CardContent.CARD_READER_BE.get(), CardReaderBlockEntity::serverTick);
    }

    /** Right-click with a card: put it in (one card; the slot must be empty). */
    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (!CardReaderBlockEntity.isCard(stack) || !(level.getBlockEntity(pos) instanceof CardReaderBlockEntity be))
            return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (!level.isClientSide()) {
            if (be.hasCard()) {
                player.displayClientMessage(Component.translatable("message.evanscomputermod.card_reader.occupied"), true);
            } else {
                be.insertByHand(stack.copyWithCount(1));
                stack.shrink(1);   // in creative too: the card is handed over, not placed
                player.displayClientMessage(Component.translatable("message.evanscomputermod.card_reader.inserted"), true);
            }
        }
        return ItemInteractionResult.sidedSuccess(level.isClientSide());
    }

    /** Empty hand: take the card out; with no card in (or sneaking): open the screen. */
    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!(level.getBlockEntity(pos) instanceof CardReaderBlockEntity be)) return InteractionResult.PASS;
        if (!level.isClientSide()) {
            if (be.hasCard() && !player.isShiftKeyDown()) {
                ItemStack out = be.takeByHand();
                if (!player.getInventory().add(out)) player.drop(out, false);
                player.displayClientMessage(Component.translatable("message.evanscomputermod.card_reader.taken"), true);
            } else {
                player.openMenu(be);
            }
        }
        return InteractionResult.sidedSuccess(level.isClientSide());
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean moved) {
        if (!state.is(newState.getBlock()) && level.getBlockEntity(pos) instanceof CardReaderBlockEntity be)
            Block.popResource(level, pos, be.takeByHand());
        super.onRemove(state, level, pos, newState, moved);
    }

    @Override
    protected boolean hasAnalogOutputSignal(BlockState state) {
        return true;
    }

    @Override
    protected int getAnalogOutputSignal(BlockState state, Level level, BlockPos pos) {
        return level.getBlockEntity(pos) instanceof CardReaderBlockEntity be && be.hasCard() ? 15 : 0;
    }
}
//?}
