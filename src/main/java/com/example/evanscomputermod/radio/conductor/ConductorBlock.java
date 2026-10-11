package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioContent;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelAccessor;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.PipeBlock;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.SimpleWaterloggedBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

/**
 * The shared base of every one-per-block RF conductor (spec "block wires"):
 * wire tiers, insulators, the feed point, coax and hardline. Like Mekanism or
 * AE2 cables they auto-connect on six sides (one boolean property per side),
 * place floating, and are waterloggable.
 *
 * <p>Each side has a {@link SideKind}: bare-conductor sides join bare sides
 * (and blocks in {@code #evanscomputermod:rf_conductors}: lightning rods,
 * iron bars, chains, metal blocks), coax sides join coax sides (and blocks in
 * {@code #evanscomputermod:rf_coax_ports}: amplifiers, tuners, radios). A
 * wrench ({@code #evanscomputermod:rf_wrenches}) cuts or restores the
 * connection on the clicked side; cuts live in the {@link ConductorBlockEntity}
 * so they survive saves and Sable moves.
 *
 * <p>Visual thickness ({@code thicknessPx}) is separate from electrical
 * radius, which comes from the {@code evanscomputermod:rf_conductor} data map.
 */
public class ConductorBlock extends Block implements SimpleWaterloggedBlock, EntityBlock {
    public static final BooleanProperty NORTH = PipeBlock.NORTH;
    public static final BooleanProperty SOUTH = PipeBlock.SOUTH;
    public static final BooleanProperty EAST = PipeBlock.EAST;
    public static final BooleanProperty WEST = PipeBlock.WEST;
    public static final BooleanProperty UP = PipeBlock.UP;
    public static final BooleanProperty DOWN = PipeBlock.DOWN;
    public static final BooleanProperty WATERLOGGED = BlockStateProperties.WATERLOGGED;

    /** What a side joins. */
    public enum SideKind { NONE, BARE, COAX }

    /** The block's role in an antenna graph. */
    public enum Role {
        /** A bare conductor: part of an antenna. */
        CONDUCTOR,
        /** Holds conductors mechanically; breaks the electrical graph. */
        INSULATOR,
        /** The antenna's feed: insulating gap with a coax port. */
        FEED,
        /** Feedline (coax, hardline, arrestor). */
        COAX
    }

    private final Role role;
    private final float thicknessPx;
    private final boolean collision;
    private final VoxelShape[] shapes = new VoxelShape[64];

    public ConductorBlock(Properties properties, Role role, float thicknessPx, boolean collision) {
        super(properties);
        this.role = role;
        this.thicknessPx = thicknessPx;
        this.collision = collision;
        BlockState def = stateDefinition.any();
        for (Direction d : Direction.values()) def = def.setValue(property(d), false);
        registerDefaultState(defaultState(def.setValue(WATERLOGGED, false)));
        double lo = 8 - thicknessPx / 2.0, hi = 8 + thicknessPx / 2.0;
        for (int mask = 0; mask < 64; mask++) {
            VoxelShape s = Block.box(lo, lo, lo, hi, hi, hi);
            for (Direction d : Direction.values()) {
                if ((mask & (1 << d.ordinal())) == 0) continue;
                s = Shapes.or(s, arm(d, lo, hi));
            }
            shapes[mask] = s.optimize();
        }
    }

    /** Subclasses set their extra properties' defaults here. */
    protected BlockState defaultState(BlockState s) {
        return s;
    }

    private static VoxelShape arm(Direction d, double lo, double hi) {
        return switch (d) {
            case NORTH -> Block.box(lo, lo, 0, hi, hi, lo);
            case SOUTH -> Block.box(lo, lo, hi, hi, hi, 16);
            case WEST -> Block.box(0, lo, lo, lo, hi, hi);
            case EAST -> Block.box(hi, lo, lo, 16, hi, hi);
            case DOWN -> Block.box(lo, 0, lo, hi, lo, hi);
            case UP -> Block.box(lo, hi, lo, hi, 16, hi);
        };
    }

    public Role role() { return role; }

    public float thicknessPx() { return thicknessPx; }

    public static BooleanProperty property(Direction d) {
        return switch (d) {
            case NORTH -> NORTH;
            case SOUTH -> SOUTH;
            case EAST -> EAST;
            case WEST -> WEST;
            case UP -> UP;
            case DOWN -> DOWN;
        };
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(NORTH, SOUTH, EAST, WEST, UP, DOWN, WATERLOGGED);
    }

    /** What this block's {@code side} joins in {@code state}. */
    public SideKind sideKind(BlockState state, Direction side) {
        return role == Role.COAX ? SideKind.COAX : SideKind.BARE;
    }

    // ------------------------------------------------------------ connections

    /** True if the block at {@code pos} has its {@code side} cut by a wrench. */
    public static boolean isCut(BlockGetter level, BlockPos pos, Direction side) {
        return level.getBlockEntity(pos) instanceof ConductorBlockEntity be && be.isCut(side);
    }

    /** Whether {@code state} at {@code pos} connects towards {@code side}, given what is there now. */
    public boolean connects(BlockGetter level, BlockPos pos, BlockState state, Direction side) {
        SideKind mine = sideKind(state, side);
        if (mine == SideKind.NONE || isCut(level, pos, side)) return false;
        BlockPos np = pos.relative(side);
        BlockState n = level.getBlockState(np);
        if (n.getBlock() instanceof ConductorBlock other) {
            if (other.sideKind(n, side.getOpposite()) != mine || isCut(level, np, side.getOpposite())) return false;
            // Insulators hold wires but two insulators (or an insulator and a feed point) have nothing to join.
            if (role != Role.CONDUCTOR && other.role != Role.CONDUCTOR && mine == SideKind.BARE) return false;
            return true;
        }
        if (mine != SideKind.BARE) return n.is(RadioConductors.RF_COAX_PORTS);
        // Other blocks: tagged conductors join electrically; tagged insulators hold a conductor's wire.
        if (RadioConductors.insulates(n)) return role == Role.CONDUCTOR;
        return n.is(RadioContent.RF_CONDUCTORS);
    }

    /**
     * Places {@code state} and works out its connections the way a player's
     * placement does ({@code setBlock} alone skips getStateForPlacement).
     * For scenarios, tests and structure builders.
     */
    public static void placeConnected(Level level, BlockPos pos, BlockState state) {
        level.setBlock(pos, state, Block.UPDATE_ALL);
        if (state.getBlock() instanceof ConductorBlock c) {
            BlockState placed = level.getBlockState(pos);
            level.setBlock(pos, c.withConnections(level, pos, placed), Block.UPDATE_ALL);
        }
    }

    protected BlockState withConnections(BlockGetter level, BlockPos pos, BlockState state) {
        for (Direction d : Direction.values()) state = state.setValue(property(d), connects(level, pos, state, d));
        return state;
    }

    @Override
    public @Nullable BlockState getStateForPlacement(BlockPlaceContext ctx) {
        BlockState s = placementState(ctx);
        if (s == null) return null;
        s = s.setValue(WATERLOGGED, ctx.getLevel().getFluidState(ctx.getClickedPos()).getType() == Fluids.WATER);
        return withConnections(ctx.getLevel(), ctx.getClickedPos(), s);
    }

    /** The state before connections are worked out (subclasses add an axis, say). */
    protected @Nullable BlockState placementState(BlockPlaceContext ctx) {
        return defaultBlockState();
    }

    @Override
    protected BlockState updateShape(BlockState state, Direction direction, BlockState neighborState,
                                     LevelAccessor level, BlockPos pos, BlockPos neighborPos) {
        if (state.getValue(WATERLOGGED)) level.scheduleTick(pos, Fluids.WATER, Fluids.WATER.getTickDelay(level));
        if (level instanceof Level l && !l.isClientSide()) AntennaManager.blockChanged(l, pos);
        return state.setValue(property(direction), connects(level, pos, state, direction));
    }

    @Override
    protected FluidState getFluidState(BlockState state) {
        return state.getValue(WATERLOGGED) ? Fluids.WATER.getSource(false) : super.getFluidState(state);
    }

    @Override
    protected void onPlace(BlockState state, Level level, BlockPos pos, BlockState oldState, boolean movedByPiston) {
        super.onPlace(state, level, pos, oldState, movedByPiston);
        if (!level.isClientSide() && !oldState.is(state.getBlock())) AntennaManager.blockChanged(level, pos);
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        if (!level.isClientSide() && !state.is(newState.getBlock())) AntennaManager.blockChanged(level, pos);
        super.onRemove(state, level, pos, newState, movedByPiston);
    }

    // ------------------------------------------------------------ shape

    protected int connectionMask(BlockState state) {
        int mask = 0;
        for (Direction d : Direction.values()) if (state.getValue(property(d))) mask |= 1 << d.ordinal();
        return mask;
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        return shapes[connectionMask(state)];
    }

    @Override
    protected VoxelShape getCollisionShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        return collision ? getShape(state, level, pos, context) : Shapes.empty();
    }

    @Override
    protected boolean propagatesSkylightDown(BlockState state, BlockGetter level, BlockPos pos) {
        return !state.getValue(WATERLOGGED);
    }

    @Override
    protected BlockState rotate(BlockState state, Rotation rotation) {
        BlockState out = state;
        for (Direction d : Direction.Plane.HORIZONTAL) out = out.setValue(property(rotation.rotate(d)), state.getValue(property(d)));
        return out;
    }

    @Override
    protected BlockState mirror(BlockState state, Mirror mirror) {
        BlockState out = state;
        for (Direction d : Direction.Plane.HORIZONTAL) out = out.setValue(property(mirror.mirror(d)), state.getValue(property(d)));
        return out;
    }

    // ------------------------------------------------------------ wrench

    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        if (!stack.is(RadioContent.RF_WRENCHES)) return super.useItemOn(stack, state, level, pos, player, hand, hit);
        Direction side = sideFromHit(state, pos, hit);
        if (!level.isClientSide()) {
            boolean cut = toggleCut(level, pos, side);
            if (player != null)
                player.displayClientMessage(Component.translatable(cut ? "message.evanscomputermod.rf_wrench.cut"
                        : "message.evanscomputermod.rf_wrench.restored", side.getName()), true);
        }
        return ItemInteractionResult.sidedSuccess(level.isClientSide());
    }

    /**
     * The side a click means: the arm it landed on, or (on the centre piece)
     * the clicked face.
     */
    public Direction sideFromHit(BlockState state, BlockPos pos, BlockHitResult hit) {
        Vec3 local = hit.getLocation().subtract(Vec3.atCenterOf(pos));
        double half = thicknessPx / 32.0 + 1e-3;
        double ax = Math.abs(local.x), ay = Math.abs(local.y), az = Math.abs(local.z);
        double m = Math.max(ax, Math.max(ay, az));
        if (m <= half) return hit.getDirection();
        if (m == ax) return local.x > 0 ? Direction.EAST : Direction.WEST;
        if (m == ay) return local.y > 0 ? Direction.UP : Direction.DOWN;
        return local.z > 0 ? Direction.SOUTH : Direction.NORTH;
    }

    /** Cuts (or restores) {@code side}; returns true if it is now cut. */
    public boolean toggleCut(Level level, BlockPos pos, Direction side) {
        if (!(level.getBlockEntity(pos) instanceof ConductorBlockEntity be)) return false;
        boolean cut = !be.isCut(side);
        be.setCut(side, cut);
        BlockState state = level.getBlockState(pos);
        BlockState now = state.setValue(property(side), connects(level, pos, state, side));
        if (now != state) level.setBlock(pos, now, Block.UPDATE_ALL);
        // Let the neighbour re-check its side against ours.
        BlockPos np = pos.relative(side);
        BlockState n = level.getBlockState(np);
        if (n.getBlock() instanceof ConductorBlock other) {
            BlockState nn = n.setValue(property(side.getOpposite()), other.connects(level, np, n, side.getOpposite()));
            if (nn != n) level.setBlock(np, nn, Block.UPDATE_ALL);
        }
        AntennaManager.blockChanged(level, pos);
        return cut;
    }

    // ------------------------------------------------------------ block entity

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new ConductorBlockEntity(pos, state);
    }
}
//?}
