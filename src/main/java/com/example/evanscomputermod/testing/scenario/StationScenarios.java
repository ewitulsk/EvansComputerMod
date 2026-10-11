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
 *   station computer (0,1,0) - Standard SDR (1,1,0) - coax east along the ground (2..9,1,0) to the feed point's west side
 *   ground-mounted quarter-wave vertical at x=10 (25 m band), fed against the ground:
 *     feed point on the grass (10,1,0), vertical axis; 6 blocks of copper wire straight up (10,2..7,0); insulator on top (10,8,0)
 *   listener computer (0,1,20) with a Basic SDR (1,1,20) and a Speaker (-1,1,20); chest (-2,1,2): Handheld Radio, analyzer
 * </pre>
 *
 * The player checks the antenna with the Antenna Analyzer (resonant in the
 * 25 m band, {@link #FREQ} inside its 2:1 SWR band), drops the songs into the
 * station computer's storage folder ({@code <world>/computer-data/<id>/radio/},
 * see {@code scenario/radio_station/CREDITS.txt}), and starts
 * {@code radio_station /radio/playlist.m3u 11.6M --mode am --loop &}. The
 * listener's {@code rx_am} plays it on its Speaker, and a Handheld Radio's
 * receiver held there hears it on SW 11.600 MHz (control: 11.670 MHz is only
 * noise). The antenna sits on the ground on purpose: the solver's patterns are
 * zero below an antenna's horizon, so receivers at ground level hear a raised
 * antenna badly. The station keeps playing after
 * the script (spawned scenarios): tune a Handheld Radio to SW 11.600 MHz.
 */
public final class StationScenarios {
    /** Broadcast frequency: 25 m shortwave broadcast band, a 5 kHz channel the Handheld Radio's SW band tunes to. */
    public static final double FREQ_HZ = 11_600_000;
    public static final String FREQ = "11.6M", FREQ_TEXT = "11.600 MHz";
    /** Control frequency: 70 kHz away, nobody there. */
    public static final String OFF_FREQ = "11.67M", OFF_FREQ_TEXT = "11.670 MHz";
    /**
     * Station transmit power (dBm): 1 mW. The Standard SDR can do 37 dBm (5 W), but that overloads the listener
     * 20 blocks away (its front end clips at -10 dBm); 1 mW still carries well past the field on 31 m.
     */
    public static final int POWER_DBM = 0;

    static final BlockPos STATION = new BlockPos(0, 1, 0), STATION_SDR = new BlockPos(1, 1, 0);
    static final BlockPos LISTENER = new BlockPos(0, 1, 20), LISTENER_SDR = new BlockPos(1, 1, 20), SPEAKER = new BlockPos(-1, 1, 20);
    static final BlockPos CHEST = new BlockPos(-2, 1, 2);
    static final int MAST_X = 10, RADIATOR = 6;
    static final BlockPos FEED = new BlockPos(MAST_X, 1, 0);

    static final String MUSIC = "/data/evanscomputermod/scenario/radio_station/";
    static final List<String> SONGS = List.of("01_wedding_march.wav", "02_jesu_joy.wav", "03_goldberg_aria.wav",
            "04_scriabin_prelude.wav", "05_vivaldi_mandolin.wav");

    private StationScenarios() {}

    static List<BlockPos> radiator() {
        List<BlockPos> out = new ArrayList<>();
        for (int i = 1; i <= RADIATOR; i++) out.add(FEED.above(i));
        return out;
    }

    /** Coax along the ground from the feed point's west side to the SDR's east side. */
    static List<BlockPos> coax() {
        List<BlockPos> out = new ArrayList<>();
        for (int x = MAST_X - 1; x >= 2; x--) out.add(new BlockPos(x, 1, 0));
        return out;
    }

    static List<BlockPos> ground() {
        return PlayerKit.box(-3, 0, -9, 19, 0, 22);
    }

    public static Scenario radioStation() {
        List<BlockPos> foot = new ArrayList<>(ground());
        foot.addAll(List.of(STATION_SDR, LISTENER_SDR, SPEAKER, CHEST, FEED, FEED.above(RADIATOR + 1)));
        foot.addAll(radiator());
        foot.addAll(coax());
        String hz = String.format(Locale.ROOT, "%.0f", FREQ_HZ);
        return Scenario.builder("radio_station",
                        "a shortwave AM broadcast station (computer + SDR + coax + a home-built ground-mounted vertical for the 25 m band)"
                                + " plays a loop of public-domain music on " + FREQ_TEXT + "; a second computer hears it, and so does a"
                                + " Handheld Radio tuned to SW " + FREQ_TEXT)
                .asPlayer()
                .realTime()
                .host("station", STATION, "-")
                .host("listener", LISTENER, "-")
                .decor(PlayerKit.decor(foot, StationScenarios::terrain, StationScenarios::build, r -> PlayerKit.emptyChest(r, CHEST)))
                .timeLimit(60_000)
                .note("Built for you: the station computer with a Standard SDR, coax along the ground to a vertical"
                        + " (a feed point on the grass, 6 blocks of copper wire straight up, insulator on top); a listener computer with a"
                        + " Basic SDR and a Speaker 20 blocks south. The chest has a Handheld Radio and an Antenna Analyzer.")
                .note("Right-click the feed point (10 blocks east, on the ground) with the Antenna Analyzer")
                .await(r -> AntennaScenarios.analyzer(r, FEED), "the analyzer reads the vertical", 30_000)
                .mutate(StationScenarios::checkAntenna, "resonant in the 25 m band, " + FREQ_TEXT + " inside the 2:1 SWR band")
                .note("The songs were dropped into the station computer's storage: <world>/computer-data/<id>/radio/")
                .send("station", "ls /radio")
                .expect("station", "01_wedding_march\\.wav", "the songs and playlist.m3u are on the station computer")
                .note("Start the station: the playlist on a loop, AM, " + FREQ_TEXT + ", 1 mW, in the background")
                .send("station", "radio_station /radio/playlist.m3u " + FREQ + " --mode am --loop --power " + POWER_DBM + " &")
                .expect("station", "^now playing: Mendelssohn - Wedding March", "the station announces the first song", "^radio_station: (?!\\d+ songs? on)")
                .mutate(r -> {
                    BlockPos f = r.abs(FEED), l = r.abs(LISTENER_SDR);
                    r.say("§7  " + r.player().command(String.format(Locale.ROOT, "/ecm radio link %d.5 %d.5 %d.5 %d.5 %d.5 %d.5 %.3f",
                            f.getX(), f.getY(), f.getZ(), l.getX(), l.getY(), l.getZ(), FREQ_HZ / 1e6)));
                }, "/ecm radio link from the feed point to the listener's SDR")
                .note("Listen on the other computer: rx_am " + FREQ + " for 6 s (its Speaker plays it)")
                .send("listener", "rx_am " + FREQ + " --seconds 6")
                .expectOrFail("listener", "^am: (strongest audio tone|no audio)", "rx_am played 6 s on the listener's Speaker",
                        "^rx_am: ")
                .mutate(r -> AntennaScenarios.logScreen(r, "listener"), "log what the listener printed")
                .note("A Handheld Radio held at the listener, SW " + FREQ_TEXT + " (control: " + OFF_FREQ_TEXT + ")")
                .await(r -> handheld(r, true), "the handheld's receiver hears the station on " + FREQ_TEXT
                        + " (signal and audio) and only noise on " + OFF_FREQ_TEXT, 15_000)
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
        PlayerKit.chest(r, CHEST, Direction.NORTH, new ItemStack(RadioHandheldContent.HANDHELD_RADIO.get()),
                new ItemStack(RadioAntennaContent.ANTENNA_ANALYZER.get()));
    }

    static void build(ScenarioRun r) {
        ScenarioPlayer p = r.player();
        Block copper = RadioAntennaContent.COPPER_WIRE.get();
        // The antenna: the feed point clicked onto the grass (vertical axis), the radiator up, an insulator on top.
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

    /** Per run: the two handheld receivers (tuned, off-frequency), ticks listened, and their summed audio power. */
    private static final java.util.Map<ScenarioRun, Object[]> HANDHELD =
            java.util.Collections.synchronizedMap(new java.util.WeakHashMap<>());

    /**
     * What a Handheld Radio held at the listener plays: its server-side receiver
     * (the code the item runs for a player) on SW {@link #FREQ_TEXT} and on
     * {@link #OFF_FREQ_TEXT}, for 3 s of game time. Null when the tuned one has
     * the station's signal and its audio, and the other only noise.
     */
    static String handheld(ScenarioRun r, boolean enforce) {
        var medium = com.example.evanscomputermod.radio.medium.RadioMediumHooks.medium();
        if (medium == null) return "no radio medium";
        BlockPos p = r.abs(LISTENER).above();
        var pose = com.example.evanscomputermod.radio.api.Pose.at(r.level().dimension().location().toString(),
                p.getX() + 0.5, p.getY() + 0.6, p.getZ() + 0.5);
        var tuned = new com.example.evanscomputermod.radio.handheld.HandheldSettings(true,
                com.example.evanscomputermod.radio.handheld.HandheldBand.SW, FREQ_HZ, 80, 0);
        var off = tuned.withFreq(11_670_000);
        Object[] st = HANDHELD.computeIfAbsent(r, k -> new Object[] {
                new com.example.evanscomputermod.radio.handheld.HandheldServer.Session(java.util.UUID.randomUUID()),
                new com.example.evanscomputermod.radio.handheld.HandheldServer.Session(java.util.UUID.randomUUID()),
                new long[1], new double[4]});
        var s1 = (com.example.evanscomputermod.radio.handheld.HandheldServer.Session) st[0];
        var s2 = (com.example.evanscomputermod.radio.handheld.HandheldServer.Session) st[1];
        long[] ticks = (long[]) st[2];
        double[] acc = (double[]) st[3];   // power sum and count, tuned then off
        float[] a1 = s1.receive(pose, tuned, medium), a2 = s2.receive(pose, off, medium);
        if (a1 != null) for (float v : a1) { acc[0] += v * v; acc[1]++; }
        if (a2 != null) for (float v : a2) { acc[2] += v * v; acc[3]++; }
        if (++ticks[0] < 60) return "listening (" + ticks[0] + " ticks)";
        s1.close(medium);
        s2.close(medium);
        HANDHELD.remove(r);
        double rms1 = acc[1] > 0 ? Math.sqrt(acc[0] / acc[1]) : 0, rms2 = acc[3] > 0 ? Math.sqrt(acc[2] / acc[3]) : 0;
        String line = String.format(Locale.ROOT, "handheld at the listener: SW %s signal %.1f dBm, audio RMS %.3f; %s signal %.1f dBm, audio RMS %.3f",
                FREQ_TEXT, s1.lastSignalDbm, rms1, OFF_FREQ_TEXT, s2.lastSignalDbm, rms2);
        EvansComputerMod.LOGGER.info("[radio_station] {}", line);
        r.say("§7  " + line);
        if (!enforce) return null;
        if (!(s1.lastSignalDbm > s2.lastSignalDbm + 20)) {
            String sdr = r.level().getBlockEntity(r.abs(STATION_SDR)) instanceof com.example.evanscomputermod.radio.sdr.SdrBlockEntity be
                    ? be.getPeripheral().radio().describe().replace('\n', ' ') +"; chain " + be.link().chain() : "no SDR";
            r.fail("the handheld doesn't hear the station: " + line + " (station SDR: " + sdr + ")");
        }
        else if (!(rms1 > 0.01)) r.fail("the handheld hears the carrier but no audio: " + line);
        return null;
    }

    static void checkAntenna(ScenarioRun r) {
        double f = AntennaScenarios.resonanceHz(r);
        double[] band = AntennaScenarios.swrBand(r);
        EvansComputerMod.LOGGER.info("[radio_station] analyzer: {}", AntennaScenarios.READING.get(r));
        if (!(f > 11.2e6 && f < 12.1e6)) r.fail("the vertical resonates at " + f + " Hz, not in the 25 m band: " + AntennaScenarios.READING.get(r));
        else if (band == null || !(band[0] <= FREQ_HZ && FREQ_HZ <= band[1]))
            r.fail(FREQ_TEXT + " is outside the 2:1 SWR band: " + AntennaScenarios.READING.get(r));
    }
}
//?}
