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
 * lidar on the floor in the middle, wired to the module. The script runs
 * {@code lidar_view.py}, which draws a top-down outline of what the lidar
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
        return Scenario.builder("lidar_room", "a lidar in a room, drawn as a live top-down outline on the computer")
                .host("pc", PC, "-")
                .decor(new LidarRoom())
                .note("Run the lidar viewer (" + PROGRAM + " is on the computer's disk)")
                .send("pc", "python " + PROGRAM)
                .expect("pc", "^LIDAR lidar_1 +scan \\d+", "the map is drawing")
                .waitMs(3_000, "let it draw a few scans")
                .timeLimit(90_000)
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

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            for (BlockPos p : floor) level.setBlock(run.abs(p), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
            for (BlockPos p : walls) level.setBlock(run.abs(p), Blocks.STONE_BRICKS.defaultBlockState(), 3);
            for (BlockPos p : obstacles) {
                BlockState st = p.getY() == 1 && p.getZ() == 4 && p.getX() > 0 ? Blocks.BARREL.defaultBlockState()
                        : Blocks.POLISHED_ANDESITE.defaultBlockState();
                level.setBlock(run.abs(p), st, 3);
            }
            level.setBlock(run.abs(LIDAR), SensorContent.LIDAR_SENSOR.get().defaultBlockState()
                    .setValue(LidarSensorBlock.FACE, AttachFace.FLOOR)
                    .setValue(LidarSensorBlock.FACING, Direction.NORTH), 3);

            TerminalBlockEntity pc = run.terminal("pc");
            if (pc == null) throw new IllegalStateException("no computer at " + run.abs(PC));
            ModuleBays bays = pc.getModuleBays();
            if (bays.cardCount() == 0) bays.installCard(ModuleBays.LEFT);
            if (bays.getStack(0).isEmpty()) bays.installModule(new ItemStack(SensorContent.WIRED_SENSOR_MODULE.get()), 0);

            wire(run);
            installProgram(pc);
        }

        /**
         * Sensor Wire from the module's connector (left_bay_1, on the computer's west face) down to
         * the floor and along it to the lidar, routed hop by hop like a player clicking block faces.
         */
        private void wire(ScenarioRun run) {
            var level = run.level();
            ItemStack wire = new ItemStack(SensorContent.SENSOR_WIRE.get(), 64);
            double floorTop = run.abs(BlockPos.ZERO).getY() + 1.0;
            Vec3 o = Vec3.atLowerCornerOf(run.abs(BlockPos.ZERO));
            List<IWireEndpoint> hops = List.of(
                    new ImaginaryWireEndpoint(new Vec3(o.x - 0.3, floorTop + 1 / 32.0, o.z + 0.7)),
                    new ImaginaryWireEndpoint(new Vec3(o.x - 0.3, floorTop + 1 / 32.0, o.z + 4.0)),
                    new BlockWireEndpoint(run.abs(LIDAR), 0));
            IWireEndpoint from = new BlockWireEndpoint(run.abs(PC), 0);
            for (IWireEndpoint to : hops) {
                var result = SensorWireItem.connect(level, wire, null, from, to);
                BlockWireEntity placed = result.getObject();
                if (!result.getResult().consumesAction() || placed == null)
                    throw new IllegalStateException("couldn't route the sensor wire to " + to.getExactPosition(level));
                from = new BlockWireEntityEndpoint(placed, true);
            }
        }

        private void installProgram(TerminalBlockEntity pc) {
            try (InputStream in = SensorScenarios.class.getResourceAsStream("/evanscomputermod/scenarios/" + PROGRAM)) {
                if (in == null) throw new IllegalStateException(PROGRAM + " is missing from the mod jar");
                Path dir = Path.of("computer-data", pc.getComputerId().toString());
                Files.createDirectories(dir);
                Files.writeString(dir.resolve(PROGRAM), new String(in.readAllBytes(), StandardCharsets.UTF_8));
            } catch (IOException e) {
                EvansComputerMod.LOGGER.error("Couldn't put {} on the scenario computer", PROGRAM, e);
            }
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
