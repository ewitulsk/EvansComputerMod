package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.radio.amp.AmplifierBlock;
import com.example.evanscomputermod.radio.amp.ExciterLink;
import com.example.evanscomputermod.radio.amp.RadioAmpContent;
import com.example.evanscomputermod.radio.amp.TunerBlock;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.cellular.CellAntennaBlock;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.ConductorBlockEntity;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.microwave.DishAim;
import com.example.evanscomputermod.radio.microwave.MicrowaveContent;
import com.example.evanscomputermod.radio.microwave.MicrowaveRadioBlock;
import com.example.evanscomputermod.radio.microwave.MicrowaveRadioBlockEntity;
import com.example.evanscomputermod.radio.microwave.dish.DishBlock;
import com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity;
import com.example.evanscomputermod.radio.power.BurnerGeneratorBlock;
import com.example.evanscomputermod.radio.power.RadioPowerContent;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.radio.sdr.SdrBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlock;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.sensor.LidarSensorBlock;
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.LevelReader;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.PipeBlock;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.AttachFace;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructurePlaceSettings;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureProcessor;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureProcessorType;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * ECM blocks turn with structures (namespace {@code ecm_radio}). A 7x6x7 cell holding every
 * block with a facing, axis or arm property (SDR, amplifier, tuner, arrestor and coax up to a
 * dipole's feed point; a microwave radio feeding a 3x3 dish; a 2x2 dish; access point,
 * interface block, network cable, terminal, card reader, charging dock, burner generator,
 * speaker, LTE antenna, lidar, fiber patch panel; a copper wire with a wrench-cut face) is
 * saved as a structure template and placed with every rotation, and every rotation under each
 * mirror, as villages and structure blocks place templates. Each copy is checked against
 * positions, not block states: a block's facing (or arm, or axis) must point at the copy of
 * the block it pointed at in the source; each dish is whole; the 3x3 dish's aim (in its block
 * entity) turned with it; the cut face still faces the same neighbour. Functional: in every
 * copy the SDR's exciter link runs through the amplifier, the tuner and the arrestor to the
 * copy's own feed point and finds the antenna, and the microwave radio finds its dish. Control:
 * the source cell itself passes the same checks.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioStructureRotationTests {
    private static final String NS = RadioTests.NS;
    private static final Vec3i SIZE = new Vec3i(7, 6, 7);
    /** Cell-local positions. */
    private static final BlockPos FEED = new BlockPos(3, 4, 1), SDR = new BlockPos(3, 0, 4),
            RADIO = new BlockPos(1, 0, 1), BIG_DISH = new BlockPos(0, 0, 1), SMALL_DISH = new BlockPos(5, 0, 5),
            CUT_WIRE = new BlockPos(5, 2, 4);
    private static final double AIM_YAW = Direction.WEST.toYRot() + 25, AIM_ELEV = 10;

    private RadioStructureRotationTests() {}

    private record Copy(String name, StructurePlaceSettings settings, BlockPos origin) {
        /** World position of the copy of cell-local {@code rel}. */
        BlockPos at(BlockPos rel) {
            return StructureTemplate.calculateRelativePosition(settings, rel).offset(origin);
        }

        /** The direction in this copy that cell-local direction {@code d} at {@code rel} became. */
        Direction turn(BlockPos rel, Direction d) {
            BlockPos from = at(rel), to = at(rel.relative(d));
            for (Direction e : Direction.values()) if (from.relative(e).equals(to)) return e;
            throw new IllegalStateException("no direction from " + from + " to " + to);
        }
    }

    /** Cell {@code i}'s minimum corner in the 40x40 test floor (4 per row, 9 apart). */
    private static BlockPos cell(GameTestHelper h, int i) {
        return h.absolutePos(new BlockPos(1 + (i % 4) * 9, 2, 1 + (i / 4) * 9));
    }

    private static void put(ServerLevel level, BlockPos o, BlockPos rel, BlockState s) {
        level.setBlock(o.offset(rel), s, Block.UPDATE_ALL);
    }

    private static void wire(ServerLevel level, BlockPos o, BlockPos rel, BlockState s) {
        ConductorBlock.placeConnected(level, o.offset(rel), s);
    }

    private static void build(ServerLevel level, BlockPos o) {
        var coax = RadioAntennaContent.COAX_CABLE.get().defaultBlockState();
        var copper = RadioAntennaContent.COPPER_WIRE.get().defaultBlockState();
        // Dipole: feed point (axis x) with two copper blocks and an insulator each side.
        wire(level, o, new BlockPos(0, 4, 1), RadioAntennaContent.INSULATOR.get().defaultBlockState());
        wire(level, o, new BlockPos(6, 4, 1), RadioAntennaContent.INSULATOR.get().defaultBlockState());
        wire(level, o, FEED, RadioAntennaContent.FEED_POINT.get().defaultBlockState().setValue(FeedPointBlock.AXIS, Direction.Axis.X));
        for (int x : new int[] {1, 2, 4, 5}) wire(level, o, new BlockPos(x, 4, 1), copper);
        // Down to the tuner (ANT on its top), arrestor on the way.
        wire(level, o, new BlockPos(3, 3, 1), coax);
        wire(level, o, new BlockPos(3, 2, 1), RadioAntennaContent.LIGHTNING_ARRESTOR.get().defaultBlockState());
        wire(level, o, new BlockPos(3, 1, 1), coax);
        // Facing north: the tuner's TX jack and the amplifier's RF IN (their backs) face the SDR side.
        put(level, o, new BlockPos(3, 0, 1), RadioAmpContent.ANTENNA_TUNER.get().defaultBlockState().setValue(TunerBlock.FACING, Direction.NORTH));
        wire(level, o, new BlockPos(3, 0, 2), coax);
        put(level, o, new BlockPos(3, 0, 3), RadioAmpContent.AMPLIFIER_100W.get().defaultBlockState().setValue(AmplifierBlock.FACING, Direction.NORTH));
        put(level, o, SDR, RadioSdrContent.SDR_ADVANCED.get().defaultBlockState().setValue(SdrBlock.FACING, Direction.SOUTH));
        // Microwave radio facing east, its waveguide (back) into the 3x3 dish's mount, the dish facing away (west).
        MicrowaveContent.DISH_LARGE.get().place(level, o.offset(BIG_DISH), Direction.WEST);
        ((DishBlockEntity) level.getBlockEntity(o.offset(BIG_DISH))).setAim(AIM_YAW, AIM_ELEV);
        put(level, o, RADIO, MicrowaveContent.MICROWAVE_RADIO.get().defaultBlockState().setValue(MicrowaveRadioBlock.FACING, Direction.EAST));
        MicrowaveContent.DISH_MEDIUM.get().place(level, o.offset(SMALL_DISH), Direction.NORTH);
        // Interface block between an access point and a network cable; a terminal on the cable.
        put(level, o, new BlockPos(0, 0, 5), ModBlocks.INTERFACE_BLOCK.get().defaultBlockState());
        put(level, o, new BlockPos(0, 0, 4), AccessPointContent.ACCESS_POINT.get().defaultBlockState().setValue(AccessPointBlock.FACING, Direction.EAST));
        put(level, o, new BlockPos(0, 0, 6), ModBlocks.NETWORK_CABLE.get().defaultBlockState());
        put(level, o, new BlockPos(1, 0, 6), ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                .setValue(HorizontalDirectionalBlock.FACING, Direction.SOUTH));
        put(level, o, new BlockPos(2, 0, 6), CardContent.CARD_READER.get().defaultBlockState()
                .setValue(BlockStateProperties.HORIZONTAL_FACING, Direction.EAST));
        put(level, o, new BlockPos(3, 0, 6), PhoneContent.CHARGING_DOCK.get().defaultBlockState()
                .setValue(BlockStateProperties.HORIZONTAL_FACING, Direction.NORTH));
        put(level, o, new BlockPos(4, 0, 6), RadioPowerContent.BURNER_GENERATOR.get().defaultBlockState()
                .setValue(BurnerGeneratorBlock.FACING, Direction.WEST));
        put(level, o, new BlockPos(6, 0, 3), SpeakerContent.SPEAKER.get().defaultBlockState()
                .setValue(HorizontalDirectionalBlock.FACING, Direction.EAST));
        put(level, o, new BlockPos(6, 0, 1), CellularContent.LTE_ANTENNA.get().defaultBlockState()
                .setValue(CellAntennaBlock.FACING, Direction.SOUTH));
        put(level, o, new BlockPos(5, 0, 3), net.minecraft.world.level.block.Blocks.IRON_BLOCK.defaultBlockState());   // the lidar's floor
        put(level, o, new BlockPos(5, 1, 3), SensorContent.LIDAR_SENSOR.get().defaultBlockState()
                .setValue(LidarSensorBlock.FACE, AttachFace.FLOOR).setValue(LidarSensorBlock.FACING, Direction.EAST));
        put(level, o, new BlockPos(6, 0, 6), ModBlocks.FIBER_PATCH_PANEL.get().defaultBlockState()
                .setValue(BlockStateProperties.HORIZONTAL_FACING, Direction.SOUTH));
        // A copper pair with the wrench cut on the first one's east face (toward the second).
        wire(level, o, CUT_WIRE, copper);
        wire(level, o, CUT_WIRE.east(), copper);
        RadioAntennaContent.COPPER_WIRE.get().toggleCut(level, o.offset(CUT_WIRE), Direction.EAST);   // as the wrench does
    }

    /** Copies' identities (access point, microwave radio, computer) are left for the block entities to mint, as village processors do. */
    private static final StructureProcessor FRESH_IDS = new StructureProcessor() {
        @Override
        public @Nullable StructureTemplate.StructureBlockInfo processBlock(LevelReader level, BlockPos offset, BlockPos pos,
                StructureTemplate.StructureBlockInfo raw, StructureTemplate.StructureBlockInfo info, StructurePlaceSettings settings) {
            if (info.nbt() == null) return info;
            CompoundTag nbt = info.nbt().copy();
            nbt.remove("ApId");
            nbt.remove("Id");
            nbt.remove("computerId");
            return new StructureTemplate.StructureBlockInfo(info.pos(), info.state(), nbt);
        }

        @Override
        protected StructureProcessorType<?> getType() {
            return StructureProcessorType.NOP;
        }
    };

    private static boolean ours(BlockState s) {
        return BuiltInRegistries.BLOCK.getKey(s.getBlock()).getNamespace().equals(EvansComputerMod.MODID);
    }

    /** Facing, axis, arms and dishes of one copy against the source; returns the number of blocks checked. */
    private static int checkShapes(ServerLevel level, BlockPos src, Copy c, String[] failure) {
        int checked = 0;
        for (BlockPos rel : BlockPos.betweenClosed(BlockPos.ZERO, new BlockPos(SIZE.getX() - 1, SIZE.getY() - 1, SIZE.getZ() - 1))) {
            BlockState a = level.getBlockState(src.offset(rel));
            if (!ours(a)) continue;
            BlockPos rel0 = rel.immutable();
            BlockState b = level.getBlockState(c.at(rel0));
            String where = c.name + " " + BuiltInRegistries.BLOCK.getKey(a.getBlock()).getPath() + " at " + rel0;
            if (a.getBlock() != b.getBlock()) {
                failure[0] = where + ": copy holds " + b;
                return checked;
            }
            checked++;
            for (Property<?> p : a.getProperties()) {
                if (p.getValueClass() == Direction.class) {
                    @SuppressWarnings("unchecked") Property<Direction> dp = (Property<Direction>) p;
                    Direction want = c.turn(rel0, a.getValue(dp));
                    if (b.getValue(dp) != want) {
                        failure[0] = where + ": " + p.getName() + " " + b.getValue(dp) + ", want " + want + " (source " + a.getValue(dp) + ")";
                        return checked;
                    }
                } else if (p == BlockStateProperties.AXIS) {
                    Direction want = c.turn(rel0, Direction.fromAxisAndDirection(a.getValue(BlockStateProperties.AXIS), Direction.AxisDirection.POSITIVE));
                    if (b.getValue(BlockStateProperties.AXIS) != want.getAxis()) {
                        failure[0] = where + ": axis " + b.getValue(BlockStateProperties.AXIS) + ", want " + want.getAxis();
                        return checked;
                    }
                }
            }
            for (Map.Entry<Direction, BooleanProperty> arm : PipeBlock.PROPERTY_BY_DIRECTION.entrySet()) {
                if (!a.hasProperty(arm.getValue())) continue;
                Direction to = c.turn(rel0, arm.getKey());
                if (a.getValue(arm.getValue()) != b.getValue(PipeBlock.PROPERTY_BY_DIRECTION.get(to))) {
                    failure[0] = where + ": arm " + arm.getKey() + "=" + a.getValue(arm.getValue()) + " became " + to + "=" + b.getValue(PipeBlock.PROPERTY_BY_DIRECTION.get(to));
                    return checked;
                }
            }
            if (b.getBlock() instanceof DishBlock dish && !dish.complete(level, b, c.at(rel0))) {
                failure[0] = where + ": the dish is not whole (" + b + ")";
                return checked;
            }
        }
        return checked;
    }

    /** Block-entity directions and the working chains of one copy; null when all hold, else what doesn't (yet). */
    private static @Nullable String checkWorking(ServerLevel level, Copy c) {
        // The 3x3 dish's aim turned with it.
        if (!(level.getBlockEntity(c.at(BIG_DISH)) instanceof DishBlockEntity dish)) return c.name + ": no 3x3 dish controller";
        double[] v = DishAim.direction(AIM_YAW, AIM_ELEV);
        // The template's point transform, as a vector: T(v) - T(0) (it shifts whole blocks into place).
        Vec3 want = StructureTemplate.transform(new Vec3(v[0], v[1], v[2]), c.settings.getMirror(), c.settings.getRotation(), BlockPos.ZERO)
                .subtract(StructureTemplate.transform(Vec3.ZERO, c.settings.getMirror(), c.settings.getRotation(), BlockPos.ZERO));
        double[] got = DishAim.direction(dish.yaw(), dish.elevation());
        if (Math.abs(got[0] - want.x) > 1e-6 || Math.abs(got[1] - want.y) > 1e-6 || Math.abs(got[2] - want.z) > 1e-6)
            return c.name + ": dish aim yaw " + dish.yaw() + " elev " + dish.elevation() + " points " + List.of(got[0], got[1], got[2]) + ", want " + want;
        if (!(level.getBlockEntity(c.at(SMALL_DISH.east())) instanceof DishBlockEntity) && !(level.getBlockEntity(c.at(SMALL_DISH)) instanceof DishBlockEntity))
            return c.name + ": the 2x2 dish lost its controller";
        // The cut face still faces the same neighbour.
        if (!(level.getBlockEntity(c.at(CUT_WIRE)) instanceof ConductorBlockEntity wire)) return c.name + ": no conductor entity on the cut wire";
        Direction cut = c.turn(CUT_WIRE, Direction.EAST);
        if (wire.cutMask() != 1 << cut.ordinal()) return c.name + ": cut mask " + wire.cutMask() + ", want only " + cut;
        // The microwave radio feeds its dish.
        if (!(level.getBlockEntity(c.at(RADIO)) instanceof MicrowaveRadioBlockEntity radio) || radio.findDish(level) == null)
            return c.name + ": the microwave radio does not find its dish";
        // The SDR's transmit chain runs through the amplifier, tuner and arrestor to this copy's feed point.
        if (!(level.getBlockEntity(c.at(SDR)) instanceof SdrBlockEntity sdr)) return c.name + ": no SDR";
        ExciterLink link = sdr.link();
        if (link == null || !link.connected()) return c.name + ": the SDR has no chain";
        var chain = link.chain();
        if (chain.ampIndex() < 0 || chain.tunerIndex() < 0 || !chain.hasArrestor())
            return c.name + ": chain misses the amplifier, tuner or arrestor: " + chain;
        if (!c.at(FEED).equals(chain.feed())) return c.name + ": chain ends at " + chain.feed() + ", want " + c.at(FEED);
        if (link.antenna() == null || !link.antenna().solved()) return c.name + ": the SDR's antenna is not solved";
        return null;
    }

    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".structure_rotation")
    public static void ecm_blocks_turn_with_structures(GameTestHelper h) {
        ServerLevel level = h.getLevel();
        BlockPos src = cell(h, 0);
        build(level, src);
        StructureTemplate template = new StructureTemplate();
        template.fillFromWorld(level, src, SIZE, false, null);

        List<Copy> copies = new ArrayList<>();
        copies.add(new Copy("source", new StructurePlaceSettings(), src));
        int i = 1;
        for (Mirror m : Mirror.values())
            for (Rotation r : Rotation.values()) {
                if (m == Mirror.NONE && r == Rotation.NONE) continue;
                StructurePlaceSettings s = new StructurePlaceSettings().setMirror(m).setRotation(r).addProcessor(FRESH_IDS);
                BoundingBox box = template.getBoundingBox(s, BlockPos.ZERO);
                BlockPos corner = cell(h, i++);
                BlockPos origin = corner.offset(-box.minX(), -box.minY(), -box.minZ());
                template.placeInWorld(level, origin, origin, s, RandomSource.create(i), Block.UPDATE_CLIENTS);
                copies.add(new Copy(m + "+" + r, s, origin));
            }

        String[] failure = {null};
        int checked = 0;
        for (Copy c : copies) {
            int n = checkShapes(level, src, c, failure);
            if (failure[0] != null) break;
            checked += n;
        }
        int perCopy = copies.isEmpty() ? 0 : checked / copies.size();
        if (failure[0] == null && perCopy < 40) failure[0] = "only " + perCopy + " ECM blocks per copy checked";
        EvansComputerMod.LOGGER.info("Structure rotation: {} copies, {} ECM blocks each, shapes ok={}", copies.size(), perCopy, failure[0] == null);

        long start = h.getTick(), wallStart = System.currentTimeMillis();
        String[] last = {null};
        TestDriver.drive(h, NS, "ecm_blocks_turn_with_structures", () -> {
            if (failure[0] != null) return false;
            last[0] = null;
            for (Copy c : copies) {
                last[0] = checkWorking(level, c);
                if (last[0] != null) break;
            }
            if (last[0] == null) {
                EvansComputerMod.LOGGER.info("Structure rotation: {} copies working after {} ticks, {} ms", copies.size(), h.getTick() - start, System.currentTimeMillis() - wallStart);
                return true;
            }
            // Antenna solves run off the server thread on wall time (the test server ticks unthrottled), as in RadioAntennaTests.steps.
            if (System.currentTimeMillis() - wallStart > 30_000) failure[0] = last[0];
            return false;
        }, () -> failure[0]);
    }
}
//?}
