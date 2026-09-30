package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.WireConnections;
import com.example.evanscomputermod.sensor.wire.WireTerminal;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.FaceAttachedHorizontalDirectionalBlock;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.AttachFace;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

import java.util.EnumMap;
import java.util.Map;

/**
 * Lidar Sensor: a small puck mounted on a floor, wall or ceiling (like a
 * button). It has no collision, so it never blocks rays (its own or another
 * sensor's). Wire it to a Wired Sensor Module to use it.
 *
 * <p>Sensor frame: forward is {@link #FACING} (out of the wall for wall
 * mounts, the direction the player looked when placing on a floor or
 * ceiling), up is the block (structure) up. Scans sweep around that up axis.
 */
public class LidarSensorBlock extends FaceAttachedHorizontalDirectionalBlock implements EntityBlock, IWireHost {
    public static final MapCodec<LidarSensorBlock> CODEC = simpleCodec(LidarSensorBlock::new);
    public static final BooleanProperty ACTIVE = BlockStateProperties.POWERED;

    /** Head (spinning part) centre, pixels, in the floor-north / wall-north model frames. */
    static final Vec3 HEAD_FLOOR = new Vec3(8, 3.5, 8);
    static final Vec3 HEAD_WALL = new Vec3(8, 8.5, 12);

    private static final WireTerminal TERMINAL_FLOOR = new WireTerminal(7, 0, 11, 9, 1.5, 12.5).withOrigin(8, 0.5, 12);
    private static final WireTerminal TERMINAL_WALL = new WireTerminal(7, 3, 14.5, 9, 5, 16).withOrigin(8, 3.5, 15.5);

    private static final Map<AttachFace, Map<Direction, WireTerminal>> TERMINALS = new EnumMap<>(AttachFace.class);
    private static final Map<AttachFace, Map<Direction, VoxelShape>> SHAPES = new EnumMap<>(AttachFace.class);

    static {
        for(AttachFace face : AttachFace.values()) {
            Map<Direction, WireTerminal> t = new EnumMap<>(Direction.class);
            Map<Direction, VoxelShape> s = new EnumMap<>(Direction.class);
            for(Direction facing : Direction.Plane.HORIZONTAL) {
                t.put(facing, place(face, facing, face == AttachFace.WALL ? TERMINAL_WALL : TERMINAL_FLOOR));
                VoxelShape shape = Shapes.empty();
                for(WireTerminal box : face == AttachFace.WALL ? wallBoxes() : floorBoxes()) {
                    shape = Shapes.or(shape, place(face, facing, box).getShape());
                }
                s.put(facing, shape.optimize());
            }
            TERMINALS.put(face, t);
            SHAPES.put(face, s);
        }
    }

    private static WireTerminal[] floorBoxes() {
        return new WireTerminal[] {
                new WireTerminal(5, 0, 5, 11, 1, 11, 0),        // base plate
                new WireTerminal(6, 1, 6, 10, 5, 10, 0),        // head
                new WireTerminal(7, 0, 11, 9, 1.5, 12.5, 0),    // connector
        };
    }

    private static WireTerminal[] wallBoxes() {
        return new WireTerminal[] {
                new WireTerminal(5, 3, 15, 11, 11, 16, 0),      // wall plate
                new WireTerminal(6, 5, 10, 10, 7, 15, 0),       // shelf
                new WireTerminal(6, 7, 10, 10, 10, 14, 0),      // head
        };
    }

    /** A box/point defined in the floor-north or wall-north frame, placed like the block model. */
    static WireTerminal place(AttachFace face, Direction facing, WireTerminal t) {
        Rotation rotation = WireTerminal.rotationFromNorth(facing);
        if(face == AttachFace.CEILING)
            return t.flipUpsideDown().rotateAroundY(rotation.getRotated(Rotation.CLOCKWISE_180));
        return t.rotateAroundY(rotation);
    }

    public LidarSensorBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any()
                .setValue(FACING, Direction.NORTH)
                .setValue(FACE, AttachFace.FLOOR)
                .setValue(ACTIVE, false));
    }

    @Override
    protected MapCodec<? extends FaceAttachedHorizontalDirectionalBlock> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACE, FACING, ACTIVE);
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        return SHAPES.get(state.getValue(FACE)).get(state.getValue(FACING));
    }

    @Override
    protected VoxelShape getCollisionShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        return Shapes.empty();
    }

    // ------------------------------------------------------------ sensor frame

    /** Where rays start, block-local (0..1). */
    public static Vec3 headCentre(BlockState state) {
        AttachFace face = state.getValue(FACE);
        Vec3 px = face == AttachFace.WALL ? HEAD_WALL : HEAD_FLOOR;
        var point = place(face, state.getValue(FACING), new WireTerminal(px.x, px.y, px.z, px.x, px.y, px.z, 0));
        return point.getOrigin();
    }

    public static Direction forward(BlockState state) {
        return state.getValue(FACING);
    }

    // ------------------------------------------------------------ wires

    @Override
    public int terminalCount() {
        return 1;
    }

    @Override
    @Nullable
    public WireTerminal terminal(BlockState state, int index) {
        if(index != 0 || !(state.getBlock() instanceof LidarSensorBlock))
            return null;
        return TERMINALS.get(state.getValue(FACE)).get(state.getValue(FACING));
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        if(!state.is(newState.getBlock()))
            WireConnections.breakAll(level, pos, terminalCount());
        super.onRemove(state, level, pos, newState, movedByPiston);
    }

    // ------------------------------------------------------------ block entity

    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new LidarSensorBlockEntity(pos, state);
    }

    @Nullable
    @Override
    @SuppressWarnings("unchecked")
    public <T extends BlockEntity> net.minecraft.world.level.block.entity.BlockEntityTicker<T> getTicker(
            Level level, BlockState state, net.minecraft.world.level.block.entity.BlockEntityType<T> type) {
        if(level.isClientSide() || type != SensorContent.LIDAR_SENSOR_BE.get())
            return null;
        return (l, p, s, be) -> LidarSensorBlockEntity.serverTick(l, p, s, (LidarSensorBlockEntity) be);
    }
}
//?}
