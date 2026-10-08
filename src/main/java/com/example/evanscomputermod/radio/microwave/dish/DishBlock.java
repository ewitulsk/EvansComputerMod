package com.example.evanscomputermod.radio.microwave.dish;

//? if <=1.21.1 {
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.level.block.state.properties.IntegerProperty;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

/**
 * A parabolic dish, 1×1, 2×2 or 3×3 blocks across.
 *
 * <p><b>Multiblock:</b> every block of the dish is this same block, with a
 * {@link #PART} property saying which piece it is (see {@link DishSize}); only
 * the controller part has a block entity (aim, peripheral). Placing the item
 * fills the whole square or nothing; breaking any part breaks the whole dish
 * and drops one item (the loot table only drops from the controller part).
 * Each part's model is its own slice of the dish, so the dish looks whole.
 *
 * <p>{@link #FACING} is the mount's direction (the way the placer looked); the
 * aim starts there and is then set in degrees by hand or by a computer. Right
 * click shows the aim; sneak + right click nudges it towards the side of the
 * dish you clicked (left/right: yaw, top/bottom: elevation), by half a
 * beamwidth when a radio is connected (1 degree otherwise).
 */
public class DishBlock extends BaseEntityBlock {
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    public static final IntegerProperty PART = IntegerProperty.create("part", 0, 8);

    private static final VoxelShape NORTH = Block.box(0, 0, 3, 16, 16, 12);
    private static final VoxelShape SOUTH = Block.box(0, 0, 4, 16, 16, 13);
    private static final VoxelShape EAST = Block.box(4, 0, 0, 13, 16, 16);
    private static final VoxelShape WEST = Block.box(3, 0, 0, 12, 16, 16);

    private final DishSize size;

    public DishBlock(DishSize size, Properties properties) {
        super(properties);
        this.size = size;
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH).setValue(PART, size.controllerPart()));
    }

    public DishSize size() {
        return size;
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return simpleCodec(p -> new DishBlock(size, p));
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, PART);
    }

    /** World position of a part relative to the controller at {@code controller}. */
    public static BlockPos partPos(DishSize size, BlockPos controller, Direction facing, int part) {
        return controller.relative(facing.getClockWise(), size.rightOf(part)).above(size.upOf(part));
    }

    /** The controller of the dish {@code state} at {@code pos} belongs to. */
    public BlockPos controllerOf(BlockState state, BlockPos pos) {
        int part = state.getValue(PART);
        return pos.relative(state.getValue(FACING).getClockWise(), -size.rightOf(part)).below(size.upOf(part));
    }

    public boolean isController(BlockState state) {
        return state.getValue(PART) == size.controllerPart();
    }

    @Override
    public @Nullable BlockState getStateForPlacement(BlockPlaceContext ctx) {
        Direction facing = ctx.getHorizontalDirection();
        Level level = ctx.getLevel();
        for (int part = 0; part < size.parts(); part++) {
            BlockPos p = partPos(size, ctx.getClickedPos(), facing, part);
            if (p.equals(ctx.getClickedPos())) continue;
            if (!level.isInWorldBounds(p) || !level.getBlockState(p).canBeReplaced(ctx)) return null;
        }
        return defaultBlockState().setValue(FACING, facing).setValue(PART, size.controllerPart());
    }

    @Override
    public void setPlacedBy(Level level, BlockPos pos, BlockState state, @Nullable LivingEntity placer, ItemStack stack) {
        super.setPlacedBy(level, pos, state, placer, stack);
        if (!level.isClientSide()) placeParts(level, pos, state.getValue(FACING));
    }

    /**
     * Build the whole dish with its controller at {@code controller} (used by
     * placement, scenarios and tests). Returns false, placing nothing, if any
     * part's block isn't air or replaceable.
     */
    public boolean place(Level level, BlockPos controller, Direction facing) {
        for (int part = 0; part < size.parts(); part++) {
            BlockPos p = partPos(size, controller, facing, part);
            if (!level.getBlockState(p).canBeReplaced()) return false;
        }
        level.setBlock(controller, defaultBlockState().setValue(FACING, facing).setValue(PART, size.controllerPart()), Block.UPDATE_ALL);
        placeParts(level, controller, facing);
        if (level.getBlockEntity(controller) instanceof DishBlockEntity be) be.resetAim(facing);
        return true;
    }

    private void placeParts(Level level, BlockPos controller, Direction facing) {
        for (int part = 0; part < size.parts(); part++) {
            if (part == size.controllerPart()) continue;
            level.setBlock(partPos(size, controller, facing, part),
                    defaultBlockState().setValue(FACING, facing).setValue(PART, part), Block.UPDATE_ALL);
        }
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean moved) {
        if (!level.isClientSide() && !state.is(newState.getBlock())) {
            BlockPos controller = controllerOf(state, pos);
            Direction facing = state.getValue(FACING);
            for (int part = 0; part < size.parts(); part++) {
                BlockPos p = partPos(size, controller, facing, part);
                if (p.equals(pos)) continue;
                BlockState s = level.getBlockState(p);
                if (s.is(this) && s.getValue(FACING) == facing && s.getValue(PART) == part)
                    level.destroyBlock(p, part == size.controllerPart());
            }
        }
        super.onRemove(state, level, pos, newState, moved);
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext ctx) {
        return switch (state.getValue(FACING)) {
            case SOUTH -> SOUTH;
            case EAST -> EAST;
            case WEST -> WEST;
            default -> NORTH;
        };
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return isController(state) ? new DishBlockEntity(pos, state) : null;
    }

    /** The dish's block entity from any of its parts. */
    public static @Nullable DishBlockEntity controllerEntity(Level level, BlockPos pos) {
        BlockState s = level.getBlockState(pos);
        if (!(s.getBlock() instanceof DishBlock dish)) return null;
        return level.getBlockEntity(dish.controllerOf(s, pos)) instanceof DishBlockEntity be ? be : null;
    }

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (level.isClientSide()) return InteractionResult.SUCCESS;
        DishBlockEntity be = controllerEntity(level, pos);
        if (be == null) return InteractionResult.PASS;
        if (player.isShiftKeyDown()) {
            Direction facing = state.getValue(FACING);
            Vec3 centre = be.centreInBlockSpace();
            Vec3 off = hit.getLocation().subtract(centre);
            Direction right = facing.getClockWise();
            double r = off.x * right.getStepX() + off.z * right.getStepZ(), u = off.y;
            double step = be.nudgeStepDeg();
            if (Math.abs(r) >= Math.abs(u)) be.nudge(r > 0 ? step : -step, 0);
            else be.nudge(0, u > 0 ? step : -step);
        }
        player.displayClientMessage(Component.translatable("message.evanscomputermod.dish.aim",
                Component.translatable("block.evanscomputermod." + size.id),
                String.format("%.2f", be.yaw()), String.format("%.2f", be.elevation()), be.radioSummary()), true);
        return InteractionResult.CONSUME;
    }
}
//?}
