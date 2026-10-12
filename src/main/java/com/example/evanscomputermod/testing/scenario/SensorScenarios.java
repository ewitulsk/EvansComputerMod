package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.sensor.LidarSensorBlock;
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.wire.BlockWireEndpoint;
import com.example.evanscomputermod.sensor.wire.BlockWireEntity;
import com.example.evanscomputermod.sensor.wire.BlockWireEntityEndpoint;
import com.example.evanscomputermod.sensor.wire.IWireEndpoint;
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.ImaginaryWireEndpoint;
import com.example.evanscomputermod.sensor.wire.SensorWireItem;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.AttachFace;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Sensor scenarios (1.21.1), spawnable with {@code /ecm scenario spawn <name>}.
 *
 * <p>{@code lidar_room}: a walled room with a few obstacles, a computer at
 * its north edge (screen facing you) with a Wired Sensor Module, and a
 * lidar on the floor in the middle, wired to the module, all placed and
 * clicked in by the scenario player. The player types {@code lidar_view.py}
 * in from Python and runs it: it draws a top-down outline of what the lidar
 * sees. Walk into the room and you show up on the map.
 * <pre>
 *   y=0 floor x -9..9, z -1..14; walls y 1..2 on x = +-9, z = 14, and z = 0
 *   except a doorway at x -2..2 where the computer (0,1,0) stands.
 *   Lidar (0,1,7), facing north.
 * </pre>
 */
public final class SensorScenarios {
    public static final String PROGRAM = "lidar_view.py";
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos LIDAR = new BlockPos(0, 1, 7);

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        Scenario s = lidarRoom();
        ALL.put(s.name, s);
    }

    private SensorScenarios() {
    }

    private static Scenario lidarRoom() {
        var b = Scenario.builder("lidar_room", "a lidar in a room, drawn as a live top-down outline on the computer")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(new LidarRoom())
                .note("Setup (done for you): the room (terrain); the lidar placed on the floor, a Module Expansion Card and a Wired"
                        + " Sensor Module clicked into the computer's left side, Sensor Wire clicked from the module's connector to the"
                        + " lidar's.")
                .note("Type the lidar viewer in (" + PROGRAM + ", from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/" + PROGRAM, "/evanscomputermod/scenarios/" + PROGRAM);
        return b.note("Run the lidar viewer")
                .send("pc", "python " + PROGRAM)
                .expect("pc", "^LIDAR lidar_1 +scan \\d+", "the map is drawing")
                .waitMs(3_000, "let it draw a few scans")
                .timeLimit(60_000)
                .build();
    }

    /** The room, the module, the lidar, the wire between them, and the program on the computer's disk. */
    private static final class LidarRoom implements Scenario.Decor {
        private final List<BlockPos> floor = new ArrayList<>();
        private final List<BlockPos> walls = new ArrayList<>();
        private final List<BlockPos> obstacles = new ArrayList<>();

        LidarRoom() {
            for (int x = -9; x <= 9; x++)
                for (int z = -1; z <= 14; z++)
                    floor.add(new BlockPos(x, 0, z));
            for (int y = 1; y <= 2; y++) {
                for (int z = 0; z <= 14; z++) {
                    walls.add(new BlockPos(-9, y, z));
                    walls.add(new BlockPos(9, y, z));
                }
                for (int x = -8; x <= 8; x++) {
                    walls.add(new BlockPos(x, y, 14));
                    if (Math.abs(x) >= 3) walls.add(new BlockPos(x, y, 0));
                }
                obstacles.add(new BlockPos(-5, y, 4));    // pillars
                obstacles.add(new BlockPos(5, y, 10));
            }
            for (int x = -7; x <= -4; x++) obstacles.add(new BlockPos(x, 1, 10));   // low wall
            obstacles.add(new BlockPos(4, 1, 4));                                    // crate
            obstacles.add(new BlockPos(5, 1, 4));
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(floor);
            all.addAll(walls);
            all.addAll(obstacles);
            all.add(LIDAR);
            return all;
        }

        /** The room: vanilla terrain, set like /fill. */
        @Override
        public void terrain(ScenarioRun run) {
            PlayerKit.fill(run, floor, Blocks.SMOOTH_STONE.defaultBlockState());
            PlayerKit.fill(run, walls, Blocks.STONE_BRICKS.defaultBlockState());
            for (BlockPos p : obstacles) {
                BlockState st = p.getY() == 1 && p.getZ() == 4 && p.getX() > 0 ? Blocks.BARREL.defaultBlockState()
                        : Blocks.POLISHED_ANDESITE.defaultBlockState();
                PlayerKit.fill(run, List.of(p), st);
            }
        }

        /** What the player does: the lidar on the floor, the card and module in the left bay, the wire between them. */
        @Override
        public void build(ScenarioRun run) {
            ScenarioPlayer p = run.player();
            p.placeAgainst(SensorContent.LIDAR_SENSOR.get(), run.abs(LIDAR), Direction.SOUTH, Direction.DOWN,
                    s -> s.getValue(LidarSensorBlock.FACE) == AttachFace.FLOOR);
            List<String> said = p.installModule(run.abs(PC), SensorContent.WIRED_SENSOR_MODULE.get());
            TerminalBlockEntity pc = run.terminal("pc");
            if (pc == null || pc.getModuleBays().getStack(0).isEmpty())
                throw new IllegalStateException("the Wired Sensor Module didn't go into left_bay_1: the game said " + said);
            ItemStack wire = new ItemStack(SensorContent.SENSOR_WIRE.get(), 64);
            clickConnector(run, wire, run.abs(PC), Direction.WEST);
            clickConnector(run, wire, run.abs(LIDAR), Direction.UP);
            if (run.level().getEntitiesOfClass(BlockWireEntity.class, new AABB(run.abs(PC)).inflate(10)).isEmpty())
                throw new IllegalStateException("no sensor wire between the module and the lidar");
        }

        /** Right-click a wire host's connector 0 with Sensor Wire in hand (the item's own click handling). */
        private static void clickConnector(ScenarioRun run, ItemStack wire, BlockPos at, Direction face) {
            var host = IWireHost.getAt(run.level(), at);
            if (host == null) throw new IllegalStateException("no wire host at " + at);
            var terminal = host.terminal(run.level().getBlockState(at), 0);
            if (terminal == null) throw new IllegalStateException("no connector on " + run.level().getBlockState(at));
            Vec3 o = terminal.getOrigin();
            // A point just inside the connector's box, on the face the player clicks.
            Vec3 hit = new Vec3(at.getX() + o.x - face.getStepX() * 0.02, at.getY() + o.y - face.getStepY() * 0.02,
                    at.getZ() + o.z - face.getStepZ() * 0.02);
            // The wire's own click handler (what the block right-click event calls) with the aimed point:
            // the scenario player holds the wire and looks at the connector.
            var hands = run.player().entity();
            run.player().clearHeard();
            run.player().lookFrom(hit.add(Vec3.atLowerCornerOf(face.getNormal()).scale(1.5)), hit);
            hands.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, wire);
            var result = SensorWireItem.useOnBlock(hands, wire, new net.minecraft.world.phys.BlockHitResult(hit, face, at, false));
            hands.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, ItemStack.EMPTY);
            List<String> said = run.player().heard();
            if (!result.consumesAction())
                throw new IllegalStateException("clicking the wire on " + run.level().getBlockState(at) + " gave " + result + ": " + said);
            run.say("§7  sensor wire clicked on " + run.level().getBlockState(at).getBlock().getName().getString()
                    + (said.isEmpty() ? "" : ": " + said));
        }

        @Override
        public void clear(ScenarioRun run) {
            // Wires first, without drops: removing the lidar would otherwise pop their items.
            BlockPos lo = run.abs(new BlockPos(-10, -1, -2)), hi = run.abs(new BlockPos(10, 4, 15));
            for (var w : run.level().getEntitiesOfClass(BlockWireEntity.class, new AABB(Vec3.atLowerCornerOf(lo), Vec3.atLowerCornerOf(hi))))
                w.discard();
        }
    }
}
//?}
