package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.microwave.DishAim;
import com.example.evanscomputermod.radio.microwave.MicrowaveContent;
import com.example.evanscomputermod.radio.microwave.MicrowaveLink;
import com.example.evanscomputermod.radio.microwave.MicrowaveRadioBlockEntity;
import com.example.evanscomputermod.radio.microwave.MwBand;
import com.example.evanscomputermod.radio.microwave.dish.DishBlock;
import com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity;
import com.example.evanscomputermod.radio.microwave.dish.DishSize;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.phys.AABB;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.Arrays;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Phase 7 (microwave links) in namespace {@code ecm_radio}: real dish and
 * radio blocks bridging two cable segments, the multiblock's place/break
 * behaviour, rain fade from Minecraft weather, and the {@code microwave_link}
 * scenario.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class MicrowaveTests {

    private static final byte[] BCAST = {-1, -1, -1, -1, -1, -1};

    private static byte[] ethernet(byte[] dst, byte[] src, int tag) {
        byte[] f = new byte[60];
        System.arraycopy(dst, 0, f, 0, 6);
        System.arraycopy(src, 0, f, 6, 6);
        f[12] = 0x08;
        f[20] = (byte) tag;
        return f;
    }

    /** Drain a NIC's queue; return the first frame from {@code src}, or null. */
    private static byte[] receiveFrom(NetworkHub hub, byte[] nic, byte[] src) {
        byte[] f, found = null;
        while ((f = hub.receive(nic)) != null)
            if (found == null && Arrays.equals(Arrays.copyOfRange(f, 6, 12), src)) found = f;
        return found;
    }

    /**
     * Two radio + 1.2 m dish pairs 30 blocks apart, each on its own cable
     * segment with a NIC. Aimed at each other (24 GHz, ATPC at -40 dBm), a
     * broadcast from NIC A reaches NIC B and B's unicast reply reaches A.
     * Control: turn dish B 30 degrees and A's broadcasts no longer arrive.
     */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".microwave_bridge")
    public static void microwave_link_bridges_segments(GameTestHelper h) {
        var hub = NetworkHub.getInstance();
        var cables = CableNetworkManager.getInstance();
        var level = h.getLevel();
        byte[] macA = {0x02, 0x7e, 1, 1, 1, 1}, macB = {0x02, 0x7e, 2, 2, 2, 2};
        BlockPos radioA = new BlockPos(4, 2, 20), radioB = new BlockPos(34, 2, 20);
        BlockPos dishA = new BlockPos(4, 3, 20), dishB = new BlockPos(34, 3, 20);
        var cable = ModBlocks.NETWORK_CABLE.get().defaultBlockState();
        for (BlockPos p : List.of(new BlockPos(4, 1, 20), new BlockPos(4, 1, 21), new BlockPos(34, 1, 20), new BlockPos(34, 1, 21)))
            h.setBlock(p, cable);
        h.setBlock(radioA, MicrowaveContent.MICROWAVE_RADIO.get().defaultBlockState());
        h.setBlock(radioB, MicrowaveContent.MICROWAVE_RADIO.get().defaultBlockState());
        var dish = MicrowaveContent.dish(DishSize.MEDIUM);
        String[] failure = {null};
        if (!dish.place(level, h.absolutePos(dishA), Direction.EAST) || !dish.place(level, h.absolutePos(dishB), Direction.WEST))
            failure[0] = "dishes could not be placed";
        hub.registerNic(macA, (irq, p) -> {});
        hub.registerNic(macB, (irq, p) -> {});
        cables.registerTerminal(h.absolutePos(new BlockPos(4, 2, 21)), level.dimension(), new byte[][] {macA},
                new BlockPos[] {h.absolutePos(new BlockPos(4, 1, 21))});
        cables.registerTerminal(h.absolutePos(new BlockPos(34, 2, 21)), level.dimension(), new byte[][] {macB},
                new BlockPos[] {h.absolutePos(new BlockPos(34, 1, 21))});
        int[] step = {0}, waited = {0};
        long[] mark = {0, 0};
        TestDriver.drive(h, RadioTests.NS, "microwave_link_bridges_segments", () -> {
            var ra = (MicrowaveRadioBlockEntity) h.getBlockEntity(radioA);
            var rb = (MicrowaveRadioBlockEntity) h.getBlockEntity(radioB);
            var da = (DishBlockEntity) h.getBlockEntity(dishA);
            var db = (DishBlockEntity) h.getBlockEntity(dishB);
            switch (step[0]) {
                case 0 -> {   // radios join their cables and find their dishes on their own ticks
                    if (ra.cabled() && rb.cabled() && ra.link().ready() && rb.link().ready() && da.radio() != null) {
                        if (cables.areOnSameNetwork(macA, macB)) failure[0] = "segments A and B are already joined by cable";
                        Pose pa = da.worldPose(), pb = db.worldPose();
                        da.aimAt(pb.x(), pb.y(), pb.z());
                        db.aimAt(pa.x(), pa.y(), pa.z());
                        ra.link().setTxPowerDbm(-40);
                        rb.link().setTxPowerDbm(-40);
                        step[0] = 1;
                    } else if (++waited[0] > 100) {
                        failure[0] = "radios not ready: cabled " + ra.cabled() + "/" + rb.cabled() + " dish " + ra.link().ready() + "/" + rb.link().ready();
                    }
                }
                case 1 -> step[0] = 2;   // radios pick up the new aim
                case 2 -> {
                    receiveFrom(hub, macB, macA);
                    hub.transmit(macA, ethernet(BCAST, macA, 1));
                    byte[] got = receiveFrom(hub, macB, macA);
                    if (got == null) {
                        failure[0] = "broadcast from segment A did not reach segment B: " + rb.link().status();
                        return false;
                    }
                    hub.transmit(macB, ethernet(macA, macB, 2));
                    byte[] reply = receiveFrom(hub, macA, macB);
                    if (reply == null || reply[20] != 2) {
                        failure[0] = "unicast reply did not return to A: " + ra.link().status();
                        return false;
                    }
                    if (!Arrays.equals(hub.bridgePortOf(macA), rb.mac()))
                        failure[0] = "B's hub didn't learn A's MAC behind radio B";
                    db.nudge(30, 0);   // control: misaim B
                    mark[0] = ra.link().txFrames();
                    step[0] = 3;
                }
                case 3 -> {
                    receiveFrom(hub, macB, macA);
                    for (int i = 0; i < 5; i++) hub.transmit(macA, ethernet(BCAST, macA, 10 + i));
                    mark[1] = h.getTick();
                    step[0] = 4;
                }
                default -> {
                    if (h.getTick() - mark[1] < 10) return false;
                    if (receiveFrom(hub, macB, macA) != null) {
                        failure[0] = "control: B heard A with its dish 30 degrees off: " + rb.link().status();
                        return false;
                    }
                    if (ra.link().txFrames() - mark[0] < 5) {
                        failure[0] = "control invalid: radio A didn't transmit";
                        return false;
                    }
                    cables.unregisterTerminal(new byte[][] {macA, macB});
                    hub.unregisterNic(macA);
                    hub.unregisterNic(macB);
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    /** A 3×3 dish places all nine parts (or none when blocked) and breaking any part removes the dish with one drop. */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".microwave_multiblock")
    public static void dish_multiblock_place_and_break(GameTestHelper h) {
        var level = h.getLevel();
        var dish = MicrowaveContent.dish(DishSize.LARGE);
        BlockPos c = h.absolutePos(new BlockPos(10, 2, 10)), blocked = h.absolutePos(new BlockPos(20, 2, 10));
        String[] failure = {null};
        if (!dish.place(level, c, Direction.NORTH)) failure[0] = "large dish not placed";
        for (int part = 0; failure[0] == null && part < 9; part++) {
            var s = level.getBlockState(DishBlock.partPos(DishSize.LARGE, c, Direction.NORTH, part));
            if (!s.is(dish) || s.getValue(DishBlock.PART) != part) failure[0] = "part " + part + " is " + s;
        }
        if (failure[0] == null && !(level.getBlockEntity(c) instanceof DishBlockEntity))
            failure[0] = "controller has no block entity";
        // Control: an obstruction in the footprint stops placement entirely.
        level.setBlock(blocked.above(2).east(), net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(), 3);
        if (failure[0] == null && dish.place(level, blocked, Direction.NORTH)) failure[0] = "control: placed over stone";
        if (failure[0] == null && level.getBlockState(blocked).is(dish)) failure[0] = "control: partial dish left behind";
        BlockPos corner = DishBlock.partPos(DishSize.LARGE, c, Direction.NORTH, 8);
        if (failure[0] == null) level.destroyBlock(corner, true);
        TestDriver.drive(h, RadioTests.NS, "dish_multiblock_place_and_break", () -> {
            for (int part = 0; part < 9; part++) {
                if (level.getBlockState(DishBlock.partPos(DishSize.LARGE, c, Direction.NORTH, part)).is(dish)) {
                    failure[0] = "part " + part + " survived breaking a corner";
                    return false;
                }
            }
            List<ItemEntity> drops = level.getEntitiesOfClass(ItemEntity.class, new AABB(c).inflate(4));
            int count = drops.stream().filter(e -> e.getItem().is(MicrowaveContent.DISH_LARGE_ITEM.get())).mapToInt(e -> e.getItem().getCount()).sum();
            if (count != 1) {
                failure[0] = "expected one dish item, got " + count;
                return false;
            }
            drops.forEach(ItemEntity::discard);
            return true;
        }, () -> failure[0]);
    }

    /**
     * Rain fade from Minecraft weather on a 3 km, 60 GHz hop between two
     * radios with 1.2 m dishes (radio engines placed 3 km apart in this level,
     * since a structure can't hold the path): the link works under a clear
     * sky (oxygen ~37 dB), and a thunderstorm set with setWeatherParameters
     * (~50 dB more rain fade) kills it.
     */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".microwave_rain")
    public static void microwave_rain_fade_60ghz(GameTestHelper h) {
        var level = h.getLevel();
        BlockPos o = h.absolutePos(new BlockPos(20, 6, 20));
        BlockPos farPos = o.east(3000);
        String dim = level.dimension().location().toString();
        List<byte[]> atB = new CopyOnWriteArrayList<>();
        var a = new MicrowaveLink(UUID.randomUUID(), new byte[] {0x02, 0x7f, 0, 0, 0, 1}, RadioMediumHooks::medium, f -> {}, System::currentTimeMillis);
        var b = new MicrowaveLink(UUID.randomUUID(), new byte[] {0x02, 0x7f, 0, 0, 0, 2}, RadioMediumHooks::medium, atB::add, System::currentTimeMillis);
        double[] pa = {o.getX() + 0.5, o.getY() + 0.5, o.getZ() + 0.5}, pb = {farPos.getX() + 0.5, farPos.getY() + 0.5, farPos.getZ() + 0.5};
        for (var l : List.of(a, b)) l.configure(MwBand.GHZ_60, 1000, 0);
        a.setDish(aimed(dim, pa, pb), 1.2);
        b.setDish(aimed(dim, pb, pa), 1.2);
        level.setWeatherParameters(6000, 0, false, false);
        String[] failure = {null};
        int[] step = {0}, waited = {0}, before = {0};
        double[] clearSinr = {0};
        TestDriver.drive(h, RadioTests.NS, "microwave_rain_fade_60ghz", () -> {
            double rainA = MicrowaveRadioBlockEntity.rainRateAt(level, o), rainB = MicrowaveRadioBlockEntity.rainRateAt(level, farPos);
            a.setRainRate(rainA);
            b.setRainRate(rainB);
            a.tick();
            b.tick();
            switch (step[0]) {
                case 0 -> {   // clear sky (weather may still be fading out from an earlier test)
                    if (rainA > 0 || rainB > 0) {
                        if (++waited[0] > 300) failure[0] = "weather never cleared";
                        return false;
                    }
                    a.fromCable(ethernet(BCAST, new byte[] {0x02, 0x11, 0, 0, 0, 1}, 1));
                    if (atB.size() != 1) {
                        failure[0] = "clear-sky 3 km 60 GHz hop failed: " + b.status();
                        return false;
                    }
                    clearSinr[0] = b.lastSinrDb();
                    level.setWeatherParameters(0, 6000, true, true);
                    waited[0] = 0;
                    step[0] = 1;
                }
                case 1 -> {   // Minecraft fades rain and thunder in over ~100 ticks
                    if (rainA < 45 || rainB < 45) {
                        if (++waited[0] > 400) failure[0] = "thunderstorm never arrived: " + rainA + " / " + rainB + " mm/h";
                        return false;
                    }
                    before[0] = atB.size();
                    for (int i = 0; i < 10; i++) a.fromCable(ethernet(BCAST, new byte[] {0x02, 0x11, 0, 0, 0, 1}, 10 + i));
                    step[0] = 2;
                }
                default -> {
                    level.setWeatherParameters(6000, 0, false, false);
                    a.stop();
                    b.stop();
                    if (atB.size() != before[0]) {
                        failure[0] = "thunderstorm: " + (atB.size() - before[0]) + "/10 frames still crossed; clear SINR " + clearSinr[0];
                        return false;
                    }
                    if ((Long) b.status().get("faded_frames") < 10) {
                        failure[0] = "frames weren't lost to rain fade: " + b.status();
                        return false;
                    }
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    private static Pose aimed(String dim, double[] from, double[] to) {
        double[] aim = DishAim.aimForWorld(new double[] {to[0] - from[0], to[1] - from[1], to[2] - from[2]}, v -> v);
        float[] q = DishAim.worldQuaternion(aim[0], aim[1], v -> v);
        return new Pose(dim, from[0], from[1], from[2], q[0], q[1], q[2], q[3]);
    }

    /** Two wired LANs joined by a microwave link: ping across, misaimed control, align restores. */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".microwave_link")
    public static void microwave_link(GameTestHelper h) {
        TestDriver.scenario(h, RadioTests.NS, RadioScenarios.ALL.get("microwave_link"));
    }

    /**
     * 7D: a radio and a 1.2 m dish assembled into a Sable sub-level. The
     * multiblock survives assembly (all parts, no drop); the radio's endpoint
     * reports the dish's pose through the sub-level; turning the ship 30
     * degrees drags the beam off a far target (> 20 dB), and re-aiming with
     * aim_at holds it at peak gain.
     */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".microwave_sable")
    public static void microwave_dish_follows_sable_ship(GameTestHelper h) {
        var level = h.getLevel();
        BlockPos radioRel = new BlockPos(10, 3, 30), dishRel = new BlockPos(10, 4, 30);
        String[] failure = {null};
        if (!com.example.evanscomputermod.sable.SableCompat.isLoaded()) failure[0] = "Sable is not in this run";
        for (int x = 9; x <= 11; x++) for (int z = 29; z <= 32; z++)
            h.setBlock(new BlockPos(x, 2, z), net.minecraft.world.level.block.Blocks.IRON_BLOCK.defaultBlockState());
        h.setBlock(radioRel, MicrowaveContent.MICROWAVE_RADIO.get().defaultBlockState());
        var dish = MicrowaveContent.dish(DishSize.MEDIUM);
        if (failure[0] == null && !dish.place(level, h.absolutePos(dishRel), Direction.EAST)) failure[0] = "dish not placed";
        int[] step = {0}, waited = {0};
        UUID[] id = {null};
        Object[] sub = {null};
        TestDriver.drive(h, RadioTests.NS, "microwave_dish_follows_sable_ship", () -> {
            switch (step[0]) {
                case 0 -> {
                    var rbe = (MicrowaveRadioBlockEntity) h.getBlockEntity(radioRel);
                    if (!rbe.link().ready()) {
                        if (++waited[0] > 40) failure[0] = "radio never found its dish";
                        return false;
                    }
                    id[0] = rbe.link().id();
                    java.util.List<BlockPos> blocks = new java.util.ArrayList<>();
                    BlockPos min = h.absolutePos(new BlockPos(9, 2, 29)), max = h.absolutePos(new BlockPos(11, 5, 32));
                    for (BlockPos p : BlockPos.betweenClosed(min, max))
                        if (!level.getBlockState(p).isAir()) blocks.add(p.immutable());
                    sub[0] = dev.ryanhcode.sable.api.SubLevelAssemblyHelper.assembleBlocks(level, h.absolutePos(radioRel), blocks,
                            new dev.ryanhcode.sable.companion.math.BoundingBox3i(min, max));
                    if (level.getBlockState(h.absolutePos(dishRel)).is(dish)) failure[0] = "dish did not move";
                    waited[0] = 0;
                    step[0] = 1;
                }
                case 1 -> {   // the moved radio ticks in the plot with the same id
                    MicrowaveLink l = MicrowaveLink.find(id[0]);
                    if (l == null || !l.ready() || ++waited[0] < 3) {
                        if (waited[0] > 60) failure[0] = "moved radio never came back: " + (l == null ? "gone" : l.status());
                        return false;
                    }
                    var s = (dev.ryanhcode.sable.sublevel.ServerSubLevel) sub[0];
                    Pose lp = l.pose();
                    var c = s.logicalPose().transformPositionInverse(new org.joml.Vector3d(lp.x(), lp.y(), lp.z()));
                    BlockPos plot = BlockPos.containing(c.x, c.y, c.z);
                    DishBlockEntity d = DishBlock.controllerEntity(level, plot);
                    if (d == null) {
                        failure[0] = "no dish in the plot at " + plot;
                        return false;
                    }
                    var ds = level.getBlockState(d.getBlockPos());
                    if (!((DishBlock) ds.getBlock()).complete(level, ds, d.getBlockPos())) {
                        failure[0] = "dish lost parts in assembly";
                        return false;
                    }
                    int drops = level.getEntitiesOfClass(ItemEntity.class, new AABB(h.absolutePos(dishRel)).inflate(4)).size();
                    if (drops != 0) {
                        failure[0] = "assembly dropped " + drops + " items";
                        return false;
                    }
                    Pose before = d.worldPose();
                    double[] bore = before.toWorld(0, 0, 1);
                    if (DishAim.angleDeg(lp.toWorld(0, 0, 1), bore) > 3) {
                        failure[0] = "radio endpoint boresight differs from the dish's: " + DishAim.angleDeg(lp.toWorld(0, 0, 1), bore);
                        return false;
                    }
                    double[] target = {before.x() + 300 * bore[0], before.y() + 300 * bore[1], before.z() + 300 * bore[2]};
                    double g0 = gainTowards(l, before, target);
                    s.logicalPose().orientation().rotateY(Math.toRadians(30));
                    Pose turned = d.worldPose();
                    double g1 = gainTowards(l, turned, target);
                    d.aimAt(target[0], target[1], target[2]);
                    double g2 = gainTowards(l, d.worldPose(), target);
                    double peak = l.dish().peakGainDbi();
                    if (Math.abs(g0 - peak) > 0.5) failure[0] = "not on target before the turn: " + g0 + " vs " + peak;
                    else if (g0 - g1 < 20) failure[0] = "a 30 degree ship turn only cost " + (g0 - g1) + " dB";
                    else if (Math.abs(g2 - peak) > 0.5) failure[0] = "re-aim did not recover: " + g2 + " vs " + peak;
                    return failure[0] == null;
                }
                default -> {
                    return false;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    private static double gainTowards(MicrowaveLink l, Pose p, double[] target) {
        double dx = target[0] - p.x(), dy = target[1] - p.y(), dz = target[2] - p.z(), n = Math.sqrt(dx * dx + dy * dy + dz * dz);
        double[] loc = p.toLocal(dx / n, dy / n, dz / n);
        return l.dish().gainDbi(loc[0], loc[1], loc[2]);
    }

    private MicrowaveTests() {}
}
//?}
