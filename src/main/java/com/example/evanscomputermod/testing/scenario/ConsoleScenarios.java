package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerNumbers;
import net.minecraft.core.BlockPos;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Console programs (crates/ecm-console): ComputerCraft programs run unchanged
 * in Lua, and the modules under them from Python. Spawnable with
 * {@code /ecm scenario spawn cc_lua} / {@code console_python}; also the
 * {@code ecm_console} GameTest.
 *
 * <p>{@code cc_lua}: the player types {@code cc_demo.lua} in, a program
 * written the way CC programs are (no require, no flags), and runs it with
 * {@code lua cc_demo.lua}. It clears the screen and draws in colour
 * ({@code term}, {@code colours}), prints the screen size and the clocks
 * ({@code os.day()} and {@code os.time()} follow the world's day time, checked
 * against the world; {@code os.getComputerID()} is this computer's number),
 * pulls its own queued event and then its timer from the one event queue
 * ({@code os.queueEvent}, {@code os.startTimer}, {@code os.pullEvent}), and
 * asks for a name with {@code read()}: the player types "Evna", presses
 * Backspace twice and types "an", and it says HELLO Evan!. Then it waits for
 * Q with {@code os.pullEvent("key")}: the player presses X (control: KEY x,
 * the program keeps waiting) and then Q (KEY q, DONE, back at the shell).
 * Controls: an unknown locale is an error ({@code BADLOCALE false}); the typo
 * never reaches the program.
 *
 * <p>{@code console_python}: the same modules from Python
 * ({@code console_demo.py}: term, colors, events, keys, computer): drawn
 * lines, the clocks, a queued event before a timer, and keys; X is a control
 * as above.
 * <pre>
 *   Terminal at (0,1,0) facing north.
 * </pre>
 */
public final class ConsoleScenarios {
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        for (Scenario s : List.of(ccLua(), python())) ALL.put(s.name, s);
    }

    private ConsoleScenarios() {
    }

    private static Scenario ccLua() {
        var b = Scenario.builder("cc_lua",
                        "A ComputerCraft program run unchanged in Lua: term colours, os clocks, one event queue, read(), key events")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Type the CC program in (cc_demo.lua, from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/cc_demo.lua", "/evanscomputermod/scenarios/cc_demo.lua");
        return b.send("pc", "lua cc_demo.lua")
                .expect("pc", "^CC DEMO CraftOS 1\\.9$", "it clears the screen and draws its title", "^lua: ")
                .expect("pc", "^SIZE \\d+x\\d+$", "term.getSize gives the console's size", "^lua: ")
                .expect("pc", "^CLOCK day \\d+ hour \\d+\\.\\d+ id \\d+$", "os.day, os.time and os.getComputerID", "^lua: ")
                .await(ConsoleScenarios::clockMatchesWorld, "the clock is the world's day time, the id this computer's number", 5_000)
                .expect("pc", "^BADLOCALE false$", "control: an unknown locale is an error", "^lua: ")
                .expect("pc", "^EVENT note queued first$", "its own queued event comes first", "^lua: ")
                .expect("pc", "^TIMER true 0\\.[5-9]$", "then its half-second timer", "^lua: ")
                .expect("pc", "^Name\\?$", "read() waits for a name", "^lua: ")
                .note("Type a name with a typo, fix it with Backspace, then Enter")
                .mutate(r -> r.typeText("pc", "Evna"), "type Evna")
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.HID_BACKSPACE, 0, "\b"), "press Backspace")
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.HID_BACKSPACE, 0, "\b"), "press Backspace")
                .send("pc", "an")
                .expect("pc", "^HELLO Evan!$", "read() returns the edited line", "^HELLO (?!Evan!$)|^lua: ")
                .expect("pc", "^Press Q$", "it waits for Q")
                .note("Control: X is a key event too, but not the one it waits for")
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.hidOf('x'), 0, "x"), "press X")
                .expect("pc", "^KEY x$", "X arrives as a key event with its key code", PlayerKit.PROMPT)
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.hidOf('q'), 0, "q"), "press Q")
                .expect("pc", "^KEY q$", "Q arrives", "^lua: ")
                .expect("pc", "^DONE", "the blit line and the end", "^lua: ")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    private static Scenario python() {
        var b = Scenario.builder("console_python",
                        "Python's console modules: term drawing, computer clocks, events (queued, timer, keys), keys")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Type the program in (console_demo.py, from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/console_demo.py", "/evanscomputermod/scenarios/console_demo.py");
        return b.send("pc", "python console_demo.py")
                .expect("pc", "^PY TERM \\d+x\\d+$", "term draws at set positions", "Traceback|Error")
                .expect("pc", "^PY CLOCK day \\d+ hour \\d+\\.\\d+ id \\d+$", "computer.day, time and id", "Traceback|Error")
                .await(ConsoleScenarios::clockMatchesWorld, "the clock is the world's day time, the id this computer's number", 5_000)
                .expect("pc", "^PY EVENT note from python$", "its queued event comes first", "Traceback|Error")
                .expect("pc", "^PY TIMER True$", "then its timer", "Traceback|Error")
                .expect("pc", "^PY press Q$", "it waits for Q", "Traceback|Error")
                .note("Control: X is a key event too, but not the one it waits for")
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.hidOf('x'), 0, "x"), "press X")
                .expect("pc", "^PY KEY x$", "X arrives as a key event", PlayerKit.PROMPT)
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.hidOf('q'), 0, "q"), "press Q")
                .expect("pc", "^PY KEY q$", "Q arrives", "Traceback|Error")
                .expect("pc", "^PY DONE$", "the end", "Traceback|Error")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    private static final Pattern CLOCK = Pattern.compile("CLOCK day (\\d+) hour (\\d+\\.\\d+) id (\\d+)");

    /** The printed clock against the world's day time (read, not set) and the computer's number. */
    private static String clockMatchesWorld(ScenarioRun r) {
        Matcher m = CLOCK.matcher(r.screen("pc"));
        if (!m.find()) return "no CLOCK line on the screen";
        long ticks = r.level().getDayTime() + 6000;
        long day = ticks / 24000 + 1;
        double hour = (ticks % 24000) / 1000.0;
        int printedDay = Integer.parseInt(m.group(1));
        double printedHour = Double.parseDouble(m.group(2));
        // The program read the clock a moment ago: allow a few seconds of world time.
        if (printedDay != day && !(printedDay == day - 1 && hour < 0.2)) return "day " + printedDay + ", the world's is " + day;
        double diff = (hour - printedHour + 24) % 24;
        if (diff > 0.2) return "hour " + printedHour + ", the world's is " + hour;
        if (!(r.level().getBlockEntity(r.abs(PC)) instanceof TerminalBlockEntity t)) return "no terminal";
        int id = ComputerNumbers.of(r.level().getServer(), t.getComputerId());
        if (Integer.parseInt(m.group(3)) != id) return "id " + m.group(3) + ", the computer's number is " + id;
        return null;
    }
}
//?}
