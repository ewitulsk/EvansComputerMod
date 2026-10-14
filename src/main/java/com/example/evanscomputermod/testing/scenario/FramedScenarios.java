package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.cellular.CellAntennaBlock;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.List;

/**
 * {@code sdr_framed}: the SDR's framed PHY mode between two player-built stations.
 *
 * <pre>
 *   A: computer (0,1,0) - Advanced SDR (1,1,0) facing south - coax (2,1,0) (3,1,0) and up (3,2,0) -
 *      LTE Sector Antenna on top of the coax (3,3,0), pointing east (its coax jack is on its bottom)
 *   B: computer (16,1,0) - Advanced SDR (15,1,0) - coax (14,1,0) (13,1,0) (13,2,0) - LTE Sector Antenna (13,3,0) pointing west
 *   floor y=0, x -1..17, z -2..2; the two panels face each other 10 m apart
 * </pre>
 *
 * Everything is typed on the two computers with the {@code framed} program: A becomes an LTE base
 * station on channel 3 (address 1), B a UE (address 2), both at 0 dBm. A broadcasts three frames
 * and B prints them with RSSI, SINR and CQI; B sends A a frame with HARQ and A's radio acknowledges
 * it; A starts a 100 ms beacon and B counts about 30 in 3 s, exactly 100 ms apart on the airtime
 * clock. Control: B retuned to channel 4 hears none of A's beacons.
 *
 * <p>The antennas are LTE sector panels because wire antennas built from blocks are modelled up to
 * 30 MHz (fine wire 500 MHz): at 800 MHz a block-wire vertical is a -30 dBi antenna with a 30 dB mismatch.
 */
public final class FramedScenarios {
    static final BlockPos A = new BlockPos(0, 1, 0), B = new BlockPos(16, 1, 0);

    private FramedScenarios() {}

    static List<BlockPos> floor() {
        return PlayerKit.box(-1, 0, -2, 17, 0, 2);
    }

    /** +1 for A (station grows east), -1 for B (west). */
    static int dir(BlockPos pc) {
        return pc.equals(A) ? 1 : -1;
    }

    /** The SDR, coax and panel of the station at {@code pc}. */
    static List<BlockPos> station(BlockPos pc) {
        int d = dir(pc);
        return List.of(pc.east(d), pc.east(2 * d), pc.east(3 * d), pc.east(3 * d).above(), pc.east(3 * d).above(2));
    }

    /** The station's antenna position. */
    static BlockPos panel(BlockPos pc) {
        return pc.east(3 * dir(pc)).above(2);
    }

    static void build(ScenarioRun r, BlockPos pc) {
        ScenarioPlayer p = r.player();
        int d = dir(pc);
        Direction toward = d > 0 ? Direction.EAST : Direction.WEST, back = toward.getOpposite();
        // The Advanced SDR next to the computer, front to the south (its side jacks face the coax).
        p.place(RadioSdrContent.SDR_ADVANCED.get(), r.abs(pc.east(d)), Direction.SOUTH, s -> s.getValue(SdrBlock.FACING) == Direction.SOUTH);
        // Coax from the SDR's side jack outwards, then one block up.
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(pc.east(2 * d)), Direction.SOUTH, back, null);
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(pc.east(3 * d)), Direction.SOUTH, back, null);
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(pc.east(3 * d).above()), Direction.SOUTH, Direction.DOWN, null);
        // The LTE Sector Antenna clicked onto the top of the coax, the player standing on the far side so it points at the other station.
        p.placeAgainst(CellularContent.LTE_ANTENNA.get(), r.abs(panel(pc)), toward, Direction.DOWN,
                s -> s.getValue(CellAntennaBlock.FACING) == toward);
    }

    public static Scenario sdrFramed() {
        List<BlockPos> floor = floor();
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(station(A));
        foot.addAll(station(B));
        return Scenario.builder("sdr_framed",
                        "two computers with Advanced SDRs and LTE sector antennas on coax use the SDR's framed PHY mode (LTE): broadcast"
                                + " frames with RSSI/SINR/CQI, a HARQ-acknowledged frame, beacon offload; control: another channel hears nothing")
                .asPlayer()
                .realTime()
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    build(r, A);
                    build(r, B);
                }, null))
                .note("Built for you: next to each computer an Advanced SDR, coax from its side jack two blocks out and one up, and an"
                        + " LTE Sector Antenna clicked onto the top of the coax (its jack is underneath), the two panels facing each other.")
                .note("A: framed LTE base station on channel 3 (791-841 MHz downlink / 703-753 MHz uplink plan), 0 dBm, address 1")
                .send("A", "framed config --phy lte --role bs --channel 3 --power 0 --addr 1")
                .expectOrFail("A", "^framed: lte bs channel 3 tx 811\\.0 MHz rx 723\\.0 MHz bw 10 MHz power 0\\.0 dBm address 00000001",
                        "A is an LTE base station on channel 3", "^framed: (?!lte bs)")
                .note("B: framed LTE UE on channel 3, 0 dBm, address 2")
                .send("B", "framed config --phy lte --role ue --channel 3 --power 0 --addr 2")
                .expectOrFail("B", "^framed: lte ue channel 3 tx 723\\.0 MHz rx 811\\.0 MHz", "B is an LTE UE on channel 3", "^framed: (?!lte ue)")
                .mutate(r -> {
                    for (BlockPos pc : List.of(A, B))
                        if (r.level().getBlockEntity(r.abs(pc.east(dir(pc)))) instanceof com.example.evanscomputermod.radio.sdr.SdrBlockEntity be) {
                            var ant = be.endpoint().antenna();
                            r.say(String.format(java.util.Locale.ROOT, "§7  %s's SDR: antenna %s, peak %.1f dBi, feed loss %.1f dB at %s",
                                    pc.equals(A) ? "A" : "B", be.link().panel() != null ? "LTE panel" : "none", ant.peakGainDbi(), ant.feedLossDb(),
                                    be.endpoint().pose()));
                        }
                }, "log both SDRs' antennas (check)")
                .note("B listens while A broadcasts three frames (CQI 1, the robust broadcast rate)")
                .send("B", "framed recv --count 3 --seconds 15")
                .send("A", "framed send bcast hello from A --count 3 --interval 300")
                .expect("A", "^framed: sent 3 frame", "A sent three frames")
                .expectOrFail("B", "^rx 00000001 -> bcast seq \\d+ rssi -\\d+\\.\\d dBm sinr \\d+\\.\\d dB cqi \\d+ \\(sent 1\\) slot \\d+ t [\\d.]+ s \"hello from A\"",
                        "B received A's frame with RSSI, SINR, CQI, slot and time", "^framed: 0 frames")
                .expectOrFail("B", "^framed: 3 frames \\(0 beacons\\)", "B received all three", "^framed: [0-2] frames")
                .note("B sends A a frame with HARQ: A's radio acknowledges it on the air, B's radio reports it acked")
                .send("A", "framed recv --count 1 --seconds 15")
                .send("B", "framed send 1 ack me --ack")
                .expectOrFail("B", "^tx 00000001: acked \\(attempts [1-4]", "B's frame was acknowledged by A's radio", "^tx 00000001: (failed|rejected|no status)")
                .expectOrFail("A", "^rx 00000002 -> 00000001 seq \\d+ .*\"ack me\"", "A received B's frame on the uplink", "^framed: 0 frames")
                .note("Beacon offload: A's radio repeats a frame every 100 ms; B counts them for 3 s")
                .send("A", "framed beacon 100 cell A")
                .expect("A", "^framed: beacon 0 every 100 ms", "A's beacon is running")
                .send("B", "framed recv --seconds 3 --quiet")
                .expectOrFail("B", "^framed: \\d+ frames \\((2[0-9]|3[0-2]) beacons\\) in [\\d.]+ s, .*beacon every 100\\.0 ms",
                        "B heard ~30 beacons exactly 100 ms apart on the airtime clock", "^framed: \\d+ frames \\((\\d|1\\d) beacons\\)")
                .note("Control: B on channel 4 hears none of A's beacons on channel 3")
                .send("B", "framed config --channel 4")
                .expect("B", "^framed: lte ue channel 4", "B is on channel 4")
                .send("B", "framed recv --seconds 3")
                .expectOrFail("B", "^framed: 0 frames", "nothing on channel 4", "^framed: [1-9]\\d* frames")
                .send("A", "framed beacon off")
                .expect("A", "^framed: beacon 0 off", "A's beacon stopped")
                .send("A", "framed status")
                .expect("A", "^beacons_sent [1-9]\\d*", "A's radio counted the beacons it sent")
                .timeLimit(75_000)
                .build();
    }
}
//?}
