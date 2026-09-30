package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.sensor.LidarSensorBlock;
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.wire.BlockWireEndpoint;
import com.example.evanscomputermod.sensor.wire.BlockWireEntity;
import com.example.evanscomputermod.sensor.wire.DeferredJunctionWireEndpoint;
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.JunctionWireEndpoint;
import com.example.evanscomputermod.sensor.wire.SensorWireItem;
import com.example.evanscomputermod.sensor.wire.WireConnections;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.AttachFace;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.BooleanSupplier;

/**
 * Lidar sensors, Sensor Wire and the Wired Sensor Module, namespace
 * {@code ecm_sensor} (1.21.1; needs Sable in the run for the move test, which
 * the runner provides).
 *
 * <p>Wires are placed through the Sensor Wire item's real click handling
 * (routing included), the module is installed through the computer's bay
 * clicks, and the lidar is read back through the peripheral hub as a program
 * would. No computer is booted: none of this needs a running program.
 */
@GameTestHolder(SensorTests.NS)
@PrefixGameTestTemplate(false)
public final class SensorTests {
    static final String NS = "ecm_sensor";
    private static final String STRUCTURE = "gametest_sensor";
    private static final long WALL_LIMIT_MS = 60_000;

    /** Computer (facing north, left bay on the west face), stone floor below y=2. */
    private static final BlockPos TERMINAL = new BlockPos(12, 2, 6);
    private static final BlockPos SENSOR = new BlockPos(8, 2, 6);
    private static final BlockPos SENSOR2 = new BlockPos(8, 2, 9);

    // ------------------------------------------------------------ tests

    /** The wire routes from the module's bay connector to the sensor, and the module finds and names it. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wire")
    public static void wire_links_sensor_to_module(GameTestHelper h) {
        Env e = new Env(h);
        e.run("wire_links_sensor_to_module", List.of(
                e::build,
                e::installModule,
                () -> {
                    check(e.state().getValue(TerminalBlock.SLOTS[0]) == ModuleSlotVisual.WIRED_SENSOR, "slot shows " + e.state().getValue(TerminalBlock.SLOTS[0]));
                    check(IWireHost.getAt(e.level(), e.abs(TERMINAL)).terminal(e.state(), 0) != null, "no bay connector");
                    e.wire(SENSOR);
                    BlockWireEntity wire = e.onlyWire();
                    check(wire.getEndpoint1() instanceof BlockWireEndpoint && wire.getEndpoint2() instanceof BlockWireEndpoint,
                            "wire ends: " + wire.getEndpoint1() + " / " + wire.getEndpoint2());
                    check(wire.segments.size() >= 2, "wire not routed along surfaces: " + wire.segments.size() + " segments");
                    check(!e.player.getMainHandItem().has(SensorContent.CONNECTION_DATA.get()), "placement still pending");
                    return true;
                },
                () -> e.names().contains("lidar_1"),
                () -> {
                    @SuppressWarnings("unchecked")
                    Map<Object, Object> mount = (Map<Object, Object>) e.call("mount", "lidar_1");
                    // Sensor head at (8.5, 2.22, 6.5); computer centre (12.5, 2.5, 6.5); computer x = north, y = west.
                    check(near(num(mount.get("x")), 0) && near(num(mount.get("y")), 4) && near(num(mount.get("z")), -0.28125),
                            "mount " + mount);
                    check(near(num(mount.get("yaw")), -90), "yaw " + mount.get("yaw"));   // sensor faces east = right of north
                    check("floor".equals(mount.get("mount")), "mount kind " + mount.get("mount"));
                    return true;
                }));
    }

    /** Ranges against a block and an entity, and points in the computer frame. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ranges")
    public static void lidar_ranges_and_points(GameTestHelper h) {
        Env e = new Env(h);
        e.run("lidar_ranges_and_points", List.of(
                e::build,
                e::installModule,
                () -> {
                    e.wire(SENSOR);
                    // An armor stand 2 blocks west of the sensor (in the 180 degree ray), on the floor.
                    var stand = EntityType.ARMOR_STAND.create(e.level());
                    Vec3 at = Vec3.atBottomCenterOf(e.abs(SENSOR.west(2)));
                    stand.moveTo(at.x, at.y, at.z, 0, 0);
                    e.level().addFreshEntity(stand);
                    return true;
                },
                () -> e.names().contains("lidar_1"),
                () -> {
                    e.call("configure", "lidar_1", Map.of("az_min", 0.0, "az_max", 180.0, "az_steps", 2, "range", 16.0));
                    e.seqWanted = ((Number) e.call("scan", "lidar_1")).intValue();
                    return true;
                },
                () -> ((Number) e.call("get_seq", "lidar_1")).intValue() >= e.seqWanted,
                () -> {
                    @SuppressWarnings("unchecked")
                    Map<Object, Object> scan = (Map<Object, Object>) e.call("get_scan", "lidar_1");
                    float[] r = floats((byte[]) scan.get("ranges"));
                    check(r.length == 2, "ranges " + r.length);
                    // East: the computer's west face at x = 12, from the head at x = 8.5.
                    check(Math.abs(r[0] - 3.5) < 0.01, "east range " + r[0]);
                    // West: the armor stand's box (half width 0.25) centred 2 blocks away.
                    check(Math.abs(r[1] - 1.75) < 0.01, "west range " + r[1]);
                    @SuppressWarnings("unchecked")
                    Map<Object, Object> pts = (Map<Object, Object>) e.call("get_points", "lidar_1");
                    float[] p = floats((byte[]) pts.get("points"));
                    check(p.length == 6, "points " + p.length);
                    // Hit on the computer's face (12, 2.22, 6.5): relative to its centre (-0.5, -0.28, 0) -> x 0, y 0.5.
                    check(near(p[0], 0) && near(p[1], 0.5) && near(p[2], -0.28125), "point " + p[0] + "," + p[1] + "," + p[2]);
                    return true;
                }));
    }

    /** A second sensor branches off the first wire through a junction; breaking a sensor detaches it. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".junction")
    public static void junction_bus_two_sensors(GameTestHelper h) {
        Env e = new Env(h);
        e.run("junction_bus_two_sensors", List.of(
                e::build,
                e::installModule,
                () -> {
                    e.wire(SENSOR);
                    BlockWireEntity trunk = e.onlyWire();
                    // Branch off the middle of the trunk's longest segment.
                    int index = 0;
                    for(int i = 1; i < trunk.segments.size(); i++)
                        if(trunk.segments.get(i).gridLength > trunk.segments.get(index).gridLength) index = i;
                    var mid = new DeferredJunctionWireEndpoint(trunk, index, trunk.segments.get(index).gridLength / 2);
                    var stack = e.player.getMainHandItem();
                    var result = SensorWireItem.connect(e.level(), stack, e.player, e.sensorEndpoint(SENSOR2), mid);
                    check(result.getResult().consumesAction(), "branch not placed");
                    return true;
                },
                () -> e.names().size() == 2,
                () -> {
                    check(new java.util.HashSet<>(e.names()).equals(java.util.Set.of("lidar_1", "lidar_2")), "names " + e.names());
                    check(e.wires().size() == 3, "expected trunk split in two plus the branch, got " + e.wires().size());
                    long junctionEnds = e.wires().stream().filter(w -> w.getEndpoint1() instanceof JunctionWireEndpoint
                            || w.getEndpoint2() instanceof JunctionWireEndpoint).count();
                    check(junctionEnds == 3, "wires at the junction: " + junctionEnds);
                    h.setBlock(SENSOR2, Blocks.AIR.defaultBlockState());
                    return true;
                },
                () -> e.names().size() == 1,
                () -> {
                    @SuppressWarnings("unchecked")
                    Map<Object, Object> mount = (Map<Object, Object>) e.call("mount", e.names().get(0));
                    check(near(num(mount.get("x")), 0) && near(num(mount.get("y")), 4), "wrong sensor left: " + mount);
                    check(WireConnections.get(e.level(), new BlockWireEndpoint(e.abs(SENSOR2), 0)).isEmpty(), "wire still on the broken sensor");
                    return true;
                }));
    }

    /**
     * Sable assembles the computer, sensor, floor and wire into a structure:
     * the wire moves into the structure with its ends re-pointed, and the
     * module finds the same sensor under the same name and mount.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".sable")
    public static void wire_moves_with_sable_structure(GameTestHelper h) {
        Env e = new Env(h);
        e.run("wire_moves_with_sable_structure", List.of(
                e::build,
                e::installModule,
                () -> {
                    e.wire(SENSOR);
                    e.wireId = e.onlyWire().getUUID();
                    return true;
                },
                () -> e.names().contains("lidar_1"),
                () -> {
                    List<BlockPos> blocks = new ArrayList<>();
                    BlockPos min = e.abs(new BlockPos(6, 1, 4)), max = e.abs(new BlockPos(13, 2, 8));
                    for(BlockPos p : BlockPos.betweenClosed(min, max))
                        if(!e.level().getBlockState(p).isAir()) blocks.add(p.immutable());
                    dev.ryanhcode.sable.api.SubLevelAssemblyHelper.assembleBlocks(e.level(), e.abs(TERMINAL), blocks,
                            new dev.ryanhcode.sable.companion.math.BoundingBox3i(min, max));
                    check(!(e.level().getBlockState(e.abs(TERMINAL)).getBlock() instanceof TerminalBlock), "computer did not move");
                    return true;
                },
                () -> {
                    // Follow the wire to the computer's new (plot) position.
                    if(!(e.level().getEntity(e.wireId) instanceof BlockWireEntity wire)) return false;
                    if(!(wire.getEndpoint1() instanceof BlockWireEndpoint a) || !(wire.getEndpoint2() instanceof BlockWireEndpoint b))
                        return false;
                    BlockPos computer = e.level().getBlockState(a.getPos()).getBlock() instanceof TerminalBlock ? a.getPos() : b.getPos();
                    if(!(e.level().getBlockEntity(computer) instanceof TerminalBlockEntity te)) return false;
                    e.movedTerminal = te;
                    return te.getPeripheralHub().names().contains("left_bay_1") && e.names().contains("lidar_1");
                },
                () -> {
                    @SuppressWarnings("unchecked")
                    Map<Object, Object> mount = (Map<Object, Object>) e.call("mount", "lidar_1");
                    check(near(num(mount.get("x")), 0) && near(num(mount.get("y")), 4), "mount after move " + mount);
                    // Nothing may be left in the world where the vehicle was built. (Sable's entity
                    // queries also return the structure's own entities, which live in its plot.)
                    var box = new AABB(Vec3.atLowerCornerOf(e.abs(new BlockPos(6, 1, 4))), Vec3.atLowerCornerOf(e.abs(new BlockPos(14, 4, 9))));
                    var left = e.level().getEntitiesOfClass(BlockWireEntity.class, box,
                            w -> !w.isRemoved() && !com.example.evanscomputermod.sensor.SensorSable.inPlot(e.level(), w.position()));
                    check(left.isEmpty(), "wires left behind: " + left.stream().map(w -> w.getUUID() + "@" + w.position()).toList());
                    check(com.example.evanscomputermod.sensor.SensorSable.inPlot(e.level(), e.level().getEntity(e.wireId).position()),
                            "the wire is not on the structure");
                    // Scan from the structure: straight ahead is the computer, on the same vehicle.
                    e.call("configure", "lidar_1", Map.of("az_min", 0.0, "az_max", 0.0, "az_steps", 1, "range", 16.0));
                    e.seqWanted = ((Number) e.call("scan", "lidar_1")).intValue();
                    return true;
                },
                () -> ((Number) e.call("get_seq", "lidar_1")).intValue() >= e.seqWanted,
                () -> {
                    @SuppressWarnings("unchecked")
                    Map<Object, Object> scan = (Map<Object, Object>) e.call("get_scan", "lidar_1");
                    float[] r = floats((byte[]) scan.get("ranges"));
                    check(r.length == 1 && Math.abs(r[0] - 3.5) < 0.02, "range to own vehicle " + java.util.Arrays.toString(r));
                    return true;
                }));
    }

    /**
     * End to end on a booted computer: a Python program uses the sensors
     * module to find the lidar, read its mount, take a scan and get points.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".python")
    public static void python_sensors_module(GameTestHelper h) {
        Env e = new Env(h);
        String script = String.join("\n",
                "import sensors",
                "hub = sensors.find()",
                "print('HUB', hub.names())",
                "lidar = hub.lidar('lidar_1')",
                "m = lidar.mount()",
                "print('MOUNT', round(m['x'], 2) + 0.0, round(m['y'], 2) + 0.0, m['mount'])",
                "lidar.configure(az_min=0, az_max=180, az_steps=2, range=4)",
                "scan = lidar.scan(timeout=20)",
                "print('SCAN', scan.seq, scan.columns, round(scan.ranges[0], 2), scan.azimuth(1))",
                "pts = lidar.points()",
                "print('POINTS', len(pts), round(pts[0][1], 2))",
                "try:",
                "    lidar.configure(range=500)",
                "except Exception as err:",
                "    print('ERR', err)",
                "print('DONE')",
                "");
        e.run("python_sensors_module", List.of(
                e::build,
                e::installModule,
                () -> {
                    e.wire(SENSOR);
                    e.terminal().initializeWasm();
                    return true;
                },
                () -> e.names().contains("lidar_1") && e.screen().contains("Welcome to Terminal OS"),
                () -> {
                    Path dir = Path.of("computer-data", e.terminal().getComputerId().toString());
                    try {
                        Files.createDirectories(dir);
                        Files.writeString(dir.resolve("lidar_test.py"), script);
                    } catch(java.io.IOException ex) {
                        throw new AssertionError("can't write the script: " + ex);
                    }
                    e.terminal().onStringInput("python lidar_test.py\n");
                    return true;
                },
                () -> e.screen().contains("DONE") || e.screen().contains("Traceback"),
                () -> {
                    String s = e.screen();
                    check(s.contains("HUB ['lidar_1']"), "find/names failed:\n" + s);
                    check(s.contains("MOUNT 0.0 4.0 floor"), "mount failed:\n" + s);
                    check(s.contains("SCAN 1 2 3.5 180.0"), "scan failed:\n" + s);
                    check(s.contains("POINTS 1 0.5"), "points failed:\n" + s);
                    check(s.contains("ERR range must be in (0, 64]"), "no error for a bad range:\n" + s);
                    return true;
                }));
    }

    // ------------------------------------------------------------ plumbing

    private static void check(boolean ok, String message) {
        if(!ok) throw new AssertionError(message);
    }

    private static boolean near(double a, double b) {
        return Math.abs(a - b) < 0.01;
    }

    private static double num(Object o) {
        return ((Number) o).doubleValue();
    }

    private static float[] floats(byte[] b) {
        var buf = ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN);
        float[] f = new float[b.length / 4];
        for(int i = 0; i < f.length; i++) f[i] = buf.getFloat();
        return f;
    }

    private static final class Env {
        final GameTestHelper h;
        final Player player;
        private int step;
        private long started;
        private String failure;
        int seqWanted;
        UUID wireId;
        TerminalBlockEntity movedTerminal;

        Env(GameTestHelper h) {
            this.h = h;
            this.player = h.makeMockPlayer(GameType.CREATIVE);
        }

        void run(String name, List<BooleanSupplier> steps) {
            TestDriver.drive(h, NS, name, () -> {
                if(started == 0) started = System.currentTimeMillis();
                if(System.currentTimeMillis() - started > WALL_LIMIT_MS) {
                    failure = "timed out at step " + (step + 1) + "/" + steps.size();
                    return false;
                }
                try {
                    while(step < steps.size() && steps.get(step).getAsBoolean()) step++;
                } catch(AssertionError | RuntimeException ex) {
                    failure = "step " + (step + 1) + ": " + ex;
                    return false;
                }
                return step >= steps.size();
            }, () -> failure);
        }

        ServerLevel level() {
            return h.getLevel();
        }

        BlockPos abs(BlockPos rel) {
            return h.absolutePos(rel);
        }

        BlockState state() {
            return h.getBlockState(TERMINAL);
        }

        TerminalBlockEntity terminal() {
            return movedTerminal != null ? movedTerminal : (TerminalBlockEntity) h.getBlockEntity(TERMINAL);
        }

        /** Stone floor, the computer, and two lidars facing east on the floor. */
        boolean build() {
            for(int x = 6; x <= 13; x++)
                for(int z = 4; z <= 10; z++)
                    h.setBlock(new BlockPos(x, 1, z), Blocks.STONE.defaultBlockState());
            h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.NORTH));
            BlockState lidar = SensorContent.LIDAR_SENSOR.get().defaultBlockState()
                    .setValue(LidarSensorBlock.FACE, AttachFace.FLOOR).setValue(LidarSensorBlock.FACING, Direction.EAST);
            h.setBlock(SENSOR, lidar);
            h.setBlock(SENSOR2, lidar);
            return true;
        }

        /** Expansion card and a Wired Sensor Module in left_bay_1, through the bay clicks. */
        boolean installModule() {
            if(terminal().getModuleBays().cardCount() == 0) {
                hold(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get()));
                useTerminal(Direction.WEST, 0.5);
                hold(new ItemStack(SensorContent.WIRED_SENSOR_MODULE.get()));
                useTerminal(Direction.WEST, 0.75);
            }
            return terminal().getPeripheralHub().names().contains("left_bay_1");
        }

        void hold(ItemStack stack) {
            player.setItemInHand(InteractionHand.MAIN_HAND, stack);
        }

        void useTerminal(Direction face, double y) {
            BlockPos p = abs(TERMINAL);
            Vec3 c = Vec3.atCenterOf(p);
            var hit = new BlockHitResult(new Vec3(c.x + face.getStepX() * 0.5, p.getY() + y, c.z + face.getStepZ() * 0.5), face, p, false);
            ItemStack held = player.getMainHandItem();
            var result = state().useItemOn(held, level(), player, InteractionHand.MAIN_HAND, hit);
            if(result == net.minecraft.world.ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION)
                state().useWithoutItem(level(), player, hit);
        }

        /** Click a wire host's connector {@code index} with the held Sensor Wire. */
        void clickConnector(BlockPos absPos, int index, Direction face) {
            var host = IWireHost.getAt(level(), absPos);
            check(host != null, "no wire host at " + absPos);
            var terminal = host.terminal(level().getBlockState(absPos), index);
            check(terminal != null, "no connector " + index + " at " + absPos);
            Vec3 o = terminal.getOrigin();
            // A point just inside the connector's box, on the face the player would click.
            Vec3 at = new Vec3(absPos.getX() + o.x - face.getStepX() * 0.02, absPos.getY() + o.y - face.getStepY() * 0.02,
                    absPos.getZ() + o.z - face.getStepZ() * 0.02);
            var r = SensorWireItem.useOnBlock(player, player.getMainHandItem(), new BlockHitResult(at, face, absPos, false));
            check(r.consumesAction(), "click on connector " + index + " at " + absPos + " gave " + r);
        }

        BlockWireEndpoint sensorEndpoint(BlockPos rel) {
            return new BlockWireEndpoint(abs(rel), 0);
        }

        /** Sensor Wire from the module's connector (left_bay_1) to the sensor at {@code sensor}. */
        void wire(BlockPos sensor) {
            hold(new ItemStack(SensorContent.SENSOR_WIRE.get(), 64));
            clickConnector(abs(TERMINAL), 0, Direction.WEST);
            clickConnector(abs(sensor), 0, Direction.UP);
        }

        String screen() {
            return com.example.evanscomputermod.testing.scenario.ScenarioRun.screen(terminal().getDisplay());
        }

        List<BlockWireEntity> wires() {
            // Only this test's build area: neighbouring tests are a few blocks away.
            var box = new AABB(Vec3.atLowerCornerOf(abs(new BlockPos(5, 0, 3))), Vec3.atLowerCornerOf(abs(new BlockPos(15, 5, 12))));
            return level().getEntitiesOfClass(BlockWireEntity.class, box, w -> !w.isRemoved());
        }

        BlockWireEntity onlyWire() {
            var all = wires();
            check(all.size() == 1, "expected one wire, found " + all.size());
            return all.get(0);
        }

        Object call(String method, Object... args) {
            byte[] frame = terminal().getPeripheralHub().call("left_bay_1", method, PeripheralValues.encode(List.of(args)), level().getServer());
            try {
                Object v = PeripheralValues.decode(java.util.Arrays.copyOfRange(frame, 1, frame.length));
                if(frame[0] != PeripheralValues.STATUS_OK) throw new AssertionError(method + " failed: " + v);
                return v;
            } catch(PeripheralValues.DecodeException ex) {
                throw new AssertionError("bad frame from " + method + ": " + ex.getMessage());
            }
        }

        @SuppressWarnings("unchecked")
        List<String> names() {
            return (List<String>) call("names");
        }
    }

    private SensorTests() {
    }
}
//?}
