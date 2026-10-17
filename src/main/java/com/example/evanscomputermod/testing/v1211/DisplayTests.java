package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.DisplayScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The display pipeline (docs/gui/DISPLAY_PIPELINE.md, DISPLAY_PIPELINE_SPEC.md
 * section 13), namespace {@code ecm_display} (1.21.1): the
 * {@link DisplayScenarios}, one batch each. The video ones need the FFmpeg
 * sidecar ({@code scripts/Test.ps1 -Ffmpeg}); without it they stop and say so.
 */
@GameTestHolder(DisplayTests.NS)
@PrefixGameTestTemplate(false)
public final class DisplayTests {
    static final String NS = "ecm_display";

    /** The Video Player at 5 ticks/s with a client 100 +- 30 ms away: on time, under 6 Mbit/s, lip sync; control untimed and exact. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".video_smooth_under_lag")
    public static void video_smooth_under_lag(GameTestHelper h) {
        TestDriver.scenario(h, NS, DisplayScenarios.ALL.get("video_smooth_under_lag"), true);
    }

    /** Seek and pause with frames queued: the client drops them; the sound goes on from the new place. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".video_seek_flush")
    public static void video_seek_flush(GameTestHelper h) {
        TestDriver.scenario(h, NS, DisplayScenarios.ALL.get("video_seek_flush"), true);
    }

    /** A paused video is exact on the client within 300 ms. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".lossy_refine")
    public static void lossy_refine(GameTestHelper h) {
        TestDriver.scenario(h, NS, DisplayScenarios.ALL.get("lossy_refine"), true);
    }

    /** player on a Tier 2 wall, two clients: one encode per frame, both on time. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wall_two_viewers")
    public static void wall_two_viewers(GameTestHelper h) {
        TestDriver.scenario(h, NS, DisplayScenarios.ALL.get("wall_two_viewers"), true);
    }

    /** mousetest presents now: no added delay, also at 5 ticks/s. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".interactive_latency")
    public static void interactive_latency(GameTestHelper h) {
        TestDriver.scenario(h, NS, DisplayScenarios.ALL.get("interactive_latency"), true);
    }

    private DisplayTests() {}
}
//?}
