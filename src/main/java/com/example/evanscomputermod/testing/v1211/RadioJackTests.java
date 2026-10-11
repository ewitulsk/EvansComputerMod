package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.amp.AmplifierBlock;
import com.example.evanscomputermod.radio.amp.ExciterLink;
import com.example.evanscomputermod.radio.amp.RadioAmpContent;
import com.example.evanscomputermod.radio.amp.TunerBlock;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.radio.sdr.SdrBlockEntity;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Coax only connects where a model draws a jack, and signal runs one way
 * through the devices ({@code RfPortBlock}), namespace {@code ecm_radio}.
 * Six SDRs facing north, each read through its {@link ExciterLink}:
 * <ol>
 *   <li>coax on the back (N jack): an arm into the SDR and a chain;</li>
 *   <li>control: coax on the front (no jack): no arm, no chain;</li>
 *   <li>coax on the right side (SMA) through a tuner facing north (TX jack on
 *       its back) and on out of its ANT feedthrough: the tuner is in the
 *       chain and the coax above it has its arm down;</li>
 *   <li>control: the same with the tuner turned so the coax meets its side:
 *       no arm into the tuner, the tuner is not in the chain;</li>
 *   <li>an amplifier touching the SDR's back with its RF IN (its own back) is
 *       in the chain; turned round (RF OUT against the SDR, control) it is
 *       not.</li>
 * </ol>
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioJackTests {
    private RadioJackTests() {}

    private static BlockState sdr() {
        return RadioSdrContent.SDR_STANDARD.get().defaultBlockState().setValue(SdrBlock.FACING, Direction.NORTH);
    }

    private static void coax(GameTestHelper h, BlockPos rel) {
        ConductorBlock.placeConnected(h.getLevel(), h.absolutePos(rel), RadioAntennaContent.COAX_CABLE.get().defaultBlockState());
    }

    private static boolean arm(GameTestHelper h, BlockPos rel, Direction d) {
        return h.getBlockState(rel).getValue(ConductorBlock.property(d));
    }

    private static ExciterLink link(GameTestHelper h, BlockPos rel) {
        return ((SdrBlockEntity) h.getBlockEntity(rel)).link();
    }

    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = RadioTests.NS + ".radio_jacks")
    public static void coax_connects_only_at_jacks(GameTestHelper h) {
        BlockPos back = new BlockPos(4, 2, 4), front = new BlockPos(10, 2, 4), tuned = new BlockPos(16, 2, 4),
                sideways = new BlockPos(22, 2, 4), amped = new BlockPos(28, 2, 4), reversed = new BlockPos(34, 2, 4);
        for (BlockPos p : new BlockPos[] {back, front, tuned, sideways, amped, reversed}) h.setBlock(p, sdr());
        // 1, 2: back and front.
        coax(h, back.south());
        coax(h, front.north());
        // 3: right side (east, SMA) -> coax -> tuner TX (its back, south face) -> ANT (top) -> coax.
        coax(h, tuned.east());
        coax(h, tuned.east().north());
        BlockPos tuner = tuned.east().north(2);
        h.setBlock(tuner, RadioAmpContent.ANTENNA_TUNER.get().defaultBlockState().setValue(TunerBlock.FACING, Direction.NORTH));
        coax(h, tuner.above());
        // 4: control, the tuner turned east: the coax from the south meets its side.
        coax(h, sideways.east());
        coax(h, sideways.east().north());
        BlockPos sideTuner = sideways.east().north(2);
        h.setBlock(sideTuner, RadioAmpContent.ANTENNA_TUNER.get().defaultBlockState().setValue(TunerBlock.FACING, Direction.EAST));
        // 5: an amplifier south of the SDR, against its back: facing south its RF IN (back) meets the SDR's N jack;
        // facing north (control) its RF OUT does.
        h.setBlock(amped.south(), RadioAmpContent.AMPLIFIER_100W.get().defaultBlockState().setValue(AmplifierBlock.FACING, Direction.SOUTH));
        h.setBlock(reversed.south(), RadioAmpContent.AMPLIFIER_100W.get().defaultBlockState().setValue(AmplifierBlock.FACING, Direction.NORTH));
        String[] failure = {null};
        long start = h.getTick();
        TestDriver.drive(h, RadioTests.NS, "coax_connects_only_at_jacks", () -> {
            if (h.getTick() - start < 12) return false;   // ExciterLinks re-resolve every 4 ticks
            if (!arm(h, back.south(), Direction.NORTH)) failure[0] = "coax behind the SDR has no arm into its N jack";
            else if (!link(h, back).connected()) failure[0] = "coax on the SDR's back jack: no chain";
            else if (arm(h, front.north(), Direction.SOUTH)) failure[0] = "control: coax in front of the SDR drew an arm into its front (no jack there)";
            else if (link(h, front).connected()) failure[0] = "control: coax on the SDR's front made a chain: " + link(h, front).chain();
            else if (!arm(h, tuned.east(), Direction.WEST)) failure[0] = "coax on the SDR's side has no arm into its SMA socket";
            else if (!arm(h, tuned.east().north(), Direction.NORTH)) failure[0] = "coax has no arm into the tuner's TX jack";
            else if (!arm(h, tuner.above(), Direction.DOWN)) failure[0] = "coax above the tuner has no arm into its ANT feedthrough";
            else if (link(h, tuned).chain().tunerIndex() < 0) failure[0] = "tuner (TX on its back) not in the chain: " + link(h, tuned).chain();
            else if (link(h, tuned).chain().lineBlocks().size() != 3) failure[0] = "chain should run on out of ANT (3 coax blocks): " + link(h, tuned).chain();
            else if (arm(h, sideways.east().north(), Direction.NORTH)) failure[0] = "control: coax drew an arm into the tuner's side (no jack there)";
            else if (link(h, sideways).chain().tunerIndex() >= 0) failure[0] = "control: a tuner side-on to the coax is in the chain";
            else if (link(h, amped).chain().ampIndex() != 0) failure[0] = "amplifier with RF IN against the SDR's back not in the chain: " + link(h, amped).chain();
            else if (link(h, reversed).chain().ampIndex() >= 0) failure[0] = "control: amplifier with RF OUT against the SDR is in the chain";
            return failure[0] == null;
        }, () -> failure[0]);
    }
}
//?}
