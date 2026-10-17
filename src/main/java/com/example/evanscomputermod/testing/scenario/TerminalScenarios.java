package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import net.minecraft.core.BlockPos;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Pattern;

/**
 * The terminal and shell as a player uses them (docs/TERMINAL_QOL_AUDIT.md,
 * {@code man keys}), spawnable with {@code /ecm scenario spawn <name>}; also
 * the {@code ecm_terminal} GameTests. Every key is a real key press through
 * the client's input packets: the console's key string (what the Terminal
 * screen sends) with the HID key the GUI reads.
 *
 * <p>{@code terminal_line_editing}: one Terminal. The player types a line with
 * a typo, goes Home and Right, deletes and retypes in the middle; deletes a
 * word with Ctrl+W and types another; kills the line with Ctrl+U and yanks it
 * back with Ctrl+Y; jumps a word left with Ctrl+Left and inserts; completes
 * {@code hist} with Tab and runs {@code history}; finds an old line with
 * Ctrl+R; reruns the last one with {@code !!}. Control: a line abandoned with
 * Ctrl+C never runs.
 *
 * <p>{@code terminal_scrollback}: long output ({@code man -P shell}) scrolls
 * off the top; Shift+PgUp shows the scrollback with its "-N" tag, Shift+Up
 * goes one line further, Shift+End returns to the live screen. A running
 * {@code sleep} is stopped with Ctrl+C; a full-screen program (the pager of
 * {@code man shell}) gets Ctrl+C as its quit key instead (control: no "^C").
 *
 * <p>{@code terminal_manual}: {@code help} (headings), {@code help networking}
 * (a heading's commands with summaries), {@code help ping},
 * {@code man keys} in the pager with a search and {@code q}, {@code man -k
 * dns}, {@code less} on a file. Controls: {@code man nosuchpage} and
 * {@code help nosuchpage} say there is none.
 *
 * <p>{@code terminal_pipes}: redirects and pipes: {@code echo > file},
 * {@code >>}, {@code cat file | grep}, {@code cat file | wc}, {@code wc <
 * file}. Control: the file isn't there before the redirect.
 *
 * <p>{@code repl_line_editing}: the Python and Lua prompts edit lines like the
 * shell: a typo fixed in the middle, Up to run a line again, and Ctrl+C
 * abandoning a half-typed line without leaving the REPL (control: the REPL is
 * still there).
 * <pre>
 *   Terminal at (0,1,0) facing north.
 * </pre>
 *
 * <p>{@code terminal_jobs_vars}: Ctrl+Z stops {@code sleep}, {@code jobs}
 * shows it Stopped, {@code bg} runs it on, {@code fg} brings it back and
 * Ctrl+C ends it; {@code $?} after an unknown command is 127; a variable set
 * with {@code NAME=value} expands (not inside single quotes; an unknown one
 * is empty); {@code ls --help} shows the manual summary; {@code man -P
 * shell | less} pages a pipeline's output with keys from the keyboard;
 * {@code echo -e "\a"} rings the bell (control: no bell before).
 *
 * <p>Not covered here: drawing the selection, double/triple click and the
 * clipboard happen in the player's own client ({@code TerminalScreen}); the
 * scenario player has no client. The bytes those keys send are checked in
 * {@code ecm-apps} and {@code ecm-lineedit} tests.
 */
public final class TerminalScenarios {
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        for (Scenario s : List.of(lineEditing(), scrollback(), manual(), pipes(), repl(), jobsAndVariables())) ALL.put(s.name, s);
    }

    private TerminalScenarios() {
    }

    // ------------------------------------------------------------ keys

    private static final int HID_RIGHT = 0x4F, HID_LEFT = 0x50, HID_DOWN = 0x51, HID_UP = 0x52;
    private static final int HID_HOME = 0x4A, HID_PGUP = 0x4B, HID_DELETE = 0x4C, HID_END = 0x4D;
    private static final int HID_TAB = 0x2B;
    private static final int CTRL = UiProto.Mods.CTRL, SHIFT = UiProto.Mods.SHIFT;

    private static void key(ScenarioRun r, int hid, int mods, String console) {
        r.pressKey("pc", hid, mods, console);
    }

    private static void keys(ScenarioRun r, int n, int hid, String console) {
        for (int i = 0; i < n; i++) key(r, hid, 0, console);
    }

    private static void ctrl(ScenarioRun r, char c) {
        key(r, ScenarioPlayer.hidOf(c), CTRL, String.valueOf((char) (c & 0x1f)));
    }

    private static void type(ScenarioRun r, String text) {
        r.typeText("pc", text);
    }

    private static void enter(ScenarioRun r) {
        key(r, ScenarioPlayer.HID_ENTER, 0, "\n");
    }

    /** null once {@code re} (MULTILINE, rows with trailing spaces) is on {@code pc}'s screen. */
    private static String shows(ScenarioRun r, String re) {
        return Pattern.compile(re, Pattern.MULTILINE).matcher(r.screen("pc")).find() ? null : "not on the screen yet: " + re;
    }

    private static String absent(ScenarioRun r, String re) {
        return Pattern.compile(re, Pattern.MULTILINE).matcher(r.screen("pc")).find() ? "on the screen: " + re : null;
    }

    private static int count(ScenarioRun r, String re) {
        var m = Pattern.compile(re, Pattern.MULTILINE).matcher(r.screen("pc"));
        int n = 0;
        while (m.find()) n++;
        return n;
    }

    /** The screen's first row: the scrollback's "-N" tag sits at its right end. */
    private static String topRow(ScenarioRun r) {
        String s = r.screen("pc");
        int nl = s.indexOf('\n');
        return (nl < 0 ? s : s.substring(0, nl)).stripTrailing();
    }

    private static final String LIVE_PROMPT = "^/\\S* > \\s*$";

    // ------------------------------------------------------------ terminal_line_editing

    private static Scenario lineEditing() {
        return Scenario.builder("terminal_line_editing",
                        "The prompt as a line editor: move and fix the middle, Ctrl+W/U/Y, word jumps, Tab, history, Ctrl+R, !!")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Type 'ecoh hello world', then Home, Right x2, Delete x2, type 'ho', Enter: the fixed line runs")
                .mutate(r -> {
                    type(r, "ecoh hello world");
                    key(r, HID_HOME, 0, "\u001b[H");
                    keys(r, 2, HID_RIGHT, "\u001b[C");
                    keys(r, 2, HID_DELETE, "\u001b[3~");
                    type(r, "ho");
                    enter(r);
                }, "type, move, fix and Enter")
                .await(r -> shows(r, "^hello world\\s*$"), "echo ran with the fixed name", 10_000)
                .note("Ctrl+W deletes the last word: 'echo one two', Ctrl+W, 'three'")
                .mutate(r -> {
                    type(r, "echo one two");
                    ctrl(r, 'w');
                    type(r, "three");
                    enter(r);
                }, "Ctrl+W then type")
                .await(r -> shows(r, "^one three\\s*$"), "the word was replaced", 10_000)
                .note("Ctrl+U kills the whole line and Ctrl+Y puts it back")
                .mutate(r -> {
                    type(r, "echo kept");
                    ctrl(r, 'u');
                    ctrl(r, 'y');
                    enter(r);
                }, "Ctrl+U, Ctrl+Y, Enter")
                .await(r -> shows(r, "^kept\\s*$"), "the yanked line ran", 10_000)
                .note("Ctrl+Left jumps back a word: 'echo alpha beta', Ctrl+Left, type 'X'")
                .mutate(r -> {
                    type(r, "echo alpha beta");
                    key(r, HID_LEFT, CTRL, "\u001b[1;5D");
                    type(r, "X");
                    enter(r);
                }, "Ctrl+Left and insert")
                .await(r -> shows(r, "^alpha Xbeta\\s*$"), "X landed before the last word", 10_000)
                .note("Control: a line abandoned with Ctrl+C doesn't run")
                .mutate(r -> {
                    type(r, "echo never");
                    ctrl(r, 'c');
                }, "type 'echo never', Ctrl+C")
                .await(r -> shows(r, "^/\\S* > echo never\\^C\\s*$"), "^C ends the line", 10_000)
                .await(r -> absent(r, "^never\\s*$"), "control: 'never' was not printed", 2_000)
                .note("Tab completes 'hist' to 'history'")
                .mutate(r -> {
                    type(r, "hist");
                    key(r, HID_TAB, 0, "\t");
                }, "type 'hist', Tab")
                .await(r -> shows(r, "^/\\S* > history \\s*$"), "the command was completed", 10_000)
                .mutate(TerminalScenarios::enter, "Enter")
                .await(r -> shows(r, "^\\s+\\d+  echo alpha Xbeta\\s*$"), "history lists the lines with numbers", 10_000)
                .note("Ctrl+R 'kep' finds 'echo kept'; Enter runs it")
                .mutate(r -> {
                    ctrl(r, 'r');
                    type(r, "kep");
                }, "Ctrl+R, type 'kep'")
                .await(r -> shows(r, "\\(reverse-i-search\\)`kep': echo kept"), "the search shows the match", 10_000)
                .mutate(TerminalScenarios::enter, "Enter")
                .await(r -> count(r, "^kept\\s*$") >= 2 ? null : "kept printed once", "the found line ran again", 10_000)
                .note("!! runs the last line again (and shows it)")
                .send("pc", "!!")
                .expect("pc", "^echo kept$", "!! became the last line")
                .expect("pc", "^kept$", "and ran")
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ terminal_scrollback

    private static Scenario scrollback() {
        return Scenario.builder("terminal_scrollback",
                        "Scroll back with Shift+PgUp/Up/End; Ctrl+C stops a program; a full-screen program gets Ctrl+C itself")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Long output scrolls off the top: man -P shell prints the whole page")
                .send("pc", "man -P shell")
                .expect("pc", PlayerKit.PROMPT, "the page was printed")
                .await(r -> absent(r, "^NAME\\s*$"), "control: the page's first line has scrolled away", 2_000)
                .note("Shift+PgUp: a page back, with the '-N' tag top right")
                .mutate(r -> key(r, HID_PGUP, SHIFT, "\u001b[5;2~"), "press Shift+PgUp")
                .await(r -> topRow(r).matches(".*-\\d+$") ? null : "no tag: " + topRow(r), "the view is scrolled back", 10_000)
                .note("Shift+Up: one line further")
                .mutate(r -> {
                    String before = topRow(r).replaceAll(".*-(\\d+)$", "$1");
                    r.note("tag", before);
                    key(r, HID_UP, SHIFT, "\u001b[1;2A");
                }, "press Shift+Up")
                .await(r -> {
                    String now = topRow(r).replaceAll(".*-(\\d+)$", "$1");
                    String before = r.noted("tag");
                    return now.matches("\\d+") && before.matches("\\d+") && Integer.parseInt(now) == Integer.parseInt(before) + 1
                            ? null : "tag " + before + " -> " + now;
                }, "one more line back", 10_000)
                .note("Shift+Home: the oldest line kept (the boot banner)")
                .mutate(r -> key(r, HID_HOME, SHIFT, "\u001b[1;2H"), "press Shift+Home")
                .await(r -> shows(r, "TERMINAL OS"), "the boot banner is shown", 10_000)
                .note("Shift+End: back to the live screen")
                .mutate(r -> key(r, HID_END, SHIFT, "\u001b[1;2F"), "press Shift+End")
                .await(r -> !topRow(r).matches(".*-\\d+$") && shows(r, LIVE_PROMPT) == null ? null : "still scrolled",
                        "the live screen and the prompt are back", 10_000)
                .note("Ctrl+C stops a running program (sleep for 10 minutes)")
                .send("pc", "sleep 600000")
                .waitMs(1500, "let it run")
                .await(r -> absent(r, "\\^C"), "control: still running, nothing stopped it", 1_000)
                .mutate(r -> ctrl(r, 'c'), "press Ctrl+C")
                .expect("pc", "^\\^C$", "^C is shown")
                .expect("pc", PlayerKit.PROMPT, "and the prompt is back")
                .note("A full-screen program gets Ctrl+C as a key: the pager quits on it")
                .send("pc", "man shell")
                .await(r -> shows(r, " man shell 1-\\d+/\\d+"), "the pager's status line", 10_000)
                .mutate(r -> ctrl(r, 'c'), "press Ctrl+C")
                .await(r -> shows(r, LIVE_PROMPT), "the pager quit and the shell is back", 10_000)
                .await(r -> count(r, "^\\^C\\s*$") == 1 ? null : "^C shown " + count(r, "^\\^C\\s*$") + " times",
                        "control: the shell didn't stop it (no second ^C)", 1_000)
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ terminal_manual

    /** Quit the pager if `help` opened one (the index doesn't fit a small screen). */
    private static void quitPagerIfOpen(ScenarioRun r) {
        if (shows(r, " help 1-\\d+/\\d+") == null) type(r, "q");
    }

    private static Scenario manual() {
        return Scenario.builder("terminal_manual",
                        "help with headings, help <topic>, man pages in the pager with search, man -k, less")
                .host("pc", PC, "-")
                .asPlayer()
                .note("help: every command under headings")
                .send("pc", "help")
                .await(r -> shows(r, "^NETWORKING\\s*$"), "the Networking heading", 10_000)
                .await(r -> shows(r, "ifconfig  ip  ping"), "with its commands", 5_000)
                .mutate(TerminalScenarios::quitPagerIfOpen, "q, if it opened in the pager")
                .await(r -> shows(r, LIVE_PROMPT), "back at the prompt", 10_000)
                .note("help <heading>: that heading's commands, each with what it does")
                .send("pc", "help networking")
                .await(r -> shows(r, "^  ping\\s+check that another computer answers"), "ping with its summary", 10_000)
                .mutate(r -> {
                    if (shows(r, " help networking 1-\\d+/\\d+") == null) type(r, "q");
                }, "q, if it opened in the pager")
                .await(r -> shows(r, LIVE_PROMPT), "back at the prompt", 10_000)
                .note("help <name>: what it does, how to type it")
                .send("pc", "help ping")
                .expect("pc", "^ping - ", "the summary")
                .expect("pc", "^USAGE$", "the usage")
                .expect("pc", "^More: man ping$", "and where to read more")
                .expect("pc", PlayerKit.PROMPT, "and ends")
                .note("Control: no such page")
                .send("pc", "help nosuchpage")
                .expect("pc", "^help: no help for nosuchpage\\.", "help says so")
                .expect("pc", PlayerKit.PROMPT, "and ends")
                .send("pc", "man nosuchpage")
                .expect("pc", "^No manual entry for nosuchpage\\.", "man says so")
                .expect("pc", PlayerKit.PROMPT, "and ends")
                .note("man keys: the pager; find 'Ctrl+R' with /, then q")
                .send("pc", "man keys")
                .await(r -> shows(r, " man keys 1-\\d+/\\d+"), "the pager shows the page", 10_000)
                .await(r -> shows(r, "^NAME\\s*$"), "from its top", 2_000)
                .mutate(r -> {
                    type(r, "/reverse");
                    enter(r);
                }, "type /reverse, Enter")
                .await(r -> shows(r, "^\\s+Ctrl\\+R\\s*$|Search backwards"), "the search moved to the history section", 10_000)
                .mutate(r -> type(r, "q"), "press q")
                .await(r -> shows(r, LIVE_PROMPT), "back at the prompt", 10_000)
                .note("man -k: search the summaries")
                .send("pc", "man -k dns")
                .expect("pc", "^dig\\s+ask DNS servers", "dig is found")
                .note("less: page a file")
                .send("pc", "man -P ls > /home/ls.txt")
                .expect("pc", PlayerKit.PROMPT, "the page is written to a file")
                .send("pc", "less /home/ls.txt")
                .await(r -> shows(r, " ls\\.txt 1-\\d+/\\d+"), "less shows it", 10_000)
                .mutate(r -> type(r, "q"), "press q")
                .await(r -> shows(r, LIVE_PROMPT), "back at the prompt", 10_000)
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ terminal_pipes

    private static Scenario pipes() {
        return Scenario.builder("terminal_pipes",
                        "Redirects and pipes: > and >> into a file, cat | grep, cat | wc, wc < file")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Control: the file isn't there yet")
                .send("pc", "cat /home/out.txt")
                .expect("pc", "^cat: /home/out\\.txt: ", "cat can't find it")
                .note("echo > file writes it instead of the screen")
                .send("pc", "echo hello pipes > /home/out.txt")
                .expect("pc", PlayerKit.PROMPT, "nothing printed")
                .send("pc", "cat /home/out.txt")
                .expect("pc", "^hello pipes$", "the file holds the line")
                .send("pc", "echo second line >> /home/out.txt")
                .expect("pc", PlayerKit.PROMPT, "nothing printed")
                .send("pc", "cat /home/out.txt | grep sec")
                .expect("pc", "^second line$", "grep got the file through the pipe", "^hello pipes$")
                .send("pc", "cat /home/out.txt | wc")
                .expect("pc", "^  2 4 24$", "wc counts 2 lines, 4 words, 24 bytes")
                .send("pc", "wc < /home/out.txt")
                .expect("pc", "^  2 4 24$", "and reads the file with <")
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ terminal_jobs_vars

    private static int bells(ScenarioRun r) {
        var c = r.computer("pc");
        return c == null ? -1 : c.bellsRung();
    }

    private static Scenario jobsAndVariables() {
        return Scenario.builder("terminal_jobs_vars",
                        "Ctrl+Z, jobs, bg, fg; $? and variables; <cmd> --help; command | less; the bell")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Ctrl+Z pauses a running program")
                .send("pc", "sleep 600000")
                .waitMs(1000, "let it run")
                .mutate(r -> ctrl(r, 'z'), "press Ctrl+Z")
                .expect("pc", "^\\[1\\]\\+  Stopped    sleep 600000$", "it is a stopped job")
                .expect("pc", PlayerKit.PROMPT, "and the prompt is back")
                .send("pc", "jobs")
                .expect("pc", "^\\[1\\]  Stopped\\s+sleep 600000$", "jobs lists it Stopped")
                .send("pc", "bg")
                .expect("pc", "^\\[1\\]\\+ sleep 600000 &$", "bg lets it run on")
                .send("pc", "jobs")
                .expect("pc", "^\\[1\\]  Running\\s+sleep 600000$", "now Running")
                .send("pc", "fg")
                .expect("pc", "^sleep 600000$", "fg brings it back")
                .waitMs(500, "it runs in the foreground")
                .mutate(r -> ctrl(r, 'c'), "press Ctrl+C")
                .expect("pc", "^\\^C$", "Ctrl+C ends it")
                .expect("pc", PlayerKit.PROMPT, "and the prompt is back")
                .note("$? and variables")
                .send("pc", "nosuchcmd")
                .expect("pc", PlayerKit.PROMPT, "unknown command")
                .send("pc", "echo status=$?")
                .expect("pc", "^status=127$", "$? is 127 after an unknown command")
                .send("pc", "NAME=world")
                .expect("pc", PlayerKit.PROMPT, "set a variable")
                .send("pc", "echo hello $NAME '$NAME' [$NOTSET]")
                .expect("pc", "^hello world \\$NAME \\[\\]$", "expanded, not in single quotes, unknown is empty")
                .note("<command> --help shows its manual summary")
                .send("pc", "ls --help")
                .expect("pc", "^ls - list the files in a directory$", "the summary")
                .expect("pc", "^More: man ls$", "and where to read more")
                .expect("pc", PlayerKit.PROMPT, "and ends")
                .note("A pager at the end of a pipeline reads the keyboard")
                .send("pc", "man -P shell | less")
                .await(r -> shows(r, " \\(stdin\\) 1-\\d+/\\d+"), "less shows the pipeline's output", 10_000)
                .mutate(r -> type(r, "G"), "press G")
                .await(r -> shows(r, "\\(END\\)"), "the keys reach less: the end", 10_000)
                .mutate(r -> type(r, "q"), "press q")
                .await(r -> shows(r, LIVE_PROMPT), "back at the prompt", 10_000)
                .note("The bell")
                .await(r -> bells(r) == 0 ? null : "bells: " + bells(r), "control: no bell yet", 1_000)
                .send("pc", "echo -e ring\\a")
                .expect("pc", "^ring$", "echo printed")
                .await(r -> bells(r) >= 1 ? null : "no bell", "the bell rang", 5_000)
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ repl_line_editing

    private static void fixTypo(ScenarioRun r, String typo, int right, int delete, String insert) {
        type(r, typo);
        key(r, HID_HOME, 0, "\u001b[H");
        keys(r, right, HID_RIGHT, "\u001b[C");
        keys(r, delete, HID_DELETE, "\u001b[3~");
        type(r, insert);
        enter(r);
    }

    private static Scenario repl() {
        return Scenario.builder("repl_line_editing",
                        "Python and Lua prompts edit lines like the shell: fix the middle, Up, Ctrl+C abandons a line")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Python: type 'pirnt(6*7)', Home, Right, Delete x2, 'ri': print(6*7)")
                .send("pc", "python")
                .expect("pc", PlayerKit.PY_PROMPT, "the Python prompt")
                .mutate(r -> fixTypo(r, "pirnt(6*7)", 1, 2, "ri"), "fix the typo and Enter")
                .await(r -> shows(r, "^42\\s*$"), "the fixed line ran", 15_000)
                .mutate(r -> {
                    key(r, HID_UP, 0, "\u001b[A");
                    enter(r);
                }, "Up, Enter")
                .await(r -> count(r, "^42\\s*$") >= 2 ? null : "once", "Up ran it again", 15_000)
                .await(r -> shows(r, "^>>> \\s*\\z|^>>> \\s*\\n(\\s*\\n)*\\z"), "the next prompt is up (Python reads Ctrl+C now)", 15_000)
                .mutate(r -> {
                    type(r, "never_defined +");
                    ctrl(r, 'c');
                    type(r, "6*9");
                    enter(r);
                }, "half a line, Ctrl+C, then 6*9")
                .await(r -> shows(r, "^54\\s*$"), "Ctrl+C dropped the line and Python is still there", 15_000)
                .await(r -> absent(r, "NameError"), "control: the abandoned line never ran", 1_000)
                .send("pc", "exit()")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .note("Lua: the same keys")
                .send("pc", "lua")
                .expect("pc", "^>$", "the Lua prompt")
                .mutate(r -> fixTypo(r, "pirnt(6*7)", 1, 2, "ri"), "fix the typo and Enter")
                .await(r -> count(r, "^42\\s*$") >= 3 ? null : "not yet", "the fixed line ran", 15_000)
                .await(r -> shows(r, "^> \\s*\\n(\\s*\\n)*\\z"), "the next prompt is up (Lua reads Ctrl+C now)", 15_000)
                .mutate(r -> {
                    type(r, "never_defined(");
                    ctrl(r, 'c');
                    type(r, "6*9");
                    enter(r);
                }, "half a line, Ctrl+C, then 6*9")
                .await(r -> count(r, "^54\\s*$") >= 2 ? null : "not yet", "Lua dropped the line and is still there", 15_000)
                .send("pc", "exit")
                .expect("pc", PlayerKit.PROMPT, "back at the shell")
                .timeLimit(60_000)
                .build();
    }
}
//?}
