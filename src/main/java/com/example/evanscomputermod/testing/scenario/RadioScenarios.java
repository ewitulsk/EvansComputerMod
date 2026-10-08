package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Radio &amp; Wireless scenarios (1.21.1), spawnable with
 * {@code /ecm scenario spawn <name>} and reused by {@code RadioTests}.
 * Each radio feature adds its scenarios here (one {@code add(...)} line each).
 */
public final class RadioScenarios {

    // (declared before ALL: the static block below builds scenarios that use them)
    /** Computer A (west) and B (east), 12 blocks apart, each with a Standard SDR on its east side; B has a Speaker on its west side. */
    private static final BlockPos A = new BlockPos(0, 1, 0), B = new BlockPos(12, 1, 0);

    /** Low transmit power keeps the receivers out of clipping at this short range. */
    private static final String TX_POWER = "--power 0";

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        // Features register their scenarios below, one line each.
        add(sdrLab());
        add(radio0Lab());
    }

    static void add(Scenario s) {
        ALL.put(s.name, s);
    }

    // ------------------------------------------------------------ SDR programs

    /**
     * {@code sdr_lab}: A transmits, B receives, through the SDR programs and
     * the server's radio medium. B listens to A's FM test tone through its
     * Speaker ({@code rx_fm}, which reports the strongest audio tone), finds
     * A's carrier with {@code scan}, and decodes an AX.25 packet A sends with
     * {@code afsk1200}. Control: B listening on another frequency decodes
     * nothing while A sends.
     */
    static Scenario sdrLab() {
        return Scenario.builder("sdr_lab",
                        "two computers with SDR blocks: FM tone (rx_fm), band scan, and an AFSK1200 packet from A to B")
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(new SdrBench(true))
                .note("B listens to 146.52 MHz NBFM for 8 s (rx_fm, on its Speaker) while A sends a 1 kHz FM test tone")
                .send("B", "rx_fm 146.52M --seconds 8")
                .send("A", "tx_tone 146.52M --fm 1000 --seconds 5 " + TX_POWER)
                .expect("A", "^tx_tone: done", "A finished transmitting")
                .expectOrFail("B", "^fm: strongest audio tone (9[6-9]\\d|10[0-4]\\d) Hz, ([2-9]\\d|1\\d\\d) dB",
                        "B heard the 1 kHz tone (>= 20 dB over the noise)", "^fm: (strongest audio tone|no audio)")
                .note("A keys a carrier 5 kHz above 146.52 MHz; B scans 146.45-146.60 MHz")
                .send("A", "tx_tone 146.52M --offset 5k --seconds 6 " + TX_POWER)
                .send("B", "scan 146.45M 146.6M --dwell 300")
                .expectOrFail("B", "^scan: [1-9]\\d* active", "scan finished with activity", "^scan: 0 active")
                .expect("B", "146\\.52[45]\\d MHz", "scan logged A's carrier at 146.525 MHz")
                .expect("A", "^tx_tone: done", "A finished transmitting")
                .note("Packet: A sends an AX.25 UI frame with afsk1200; B decodes it")
                .send("B", "afsk1200 recv 144.39M --count 1 --seconds 15")
                .send("A", "afsk1200 send 144.39M N0CALL-1 APRS hello from A " + TX_POWER)
                .expectOrFail("B", "^N0CALL-1>APRS:hello from A", "B decoded A's packet", "^afsk1200: 0 frames")
                .expect("A", "^afsk1200: sent", "A sent it")
                .note("Control: B tuned to 145.00 MHz hears nothing of A's 144.39 MHz packet")
                .send("B", "afsk1200 recv 145.00M --seconds 4")
                .send("A", "afsk1200 send 144.39M N0CALL-1 APRS not for you " + TX_POWER)
                .expectOrFail("B", "^afsk1200: 0 frames decoded", "B decoded nothing off-frequency", "^N0CALL-1>APRS")
                .timeLimit(75_000)
                .build();
    }

    /**
     * {@code radio0_lab}: both computers run {@code radiod} (KISS-TNC style
     * bridge), giving each a {@code radio0} interface on 144.39 MHz packet;
     * A pings B over the air. Control: a ping to an address no station has
     * gets no reply.
     */
    static Scenario radio0Lab() {
        return Scenario.builder("radio0_lab",
                        "IP over AFSK1200 packet: radiod makes radio0 on two computers, and ping crosses the air")
                .host("A", A, "-")
                .host("B", B, "-")
                .decor(new SdrBench(false))
                .note("Both computers bring up radio0 on 144.39 MHz (radiod in the background)")
                .send("A", "radiod radio0 up sdr_0 144.39M --call N0CALL-1 --ip 10.44.0.1/24 --seconds 50 --gain 10 --txdelay 100 -v " + TX_POWER + " &")
                .expect("A", "radio0 up on sdr_0", "A's radio0 is up")
                .send("B", "radiod radio0 up sdr_0 144.39M --call N0CALL-2 --ip 10.44.0.2/24 --seconds 50 --gain 10 --txdelay 100 -v " + TX_POWER + " &")
                .expect("B", "radio0 up on sdr_0", "B's radio0 is up")
                .note("A pings B over the radio (ARP, then echo, each an AX.25 frame; the first may time out while ARP resolves)")
                .send("A", "ping 10.44.0.2 -n 3")
                .expectOrFail("A", "^3 packets sent, [123] received", "B answered over radio0", "^3 packets sent, 0 received")
                .note("Control: nobody has 10.44.0.9")
                .send("A", "ping 10.44.0.9 -n 1")
                .expectOrFail("A", "^1 packets sent, 0 received", "no reply from a missing station", "^1 packets sent, 1 received")
                .timeLimit(75_000)
                .build();
    }

    /** A floor, a Standard SDR east of each computer, and (optionally) a Speaker west of B. */
    private static final class SdrBench implements Scenario.Decor {
        private final boolean speaker;

        SdrBench(boolean speaker) {
            this.speaker = speaker;
        }

        private static BlockPos sdrOf(BlockPos pc) {
            return pc.east();
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>();
            for (int x = -1; x <= 14; x++)
                for (int z = -1; z <= 2; z++)
                    all.add(new BlockPos(x, 0, z));
            all.add(sdrOf(A));
            all.add(sdrOf(B));
            if (speaker) all.add(B.west());
            return all;
        }

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            for (int x = -1; x <= 14; x++)
                for (int z = -1; z <= 2; z++)
                    level.setBlock(run.abs(new BlockPos(x, 0, z)), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
            var sdr = RadioSdrContent.SDR_STANDARD.get().defaultBlockState().setValue(SdrBlock.FACING, Direction.SOUTH);
            level.setBlock(run.abs(sdrOf(A)), sdr, 3);
            level.setBlock(run.abs(sdrOf(B)), sdr, 3);
            if (speaker) {
                level.setBlock(run.abs(B.west()), SpeakerContent.SPEAKER.get().defaultBlockState()
                        .setValue(SpeakerBlock.FACING, Direction.SOUTH), 3);
            }
        }
    }

    private RadioScenarios() {}
}
//?}
