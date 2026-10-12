package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
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
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

import java.util.EnumMap;
import java.util.Map;

/**
 * Charging Dock: a desk stand for one ePhone. Right-click it holding a phone
 * to dock it, with an empty hand to take it back. A docked phone is off (not
 * in a hotbar) and charges at {@code battery.dockPerMinute}; the stand's lamp
 * ({@code active}) is lit while it charges. Accepts FE from any side (stored;
 * spent only when {@code battery.dockFePerPercent} is above 0).
 */
public class ChargingDockBlock extends BaseEntityBlock {
    public static final MapCodec<ChargingDockBlock> CODEC = simpleCodec(ChargingDockBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    public static final BooleanProperty ACTIVE = BooleanProperty.create("active");

    /** Base plate and back rest, for a dock facing north (the phone's screen faces the front). */
    private static final double[][] BOXES = {{2, 0, 3, 14, 2, 13}, {3, 2, 11, 13, 11, 13}};
    private static final Map<Direction, VoxelShape> SHAPES = new EnumMap<>(Direction.class);

    static {
        for (Direction d : Direction.Plane.HORIZONTAL) {
            VoxelShape s = Shapes.empty();
            for (double[] b : BOXES) s = Shapes.or(s, rotated(b, d));
            SHAPES.put(d, s);
        }
    }

    /** {@code b} (north-facing pixels) turned to face {@code d}. */
    private static VoxelShape rotated(double[] b, Direction d) {
        double x0 = b[0], z0 = b[2], x1 = b[3], z1 = b[5];
        double[] r = switch (d) {
            case SOUTH -> new double[]{16 - x1, 16 - z1, 16 - x0, 16 - z0};
            case EAST -> new double[]{16 - z1, x0, 16 - z0, x1};
            case WEST -> new double[]{z0, 16 - x1, z1, 16 - x0};
            default -> new double[]{x0, z0, x1, z1};
        };
        return Block.box(r[0], b[1], r[1], r[2], b[4], r[3]);
    }

    public ChargingDockBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH).setValue(ACTIVE, false));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, ACTIVE);
    }

    @Override
    public BlockState getStateForPlacement(BlockPlaceContext ctx) {
        return defaultBlockState().setValue(FACING, ctx.getHorizontalDirection().getOpposite());
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext ctx) {
        return SHAPES.get(state.getValue(FACING));
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new ChargingDockBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null
                : createTickerHelper(type, PhoneContent.CHARGING_DOCK_BE.get(), ChargingDockBlockEntity::serverTick);
    }

    /** Right-click with a phone: dock it. */
    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (!PhoneItemData.isPhone(stack) || !(level.getBlockEntity(pos) instanceof ChargingDockBlockEntity be))
            return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (!be.phone().isEmpty()) return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (!level.isClientSide()) {
            be.setPhone(stack.copyWithCount(1));
            stack.shrink(1);
        }
        return ItemInteractionResult.sidedSuccess(level.isClientSide());
    }

    /** Empty hand: take the phone back (or show the charge). */
    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!(level.getBlockEntity(pos) instanceof ChargingDockBlockEntity be)) return InteractionResult.PASS;
        if (!level.isClientSide()) {
            ItemStack phone = be.phone();
            if (phone.isEmpty()) {
                player.displayClientMessage(be.status(), true);
            } else {
                be.setPhone(ItemStack.EMPTY);
                if (!player.getInventory().add(phone)) player.drop(phone, false);
            }
        }
        return InteractionResult.sidedSuccess(level.isClientSide());
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean moved) {
        if (!state.is(newState.getBlock()) && level.getBlockEntity(pos) instanceof ChargingDockBlockEntity be) {
            Block.popResource(level, pos, be.phone());
        }
        super.onRemove(state, level, pos, newState, moved);
    }
}
//?}
