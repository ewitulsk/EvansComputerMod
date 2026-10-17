package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.RedstoneLampBlock;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * The Lua 5.2 interpreter ({@code rust/wasm-programs/lua}), spawnable with
 * {@code /ecm scenario spawn lua_basics}; also the {@code ecm_lua} GameTest.
 *
 * <p>{@code lua_basics}: a Terminal with a Redstone Lamp the player puts on
 * top of it. The player types {@code lua_demo.lua} in (from Python) and runs
 * it: coroutines, an error caught by {@code pcall} (Lua's C error handling,
 * setjmp/longjmp over WASM exception handling), a file written with Lua's
 * {@code io} and read back through {@code shell}, the peripheral list, and
 * every redstone side powered, which lights the lamp. Then the REPL: an
 * expression, a multi-line loop, and the lamp switched off from the
 * {@code shell} module. Controls: the script without its argument stops
 * with its own error, and a missing file is reported.
 * <pre>
 *   Terminal at (0,1,0) facing north, Redstone Lamp at (0,2,0) on top of it.
 * </pre>
 *
 * <p>{@code lua_modules}: Lua has the modules Python has (AGENTS.md). A
 * Terminal with a Speaker the player puts next to it; the player types a
 * module of their own ({@code /lib/tones.lua}) and {@code lua_modules.lua} in
 * and runs it: {@code require} finds the module on the computer,
 * {@code terminal} is {@code shell}, {@code net} lists the interfaces,
 * {@code peripheral.find} finds the Speaker, {@code audio} plays three notes
 * on it (checked on the Speaker block), and {@code radio} sends an AX.25
 * packet through an AFSK modem and FM and decodes it. Controls: a module
 * that isn't there is an error, no Wired Sensor Module or controller is
 * attached ({@code sensors.find()} and {@code controller.find()} are nil),
 * {@code ui.App} can't connect while the GUI isn't running, and the Speaker
 * is quiet before the script runs.
 * <pre>
 *   Terminal at (0,1,0) facing north, Speaker at (-1,1,0) west of it.
 * </pre>
 *
 * <p>{@code lua_ui}: a GUI app written in Lua. The player types
 * {@code lua_counter.lua} in, starts the GUI, launches the app with
 * {@code gui launch lua /lua_counter.lua} and clicks its Add one button
 * twice: the label goes from Count: 0 to Count: 2 (control: Count: 0 before
 * any click).
 */
public final class LuaScenarios {
    public static final String PROGRAM = "lua_demo.lua";
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos LAMP = new BlockPos(0, 2, 0);
    private static final String REPL_PROMPT = "^>$";

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    private static final BlockPos SPEAKER = new BlockPos(-1, 1, 0);
    private static final String COUNTER_APP = "dev.lua.counter";

    static {
        for (Scenario s : List.of(basics(), modules(), ui())) ALL.put(s.name, s);
    }

    private LuaScenarios() {
    }

    private static Scenario basics() {
        var b = Scenario.builder("lua_basics",
                        "Lua 5.2: run a script (coroutines, pcall, io, peripherals, redstone lamp), then the REPL")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(LAMP), null, LuaScenarios::placeLamp, null))
                .note("Setup (done for you): a Redstone Lamp placed on top of the computer")
                .note("Type the Lua demo in (" + PROGRAM + ", from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/" + PROGRAM, "/evanscomputermod/scenarios/" + PROGRAM);
        return b.note("Control: without its count the script stops with its own error")
                .send("pc", "lua " + PROGRAM)
                .expect("pc", "^lua: give a count", "the script's error is reported", "^(LUA|SQUARES) ")
                .expect("pc", PlayerKit.PROMPT, "and the shell prompt is back")
                .note("Control: a file that isn't there")
                .send("pc", "lua nothere.lua")
                .expect("pc", "^lua: cannot open nothere\\.lua", "lua says it can't open it")
                .expect("pc", PlayerKit.PROMPT, "and the shell prompt is back")
                .await(r -> lit(r) ? "the lamp is lit" : null, "control: the lamp starts off", 2_000)
                .note("Run the demo with a count of 5")
                .send("pc", "lua " + PROGRAM + " 5")
                .expect("pc", "^LUA Lua 5\\.2$", "it runs on Lua 5.2", "^lua: ")
                .expect("pc", "^SQUARES 1 4 9 16 25$", "a coroutine yields the squares", "^lua: ")
                .expect("pc", "^CAUGHT false 42$", "pcall catches an error thrown as a table", "^lua: ")
                .expect("pc", "^FILE count=5$", "the file Lua's io wrote reads back", "^lua: ")
                .expect("pc", "^PERIPHERALS \\d+", "the peripheral list", "^lua: ")
                .expect("pc", "^REDSTONE on$", "every side powered", "^lua: ")
                .expect("pc", PlayerKit.PROMPT, "the script ends")
                .await(r -> lit(r) ? null : "the lamp is off", "the lamp on top lights up", 5_000)
                .note("The REPL: an expression, a multi-line loop, the shell module")
                .send("pc", "lua")
                .expect("pc", "^Lua 5\\.2 ", "the REPL's banner")
                .expect("pc", REPL_PROMPT, "the REPL prompt")
                .send("pc", "6 * 7")
                .expect("pc", "^42$", "an expression prints its value")
                .send("pc", "for i = 1, 3 do")
                .expect("pc", "^>>$", "an unfinished statement asks for more")
                .send("pc", "print(i * 10)")
                .send("pc", "end")
                .expect("pc", "^30$", "the loop runs once it is closed")
                .send("pc", "for side = shell.DOWN, shell.RIGHT do shell.set_redstone(side, 0) end")
                .expect("pc", REPL_PROMPT, "redstone off")
                .await(r -> lit(r) ? "the lamp is still lit" : null, "the lamp goes out", 5_000)
                .send("pc", "exit")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    private static Scenario modules() {
        var b = Scenario.builder("lua_modules",
                        "Lua has Python's modules: require of your own file, net, peripheral, audio on a Speaker, radio, ui")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(SPEAKER), null, LuaScenarios::placeSpeaker, null))
                .note("Setup (done for you): a Speaker placed west of the computer")
                .note("Type a module (/lib/tones.lua) and the script (lua_modules.lua) in, from Python");
        typeBundled(b, Map.of("/lib/tones.lua", "lua_tones.lua", "/lua_modules.lua", "lua_modules.lua"));
        return b.await(r -> drained(r) > 0 ? "the speaker already played" : null, "control: the speaker is quiet", 2_000)
                .send("pc", "lua lua_modules.lua")
                .expect("pc", "^REQUIRE tones$", "require finds /lib/tones.lua", "^lua: ")
                .expect("pc", "^MISSING false$", "control: a module that isn't there is an error", "^lua: ")
                .expect("pc", "^TERMINAL true$", "terminal is the shell module", "^lua: ")
                .expect("pc", "^NET [1-9]\\d* \\d+\\.\\d+\\.\\d+\\.\\d+$", "net lists the interfaces and the DNS server", "^lua: ")
                .expect("pc", "^PERIPHERAL \\w+=speaker$", "peripheral.find finds the Speaker", "^(?:lua: |PERIPHERAL none)")
                .expect("pc", "^SENSORS nil CONTROLLER nil$", "control: no sensor module or controller attached", "^lua: ")
                .expect("pc", "^AUDIO 8000$", "audio plays the notes (8 kHz tones)", "^lua: ")
                .await(r -> drained(r) > 0 && rate(r) == 8000 ? null : "the speaker played nothing (rate " + rate(r) + ")",
                        "the Speaker played the notes", 10_000)
                .expect("pc", "^RADIO N0LUA>CQ:tones$", "radio: the packet comes back through AFSK and FM", "^(?:lua: |RADIO nothing)")
                .expect("pc", "^UI false console mode$", "control: ui.App can't connect without the GUI", "^lua: ")
                .expect("pc", PlayerKit.PROMPT, "the script ends")
                .timeLimit(120_000)
                .build();
    }

    private static Scenario ui() {
        var b = Scenario.builder("lua_ui",
                        "A GUI app in Lua: gui launch lua /lua_counter.lua, click Add one twice, Count: 2")
                .host("pc", PC, "-")
                .asPlayer()
                .realTime()
                .note("Type the app in (lua_counter.lua, from Python)");
        typeBundled(b, Map.of("/lua_counter.lua", "lua_counter.lua"));
        return b.send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and shell start", "(?i)unknown command|no GUI device")
                .note("Ctrl+Alt+Backspace back to the console (the GUI keeps running) to launch the app")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> GuiScenarios.ui(r) != null && !GuiScenarios.ui(r).guiVisible() ? null : "GUI still shown",
                        "the console is shown", 5_000)
                .send("pc", "gui launch lua /lua_counter.lua")
                .expect("pc", "^gui: launched lua", "the Lua app starts")
                .until("pc", "gui status", "\"Lua Counter\" .*\\d+x\\d+", "its window is open")
                .note("Back to the GUI and click the app's button")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> GuiScenarios.ui(r) != null && GuiScenarios.ui(r).guiVisible() ? null : "the console is still shown",
                        "the GUI is shown", 5_000)
                .await(r -> GuiScenarios.shows(r, COUNTER_APP, "Count: 0"), "control: the label starts at Count: 0", 15_000)
                .await(r -> GuiScenarios.visible(r, COUNTER_APP, "Add one"), "the Add one button is drawn", 10_000)
                .mutate(r -> GuiScenarios.clickText(r, COUNTER_APP, "Add one"), "click Add one")
                .await(r -> GuiScenarios.shows(r, COUNTER_APP, "Count: 1"), "the label says Count: 1", 10_000)
                .mutate(r -> GuiScenarios.clickText(r, COUNTER_APP, "Add one"), "click Add one again")
                .await(r -> GuiScenarios.shows(r, COUNTER_APP, "Count: 2"), "the label says Count: 2", 10_000)
                .timeLimit(120_000)
                .build();
    }

    /** {@link PlayerKit#typeFiles} with Lua files bundled in the mod (computer path -> resource name). */
    private static void typeBundled(Scenario.Builder b, Map<String, String> files) {
        Map<String, String> texts = new LinkedHashMap<>();
        for (var e : new TreeMap<>(files).entrySet()) {
            String resource = "/evanscomputermod/scenarios/" + e.getValue();
            try (InputStream in = LuaScenarios.class.getResourceAsStream(resource)) {
                if (in == null) throw new IllegalStateException("missing bundled file " + resource);
                String text = new String(in.readAllBytes(), StandardCharsets.UTF_8).replace("\r\n", "\n");
                texts.put(e.getKey(), text.endsWith("\n") ? text.substring(0, text.length() - 1) : text);
            } catch (IOException ex) {
                throw new IllegalStateException("reading " + resource, ex);
            }
        }
        PlayerKit.typeFiles(b, Map.of("pc", texts));
    }

    /** The player puts a Speaker west of the computer. */
    private static void placeSpeaker(ScenarioRun run) {
        run.player().place(SpeakerContent.SPEAKER.get(), run.abs(SPEAKER), Direction.SOUTH,
                s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
    }

    private static long drained(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(SPEAKER)) instanceof SpeakerBlockEntity s ? s.audio().samplesDrained() : -1;
    }

    private static int rate(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(SPEAKER)) instanceof SpeakerBlockEntity s ? s.audio().rate() : -1;
    }

    /** The player puts a Redstone Lamp on top of the computer. */
    private static void placeLamp(ScenarioRun run) {
        run.player().placeAgainst(Blocks.REDSTONE_LAMP, run.abs(LAMP), Direction.SOUTH, Direction.DOWN,
                s -> s.is(Blocks.REDSTONE_LAMP));
    }

    private static boolean lit(ScenarioRun run) {
        var st = run.level().getBlockState(run.abs(LAMP));
        return st.is(Blocks.REDSTONE_LAMP) && st.getValue(RedstoneLampBlock.LIT);
    }
}
//?}
