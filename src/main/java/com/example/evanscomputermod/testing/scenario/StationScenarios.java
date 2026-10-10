package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.handheld.RadioHandheldContent;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * {@code radio_station}: a shortwave AM broadcast station a player builds,
 * playing public-domain music on a loop, to listen to with the Handheld Radio.
 *
 * <pre>
 *   station computer (0,1,0) - Standard SDR (1,1,0) - coax east along y=1 to x=9, up to the feed point's west side
 *   ground-plane vertical at x=10 (31 m band):
 *     spruce-fence mast (10,1..2,0); copper-wire hub (10,3,0) with three 7-block copper radials (east, north, south);
 *     feed point (10,4,0), vertical axis; 7 blocks of copper wire up (10,5..11,0); insulator on top (10,12,0)
 *   listener computer (0,1,20) with a Basic SDR (1,1,20) and a Speaker (-1,1,20); chest (-2,1,2): Handheld Radio, analyzer
 * </pre>
 *
 * The player checks the antenna with the Antenna Analyzer (resonant in the
 * 31 m band, {@link #FREQ} inside its 2:1 SWR band), drops the songs into the
 * station computer's storage folder ({@code <world>/computer-data/<id>/radio/},
 * see {@code scenario/radio_station/CREDITS.txt}), and starts
 * {@code radio_station /radio/playlist.m3u 9.7M --mode am --loop &}. The
 * listener's {@code rx_am} hears the music on 9.700 MHz (and plays it on its
 * Speaker); control: a scan of 9.66-9.80 MHz finds only the station (nothing at 9.770 MHz). The station keeps playing after
 * the script (spawned scenarios): tune a Handheld Radio to SW 9.700 MHz.
 */
public final class StationScenarios {
    /** Broadcast frequency: 31 m shortwave band, a 5 kHz channel the Handheld Radio's SW band tunes to. */
    public static final double FREQ_HZ = 9_700_000;
    public static final String FREQ = "9.7M", FREQ_TEXT = "9.700 MHz";
    /** Control frequency: 70 kHz away, nobody there. */
    public static final String OFF_FREQ = "9.77M", OFF_FREQ_TEXT = "9.770 MHz";
    /** Station transmit power (dBm): the Standard SDR's full 5 W. */
    public static final int POWER_DBM = 37;

    static final BlockPos STATION = new BlockPos(0, 1, 0), STATION_SDR = new BlockPos(1, 1, 0);
    static final BlockPos LISTENER = new BlockPos(0, 1, 20), LISTENER_SDR = new BlockPos(1, 1, 20), SPEAKER = new BlockPos(-1, 1, 20);
    static final BlockPos CHEST = new BlockPos(-2, 1, 2);
    static final int MAST_X = 10, RADIAL = 7, RADIATOR = 7;
    static final BlockPos HUB = new BlockPos(MAST_X, 3, 0), FEED = HUB.above();

    static final String MUSIC = "/data/evanscomputermod/scenario/radio_station/";
    static final List<String> SONGS = List.of("01_wedding_march.wav", "02_jesu_joy.wav", "03_goldberg_aria.wav",
            "04_scriabin_prelude.wav", "05_vivaldi_mandolin.wav");

    private StationScenarios() {}

    static List<BlockPos> radiator() {
        List<BlockPos> out = new ArrayList<>();
        for (int i = 1; i <= RADIATOR; i++) out.add(FEED.above(i));
        return out;
    }

    static List<List<BlockPos>> radials() {
        List<List<BlockPos>> out = new ArrayList<>();
        for (Direction d : List.of(Direction.EAST, Direction.NORTH, Direction.SOUTH)) {
            List<BlockPos> r = new ArrayList<>();
            for (int i = 1; i <= RADIAL; i++) r.add(HUB.relative(d, i));
            out.add(r);
        }
        return out;
    }

    /** Coax from the feed point's west side down to the ground and west to the SDR's east side. */
    static List<BlockPos> coax() {
        List<BlockPos> out = new ArrayList<>();
        for (int y = FEED.getY(); y >= 1; y--) out.add(new BlockPos(MAST_X - 1, y, 0));
        for (int x = MAST_X - 2; x >= 2; x--) out.add(new BlockPos(x, 1, 0));
        return out;
    }

    static List<BlockPos> ground() {
        return PlayerKit.box(-3, 0, -9, 19, 0, 22);
    }

    public static Scenario radioStation() {
        List<BlockPos> foot = new ArrayList<>(ground());
        foot.addAll(List.of(STATION_SDR, LISTENER_SDR, SPEAKER, CHEST, HUB, FEED, FEED.above(RADIATOR + 1),
                new BlockPos(MAST_X, 1, 0), new BlockPos(MAST_X, 2, 0)));
        foot.addAll(radiator());
        radials().forEach(foot::addAll);
        foot.addAll(coax());
        String hz = String.format(Locale.ROOT, "%.0f", FREQ_HZ);
        return Scenario.builder("radio_station",
                        "a shortwave AM broadcast station (computer + SDR + coax + a home-built ground-plane vertical for the 31 m band)"
                                + " plays a loop of public-domain music on " + FREQ_TEXT + "; a second computer hears it, and so does a"
                                + " Handheld Radio tuned to SW " + FREQ_TEXT)
                .asPlayer()
                .realTime()
                .host("station", STATION, "-")
                .host("listener", LISTENER, "-")
                .decor(PlayerKit.decor(foot, StationScenarios::terrain, StationScenarios::build, r -> PlayerKit.emptyChest(r, CHEST)))
                .timeLimit(60_000)
                .note("Built for you: the station computer with a Standard SDR, coax to a ground-plane vertical (7-block copper"
                        + " radiator on a vertical feed point, three 7-block radials, insulator on top); a listener computer with a"
                        + " Basic SDR and a Speaker 20 blocks south. The chest has a Handheld Radio and an Antenna Analyzer.")
                .note("Right-click the feed point (10 blocks east, 4 up) with the Antenna Analyzer")
                .await(r -> AntennaScenarios.analyzer(r, FEED), "the analyzer reads the vertical", 30_000)
                .mutate(StationScenarios::checkAntenna, "resonant in the 31 m band, " + FREQ_TEXT + " inside the 2:1 SWR band")
                .note("The songs were dropped into the station computer's storage: <world>/computer-data/<id>/radio/")
                .send("station", "ls /radio")
                .expect("station", "01_wedding_march\\.wav", "the songs and playlist.m3u are on the station computer")
                .note("Start the station: the playlist on a loop, AM, " + FREQ_TEXT + ", 5 W, in the background")
                .send("station", "radio_station /radio/playlist.m3u " + FREQ + " --mode am --loop --power " + POWER_DBM + " &")
                .expect("station", "^now playing: Mendelssohn - Wedding March", "the station announces the first song", "^radio_station: (?!\\d+ songs? on)")
                .note("Listen on the other computer: rx_am " + FREQ + " for 6 s (its Speaker plays it)")
                .send("listener", "rx_am " + FREQ + " --seconds 6")
                .expectOrFail("listener", "^am: strongest audio tone \\d+ Hz, ([2-9]\\d|1\\d\\d) dB",
                        "the listener hears the music (>= 20 dB over the noise)", "^am: (no audio|strongest audio tone \\d+ Hz, 1?\\d dB)")
                .mutate(r -> AntennaScenarios.logScreen(r, "listener"), "log what the listener printed")
                .note("Control: scan 9.66-9.80 MHz: the station's carrier at " + FREQ_TEXT + " is the only signal; "
                        + OFF_FREQ_TEXT + " is empty")
                .send("listener", "scan 9.66M 9.8M --dwell 300")
                .expectOrFail("listener", "^scan: [1-9]\\d* active", "the scan finished with activity", "^scan: 0 active")
                .expectOrFail("listener", "\\A(?=[\\s\\S]*9\\.(69[5-9]|70[0-5])\\d MHz)(?![\\s\\S]*9\\.7[4-9]\\d\\d MHz)",
                        "a carrier at " + FREQ_TEXT + ", nothing at " + OFF_FREQ_TEXT, "9\\.7[4-9]\\d\\d MHz")
                .mutate(r -> AntennaScenarios.logScreen(r, "listener"), "log the scan")
                .note("Now take the Handheld Radio (chest, or it was given to you), right-click to switch it on, sneak + right-click"
                        + " to tune: band SW, " + FREQ_TEXT + " (" + hz + " Hz). The station loops until you break its computer.")
                .mutate(r -> r.say("§7  " + r.player().command("/give @a[distance=..64] evanscomputermod:handheld_radio")),
                        "/give @a[distance=..64] evanscomputermod:handheld_radio")
                .send("station", "jobs")
                .expect("station", "radio_station", "the station is still on the air (jobs)")
                .build();
    }

    static void terrain(ScenarioRun r) {
        PlayerKit.fill(r, ground(), Blocks.GRASS_BLOCK.defaultBlockState());
        PlayerKit.fill(r, List.of(new BlockPos(MAST_X, 1, 0), new BlockPos(MAST_X, 2, 0)), Blocks.SPRUCE_FENCE.defaultBlockState());
        PlayerKit.chest(r, CHEST, Direction.NORTH, new ItemStack(RadioHandheldContent.HANDHELD_RADIO.get()),
                new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()));
    }

    static void build(ScenarioRun r) {
        ScenarioPlayer p = r.player();
        Block copper = RadioAntennaContent.COPPER_WIRE.get();
        // The antenna: hub on the mast, radials out from it, the feed point on the hub, the radiator up, an insulator on top.
        p.placeAgainst(copper, r.abs(HUB), Direction.SOUTH, Direction.DOWN, null);
        for (List<BlockPos> radial : radials()) {
            List<BlockPos> run = new ArrayList<>(List.of(HUB));
            run.addAll(radial);
            for (int i = 1; i < run.size(); i++) {
                Direction back = null;
                for (Direction d : Direction.values()) if (run.get(i).relative(d).equals(run.get(i - 1))) back = d;
                p.placeAgainst(copper, r.abs(run.get(i)), Direction.UP, back, null);
            }
        }
        p.placeAgainst(RadioAntennaContent.FEED_POINT.get(), r.abs(FEED), Direction.SOUTH, Direction.DOWN,
                s -> s.getValue(FeedPointBlock.AXIS) == Direction.Axis.Y);
        for (BlockPos q : radiator()) p.placeAgainst(copper, r.abs(q), Direction.SOUTH, Direction.DOWN, null);
        p.placeAgainst(RadioAntennaContent.INSULATOR.get(), r.abs(FEED.above(RADIATOR + 1)), Direction.SOUTH, Direction.DOWN, null);
        // The SDRs and the speaker, then the coax from the feed point to the station's SDR.
        p.place(RadioSdrContent.SDR_STANDARD.get(), r.abs(STATION_SDR), Direction.NORTH, s -> s.getValue(SdrBlock.FACING) == Direction.NORTH);
        p.place(RadioSdrContent.SDR_BASIC.get(), r.abs(LISTENER_SDR), Direction.NORTH, s -> s.getValue(SdrBlock.FACING) == Direction.NORTH);
        p.place(SpeakerContent.SPEAKER.get(), r.abs(SPEAKER), Direction.NORTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.NORTH);
        List<BlockPos> coax = coax();
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(coax.get(0)), Direction.WEST, Direction.EAST, null);
        for (int i = 1; i < coax.size(); i++) {
            Direction back = null;
            for (Direction d : Direction.values()) if (coax.get(i).relative(d).equals(coax.get(i - 1))) back = d;
            p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(coax.get(i)), Direction.NORTH, back, null);
        }
        // The music: what a player copies into <world>/computer-data/<station id>/radio/.
        List<String> files = new ArrayList<>(SONGS);
        files.add("playlist.m3u");
        files.add("CREDITS.txt");
        var dir = PlayerKit.dropFiles(r, "station", "radio", MUSIC, files);
        r.say("§7  songs copied to " + dir);
    }

    static void checkAntenna(ScenarioRun r) {
        double f = AntennaScenarios.resonanceHz(r);
        double[] band = AntennaScenarios.swrBand(r);
        EvansComputerMod.LOGGER.info("[radio_station] analyzer: {}", AntennaScenarios.READING.get(r));
        if (!(f > 9.2e6 && f < 10.2e6)) r.fail("the vertical resonates at " + f + " Hz, not in the 31 m band: " + AntennaScenarios.READING.get(r));
        else if (band == null || !(band[0] <= FREQ_HZ && FREQ_HZ <= band[1]))
            r.fail(FREQ_TEXT + " is outside the 2:1 SWR band: " + AntennaScenarios.READING.get(r));
    }
}
//?}
