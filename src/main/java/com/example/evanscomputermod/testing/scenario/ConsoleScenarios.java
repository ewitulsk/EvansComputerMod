package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerNumbers;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.Container;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LayeredCauldronBlock;
import net.minecraft.world.level.block.LeverBlock;
import net.minecraft.world.level.block.RedstoneLampBlock;

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
 *
 * <p>{@code cc_world} (phase 2): a Redstone Lamp on top of the computer and
 * a Lever on its left, both placed by the player. {@code cc_world.lua} saves
 * a table with {@code textutils.serialize} through {@code fs.open}, reads it
 * back, lists the folder, writes JSON, prints the sides and peripherals
 * ({@code rs.getSides}, {@code peripheral.getNames}), reports the lever off,
 * waits for {@code os.pullEvent("redstone")} (the player flips the lever:
 * {@code LEVER true}), then blinks the lamp from one function of
 * {@code parallel.waitForAny} while the other waits for a typed key (the
 * player types K: the key waiter wins) and leaves the lamp on. Controls: a
 * missing file is {@code nil, "/nope.txt: No such file"}; the lever starts
 * off; the lamp is off before the program.
 *
 * <p>{@code console_world}: the same files, textutils and redstone from
 * Python ({@code console_world.py}).
 *
 * <p>{@code generic_chests} (phase 3): a chest on each side of the computer,
 * placed by the player, who puts 16 cobblestone and 5 diamonds into the left
 * one. {@code cc_chests.lua}, a CC storage program, finds both as
 * {@code inventory} peripherals (no peripheral code in the chest: NeoForge's
 * item capability), lists the left one, reads a diamond's details
 * ({@code getItemDetail}, CC's field names), pushes 3 diamonds right and has
 * the right chest pull the cobblestone; the chests' real contents are
 * checked. Controls: a target that isn't attached and a slot out of range
 * are errors.
 *
 * <p>{@code generic_tanks}: a cauldron on each side, the left one filled
 * with a water bucket by the player; {@code tanks.py} (Python) sees both as
 * {@code fluid_storage}, moves the 1000 mB right ({@code push_fluid}) and the
 * cauldrons' real states are checked; control: no target.
 * <pre>
 *   Terminal at (0,1,0) facing north, Redstone Lamp at (0,2,0) on top,
 *   stone at (-1,1,0) on its left, Lever at (-2,1,0) on the stone.
 * </pre>
 */
public final class ConsoleScenarios {
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    /** Chests or cauldrons on either side (generic_chests, generic_tanks). */
    private static final BlockPos LEFT = new BlockPos(-1, 1, 0), RIGHT = new BlockPos(1, 1, 0);
    private static final BlockPos LAMP = new BlockPos(0, 2, 0);
    /** A block against the computer's left side, and the Lever on it: a lever on the computer itself
     *  would power the computer's block, and through it the lamp. */
    private static final BlockPos STONE = new BlockPos(-1, 1, 0);
    private static final BlockPos LEVER = new BlockPos(-2, 1, 0);
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        for (Scenario s : List.of(ccLua(), python(), ccWorld(), pythonWorld(), genericChests(), genericTanks())) ALL.put(s.name, s);
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

    private static Scenario ccWorld() {
        var b = Scenario.builder("cc_world",
                        "A CC program with files, textutils, redstone (a lever and a lamp), parallel and peripherals")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(LAMP, STONE, LEVER), null, ConsoleScenarios::placeLampAndLever, null))
                .note("Setup (done for you): a Redstone Lamp on top of the computer, a stone block on its left with a Lever on it")
                .note("Type the CC program in (cc_world.lua, from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/cc_world.lua", "/evanscomputermod/scenarios/cc_world.lua");
        return b.await(r -> lit(r) ? "the lamp is lit" : null, "control: the lamp starts off", 2_000)
                .send("pc", "lua cc_world.lua")
                .expect("pc", "^FILES state\\.txt true$", "fs.open wrote the file; fs.list and fs.getSize see it", "^lua: ")
                .expect("pc", "^ROUNDTRIP base 3 2$", "textutils.serialize and unserialize round-trip the table", "^lua: ")
                .expect("pc", "^JSON \\[\"a\",\"b\"\\]$", "textutils.serializeJSON", "^lua: ")
                .expect("pc", "^MISSING /nope\\.txt: No such file$", "control: a missing file is nil and CC's message", "^lua: ")
                .expect("pc", "^SIDES 6 PERIPHERALS \\d+$", "rs.getSides and peripheral.getNames", "^lua: ")
                .expect("pc", "^LEVER false$", "control: the lever is off", "^lua: ")
                .expect("pc", "^Flip the lever$", "it waits for a redstone event")
                .mutate(r -> r.player().useEmpty(r.abs(LEVER), Direction.WEST, false), "flip the lever on")
                .await(r -> leverOn(r) ? null : "the lever is off", "the lever is on", 2_000)
                .expect("pc", "^LEVER true$", "os.pullEvent(\"redstone\") returned and rs.getInput sees it", "^lua: ")
                .expect("pc", "^Press K$", "parallel: one function waits for a key while the other blinks the lamp")
                .await(r -> lit(r) ? null : "the lamp is off", "the lamp blinks: on", 5_000)
                .await(r -> lit(r) ? "the lamp is on" : null, "off", 5_000)
                .await(r -> lit(r) ? null : "the lamp is off", "and on again", 5_000)
                .mutate(r -> r.pressKey("pc", ScenarioPlayer.hidOf('k'), 0, "k"), "type K")
                .expect("pc", "^WINNER 2 BLINKED true$", "the key waiter finishes first and stops the blinking", "^lua: ")
                .expect("pc", "^LAMP true$", "rs.getOutput reads back the lamp's output", "^lua: ")
                .await(r -> lit(r) ? null : "the lamp is off", "the lamp stays on", 5_000)
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    private static Scenario pythonWorld() {
        var b = Scenario.builder("console_world",
                        "Python's fs, textutils and redstone modules: a file round trip, a lever, a lamp")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(LAMP, STONE, LEVER), null, ConsoleScenarios::placeLampAndLever, null))
                .note("Setup (done for you): a Redstone Lamp on top of the computer, a stone block on its left with a Lever on it")
                .note("Type the program in (console_world.py, from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/console_world.py", "/evanscomputermod/scenarios/console_world.py");
        return b.await(r -> lit(r) ? "the lamp is lit" : null, "control: the lamp starts off", 2_000)
                .send("pc", "python console_world.py")
                .expect("pc", "^PY FILES state\\.txt$", "fs.open wrote the file", "Traceback|Error")
                .expect("pc", "^PY ROUNDTRIP base 3 2$", "textutils round-trips the dict", "Traceback|Error")
                .expect("pc", "^PY JSON \\[\"a\",\"b\"\\]$", "textutils.serialize_json", "Traceback|Error")
                .expect("pc", "^PY MISSING /nope\\.txt: No such file$", "control: a missing file", "Traceback|Error")
                .expect("pc", "^PY LEVER False$", "control: the lever is off", "Traceback|Error")
                .expect("pc", "^PY flip the lever$", "it waits for a redstone event")
                .mutate(r -> r.player().useEmpty(r.abs(LEVER), Direction.WEST, false), "flip the lever on")
                .expect("pc", "^PY LEVER True$", "events.pull(\"redstone\") returned and redstone.get_input sees it", "Traceback|Error")
                .expect("pc", "^PY LAMP True$", "redstone.set_output lights the lamp", "Traceback|Error")
                .await(r -> lit(r) ? null : "the lamp is off", "the lamp is lit", 5_000)
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    private static Scenario genericChests() {
        var b = Scenario.builder("generic_chests",
                        "Any inventory is a peripheral: a CC program lists two chests and moves items between them")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(LEFT, RIGHT), null, ConsoleScenarios::placeChests, null))
                .note("Setup (done for you): a chest on each side of the computer, 16 cobblestone and 5 diamonds put into the left one")
                .note("Type the CC program in (cc_chests.lua, from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/cc_chests.lua", "/evanscomputermod/scenarios/cc_chests.lua");
        return b.send("pc", "lua cc_chests.lua")
                .expect("pc", "^TYPES inventory inventory$", "both chests are inventory peripherals", "^lua: ")
                .expect("pc", "^SIZE 27$", "a chest has 27 slots", "^lua: ")
                .expect("pc", "^ITEM \\d+ minecraft:cobblestone 16$", "list() shows the cobblestone", "^lua: ")
                .expect("pc", "^ITEM \\d+ minecraft:diamond 5$", "and the diamonds", "^lua: ")
                .expect("pc", "^DETAIL Diamond 64$", "getItemDetail: displayName and maxCount", "^lua: ")
                .expect("pc", "^PUSHED 3$", "pushItems moves 3 diamonds right", "^lua: ")
                .expect("pc", "^PULLED 16$", "the right chest pulls the cobblestone", "^lua: ")
                .expect("pc", "^COUNTS 2 19$", "2 items stay left, 19 are right", "^lua: ")
                .await(r -> count(r, LEFT, Items.DIAMOND) == 2 && count(r, RIGHT, Items.DIAMOND) == 3
                                && count(r, RIGHT, Items.COBBLESTONE) == 16 && count(r, LEFT, Items.COBBLESTONE) == 0 ? null
                                : "left " + count(r, LEFT, Items.DIAMOND) + " diamonds, right " + count(r, RIGHT, Items.DIAMOND)
                                + " diamonds and " + count(r, RIGHT, Items.COBBLESTONE) + " cobblestone",
                        "the chests really hold them", 5_000)
                .expect("pc", "^NOTARGET .*Target 'top' does not exist$", "control: nothing attached on top", "^lua: ")
                .expect("pc", "^BADSLOT .*Slot out of range \\(1-27\\)$", "control: slot 99", "^lua: ")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    private static Scenario genericTanks() {
        var b = Scenario.builder("generic_tanks",
                        "Any tank is a peripheral: Python moves a cauldron's water into another cauldron")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(LEFT, RIGHT), null, ConsoleScenarios::placeCauldrons, null))
                .note("Setup (done for you): a cauldron on each side of the computer, the left one filled with a water bucket")
                .note("Type the program in (tanks.py, from Python)");
        PlayerKit.typeBundledFile(b, "pc", "/tanks.py", "/evanscomputermod/scenarios/tanks.py");
        return b.await(r -> water(r, LEFT) == 3 && water(r, RIGHT) == 0 ? null : "the cauldrons aren't full and empty",
                        "control: the left cauldron is full, the right one empty", 2_000)
                .send("pc", "python tanks.py")
                .expect("pc", "^PY TYPES fluid_storage fluid_storage$", "both cauldrons are fluid_storage peripherals", "Traceback")
                .expect("pc", "^PY LEFT minecraft:water=1000$", "the left one holds a bucket of water", "Traceback")
                .expect("pc", "^PY MOVED 1000$", "push_fluid moves it right", "Traceback")
                .expect("pc", "^PY RIGHT minecraft:water=1000$", "the right one has it now", "Traceback")
                .await(r -> water(r, LEFT) == 0 && water(r, RIGHT) == 3 ? null
                                : "left level " + water(r, LEFT) + ", right level " + water(r, RIGHT),
                        "the cauldrons really changed", 5_000)
                .expect("pc", "^PY NOTARGET .*Target 'top' does not exist$", "control: nothing attached on top", "Traceback")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }

    /** The player puts a chest on each side and 16 cobblestone and 5 diamonds into the left one. */
    private static void placeChests(ScenarioRun run) {
        run.player().placeAgainst(Blocks.CHEST, run.abs(LEFT), Direction.SOUTH, Direction.EAST, s -> s.is(Blocks.CHEST));
        run.player().placeAgainst(Blocks.CHEST, run.abs(RIGHT), Direction.SOUTH, Direction.WEST, s -> s.is(Blocks.CHEST));
        var p = run.player().entity();
        if (run.player().openMenu(run.abs(LEFT), Direction.UP, false) == null) throw new IllegalStateException("the left chest did not open");
        int hotbar0 = p.containerMenu.slots.size() - 9;
        p.getInventory().setItem(1, new ItemStack(Items.COBBLESTONE, 16));
        p.getInventory().setItem(2, new ItemStack(Items.DIAMOND, 5));
        run.player().shiftClick(hotbar0 + 1);
        run.player().shiftClick(hotbar0 + 2);
        p.closeContainer();
        p.getInventory().clearContent();
    }

    /** The player puts a cauldron on each side and pours a water bucket into the left one. */
    private static void placeCauldrons(ScenarioRun run) {
        run.player().placeAgainst(Blocks.CAULDRON, run.abs(LEFT), Direction.SOUTH, Direction.EAST, s -> s.is(Blocks.CAULDRON));
        run.player().placeAgainst(Blocks.CAULDRON, run.abs(RIGHT), Direction.SOUTH, Direction.WEST, s -> s.is(Blocks.CAULDRON));
        run.player().use(new ItemStack(Items.WATER_BUCKET), run.abs(LEFT), Direction.UP, false);
    }

    private static int count(ScenarioRun r, BlockPos at, net.minecraft.world.item.Item item) {
        if (!(r.level().getBlockEntity(r.abs(at)) instanceof Container c)) return -1;
        int n = 0;
        for (int i = 0; i < c.getContainerSize(); i++) if (c.getItem(i).is(item)) n += c.getItem(i).getCount();
        return n;
    }

    /** Water level of a cauldron (0 empty, 3 full), -1 if it isn't one. */
    private static int water(ScenarioRun r, BlockPos at) {
        var st = r.level().getBlockState(r.abs(at));
        if (st.is(Blocks.CAULDRON)) return 0;
        if (st.is(Blocks.WATER_CAULDRON)) return st.getValue(LayeredCauldronBlock.LEVEL);
        return -1;
    }

    /** The player puts a Redstone Lamp on top of the computer, a stone block against its left side and a Lever on the stone. */
    private static void placeLampAndLever(ScenarioRun run) {
        run.player().placeAgainst(Blocks.REDSTONE_LAMP, run.abs(LAMP), Direction.SOUTH, Direction.DOWN,
                s -> s.is(Blocks.REDSTONE_LAMP));
        run.player().placeAgainst(Blocks.STONE, run.abs(STONE), Direction.SOUTH, Direction.EAST,
                s -> s.is(Blocks.STONE));
        run.player().placeAgainst(Blocks.LEVER, run.abs(LEVER), Direction.SOUTH, Direction.EAST,
                s -> s.is(Blocks.LEVER));
    }

    private static boolean lit(ScenarioRun run) {
        var st = run.level().getBlockState(run.abs(LAMP));
        return st.is(Blocks.REDSTONE_LAMP) && st.getValue(RedstoneLampBlock.LIT);
    }

    private static boolean leverOn(ScenarioRun run) {
        var st = run.level().getBlockState(run.abs(LEVER));
        return st.is(Blocks.LEVER) && st.getValue(LeverBlock.POWERED);
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
