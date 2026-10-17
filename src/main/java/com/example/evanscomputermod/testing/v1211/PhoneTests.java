package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.PhoneScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * ePhone hardware, namespace {@code ecm_phone} (docs/gui/EPHONE.md,
 * ROADMAP P5). Each test is the scenario {@code /ecm scenario spawn} builds
 * (PhoneScenarios), one per batch so they run one after another.
 */
@GameTestHolder(PhoneTests.NS)
@PrefixGameTestTemplate(false)
public final class PhoneTests {
    static final String NS = "ecm_phone";
    private static final String STRUCTURE = "gametest_phone";

    /** Held = awake, hotbar = asleep (frozen), out of the hotbar = off and back in the same state; hotkey slot 1 reaches the computer. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".basics")
    public static void phone_basics(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_basics"), true);
    }

    /** The battery drains while held, the dock charges it, an empty battery powers the phone off. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".battery")
    public static void phone_battery(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_battery"), true);
    }

    /** Docking and taking the phone back: the dock's state and the update clients get, both ways, and a chunk reload. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".dock")
    public static void phone_dock(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_dock"), true);
    }

    /** Server visibility GLOW sends another player no display packets; FULL does (control); the two toggles. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".visibility")
    public static void phone_visibility(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_visibility"), true);
    }

    /** Two players holding duplicates, a dimension change, dropping the phone. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".duplicates")
    public static void phone_duplicates(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_duplicates"), true);
    }

    /** P5b: boots into the phone shell; Notes; hotkey slot 1 reaches the open app; switch-app key; the hotbar suspends the app and it resumes in the same state. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".gui")
    public static void phone_gui(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.PhoneGuiScenarios.ALL.get("phone_gui"), true);
    }

    /** P5b: a refresh wake in the hotbar posts a notification and is cut after its budget; music plays on under the audio exception. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".background")
    public static void phone_background(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.PhoneGuiScenarios.ALL.get("phone_background"), true);
    }

    /** Background apps: the first-time prompt (Don't allow / Allow), music at home, in the hotbar and in the pocket; a chest turns it off. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".bg_pocket")
    public static void background_pocket(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.BackgroundScenarios.ALL.get("background_pocket"), true);
    }

    /** Background apps: a call that carries sound ducks the music; its quiet stretches don't. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".bg_duck")
    public static void background_duck(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.BackgroundScenarios.ALL.get("background_duck"), true);
    }

    /** Background apps: apps.maxBackgroundApps stops the oldest and names it; 3 keeps all. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".bg_limit")
    public static void background_limit(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.BackgroundScenarios.ALL.get("background_limit"), true);
    }

    /** Background apps: a wake socket wakes a pocketed app at once; without background refresh it waits. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".bg_wake")
    public static void background_wake(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.BackgroundScenarios.ALL.get("background_wake"), true);
    }

    /** Music on the ePhone over Wi-Fi: plays at home, in the hotbar and the pocket, Now Playing card, hotkey 1; a chest stops it. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".music_background")
    public static void music_background(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.MusicScenarios.ALL.get("music_background"), true);
    }

    /** P5b: the GBA app (dialog, ROM list, touch buttons, hotkey A, a paired controller). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".gba")
    public static void phone_gba(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.PhoneGuiScenarios.ALL.get("phone_gba"), true);
    }

    /** The Video Player on the phone: library, a video with sound, hotkey slot 1 pauses, home saves the place. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".video")
    public static void video_phone(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.VideoScenarios.ALL.get("video_phone"), true);
    }

    /** Three Terminals and an ePhone boot in the same tick from a cold kernel cache: all reach a prompt. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".boot_together")
    public static void phone_boot_together(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_boot_together"), true);
    }

    /**
     * Cellular v2 phase 3, phone side: a SIM written at a card writer and an LTE Modem fitted in the slot screen; the
     * phone runs celld by itself, which reads the SIM through the SIM tray and searches; taking the SIM out stops it.
     * (cell_phone_attach, the whole network, has no test until cored and celld-site are merged.)
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cell_celld")
    public static void cell_phone_celld(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.CellPhoneScenarios.ALL.get("cell_phone_celld"), true);
    }

    /** Wi-Fi stays associated walking away from an Access Point until the link runs out (behind a stone wall). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_walk")
    public static void phone_wifi_walk(GameTestHelper h) {
        TestDriver.scenario(h, NS, PhoneScenarios.ALL.get("phone_wifi_walk"), true);
    }
}
//?}
