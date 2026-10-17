package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.item.ModItems;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Pattern;

/**
 * Vim (the real Vim 9.2, {@code rust/wasm-programs/vim}) as a player uses it
 * at a Terminal, spawnable with {@code /ecm scenario spawn <name>}; also the
 * {@code ecm_vim} GameTests. Every key is a real key press through the
 * client's input packets: Esc and Enter as the Terminal screen sends them,
 * Ctrl+letters as control characters.
 *
 * <p>{@code vim_editing}: {@code vim notes.txt}; insert three lines, then
 * change a word ({@code cw}), delete with a count ({@code d2w}), undo and redo
 * ({@code u}, Ctrl+R), change inside quotes ({@code ci"}), delete a word with
 * its space ({@code daw}), yank into register a and put it ({@code "ayy},
 * {@code "ap}), record a macro and play it twice ({@code qq...q},
 * {@code 2@q}), repeat with {@code .}, substitute ({@code :%s}), search and a
 * mark ({@code /lazy}, {@code ma}, {@code 'a}), Visual line and charwise
 * Visual ({@code VjU}, {@code ve~}); {@code :w}, {@code :q}; {@code cat}
 * shows the file. Controls: Ctrl+C doesn't stop Vim (it says how to quit),
 * {@code :q} with changes refuses (E37), and the file isn't there before
 * {@code :w}.
 *
 * <p>{@code vim_windows}: two files as buffers ({@code :ls}, {@code :bn}),
 * a split showing both, Ctrl+W j to the other window, a vertical split on a
 * new file written there, a tab page and {@code gt}, {@code :only},
 * {@code :qa!}. Control: the new file holds what was typed.
 *
 * <p>{@code vim_runtime}: a {@code ~/.vimrc} from the computer's files turns on
 * line numbers and syntax; {@code vi} (the alias) opens a Python file: the
 * filetype is python, {@code def} is highlighted as pythonStatement in its
 * own colour, {@code :help} opens the main help file and {@code :version}
 * names the port. Controls: {@code :!ls} says shell commands aren't
 * available (E9999), Ctrl+Z says Vim can't be suspended (E9998).
 *
 * <p>{@code vim_resize}: Vim on a Tier 2 Terminal is 160x50; the player swaps
 * in a Phosphor panel (80x25) and Vim redraws at the new size, still editing
 * the same buffer. Control: before the swap Vim had the old size.
 * <pre>
 *   Terminal at (0,1,0) facing north.
 * </pre>
 */
public final class VimScenarios {
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static {
        for (Scenario s : List.of(editing(), windows(), runtime(), resize())) ALL.put(s.name, s);
    }

    private VimScenarios() {
    }

    // ------------------------------------------------------------ keys

    private static final int HID_ESC = 0x29;
    private static final int CTRL = UiProto.Mods.CTRL;
    /** Vim starts from a 3 MB program: the first start can take a while while it compiles. */
    private static final int START_MS = 30_000;

    private static void type(ScenarioRun r, String text) {
        r.typeText("pc", text);
    }

    private static void esc(ScenarioRun r) {
        r.pressKey("pc", HID_ESC, 0, "\u001b");
    }

    private static void enter(ScenarioRun r) {
        r.pressKey("pc", ScenarioPlayer.HID_ENTER, 0, "\n");
    }

    private static void ctrl(ScenarioRun r, char c) {
        r.pressKey("pc", ScenarioPlayer.hidOf(c), CTRL, String.valueOf((char) (c & 0x1f)));
    }

    /** A command-line command: ':' + text + Enter. */
    private static void ex(ScenarioRun r, String cmd) {
        type(r, ":" + cmd);
        enter(r);
    }

    /** null once {@code re} (MULTILINE, rows with trailing spaces) is on the screen. */
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

    /** Every line in order, each alone on its row (Vim's text, no line numbers). */
    private static String lines(ScenarioRun r, String... lines) {
        StringBuilder re = new StringBuilder("^");
        for (int i = 0; i < lines.length; i++) {
            if (i > 0) re.append("\\n");
            re.append(Pattern.quote(lines[i])).append(" *$");
        }
        return shows(r, re.toString());
    }

    private static final String LIVE_PROMPT = "^/\\S* > \\s*$";

    /**
     * Null once Esc has taken effect (Insert mode's "-- INSERT --" is gone). Vim
     * waits 'ttimeoutlen' (100 ms) after an Esc to see whether it starts a key's
     * escape sequence, so keys typed right after it would join it (Esc j is
     * Alt+J), as on any terminal: the next keys wait for this.
     */
    private static String normalMode(ScenarioRun r) {
        return absent(r, "-- INSERT --");
    }

    // ------------------------------------------------------------ vim_editing

    private static Scenario editing() {
        return Scenario.builder("vim_editing",
                        "Vim's modes, motions, operators, counts, text objects, registers, macros, ., undo/redo, :s, search, marks and Visual mode")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Open a new file: vim notes.txt")
                .send("pc", "vim notes.txt")
                .await(r -> shows(r, "\"notes\\.txt\" \\[New\\]"), "Vim shows the new file", START_MS)
                .note("Vim holds the keyboard: the Terminal screen sends plain Esc to it (Shift+Esc closes the screen)")
                .await(r -> r.computer("pc").programHoldsKeys() ? null : "the screen would take Esc",
                        "Esc is Vim's key (the computer tells the player's screen)", 5_000)
                .note("i, type three lines (Enter between them), Esc")
                .mutate(r -> {
                    type(r, "iThe quick brown fox");
                    enter(r);
                    type(r, "jumps over the lazy dog");
                    enter(r);
                    type(r, "say \"hello\" twice");
                    esc(r);
                }, "insert three lines")
                .waitMs(300, "Vim's Esc timeout (ttimeoutlen, 100 ms) passes")
                .await(VimScenarios::normalMode, "Esc took effect: Normal mode", 5_000)
                .await(r -> lines(r, "The quick brown fox", "jumps over the lazy dog", "say \"hello\" twice"), "the three lines", 10_000)
                .note("Control: Ctrl+C is Vim's key here: Vim says how to quit and keeps running")
                .mutate(r -> ctrl(r, 'c'), "press Ctrl+C")
                .await(r -> shows(r, "Type  :qa!  and press <Enter> to abandon all changes and exit Vim"), "Vim's own Ctrl+C message", 10_000)
                .await(r -> absent(r, LIVE_PROMPT), "control: still in Vim, no shell prompt", 1_000)
                .note("gg w cw slow Esc: change a word")
                .mutate(r -> {
                    type(r, "ggwcwslow");
                    esc(r);
                }, "gg w cw slow Esc")
                .waitMs(300, "Vim's Esc timeout (ttimeoutlen, 100 ms) passes")
                .await(VimScenarios::normalMode, "Esc took effect: Normal mode", 5_000)
                .await(r -> lines(r, "The slow brown fox"), "the word changed", 10_000)
                .note("j 0 d2w: delete two words")
                .mutate(r -> type(r, "j0d2w"), "j 0 d2w")
                .await(r -> lines(r, "the lazy dog"), "two words gone", 10_000)
                .note("u: undo, then Ctrl+R: redo")
                .mutate(r -> type(r, "u"), "u")
                .await(r -> lines(r, "jumps over the lazy dog"), "undone", 10_000)
                .mutate(r -> ctrl(r, 'r'), "Ctrl+R")
                .await(r -> lines(r, "the lazy dog"), "redone", 10_000)
                .note("3G f\" ci\" bye Esc: change inside the quotes; $ daw: delete the last word")
                .mutate(r -> {
                    type(r, "3Gf\"ci\"bye");
                    esc(r);
                }, "3G f\" ci\"bye Esc")
                .waitMs(300, "Vim's Esc timeout (ttimeoutlen, 100 ms) passes")
                .await(VimScenarios::normalMode, "Esc took effect: Normal mode", 5_000)
                .mutate(r -> type(r, "$daw"), "$ daw")
                .await(r -> lines(r, "say \"bye\""), "text objects", 10_000)
                .note("gg \"ayy G \"ap: yank line 1 into register a, put it at the end")
                .mutate(r -> type(r, "gg\"ayyG\"ap"), "gg \"ayy G \"ap")
                .await(r -> lines(r, "The slow brown fox", "the lazy dog", "say \"bye\"", "The slow brown fox"), "the line was put", 10_000)
                .note("gg qq A! Esc j q: record a macro; 2@q plays it on the next two lines; . repeats the last change on line 4")
                .mutate(r -> {
                    type(r, "ggqqA!");
                    esc(r);
                }, "gg qq A! Esc")
                .waitMs(300, "Vim's Esc timeout (ttimeoutlen, 100 ms) passes")
                .await(VimScenarios::normalMode, "Esc took effect: Normal mode", 5_000)
                .mutate(r -> type(r, "jq2@q."), "j q, 2@q, .")
                .await(r -> lines(r, "The slow brown fox!", "the lazy dog!", "say \"bye\"!", "The slow brown fox!"), "every line ends in !", 10_000)
                .note(":%s/slow/quick/g")
                .mutate(r -> ex(r, "%s/slow/quick/g"), ":%s/slow/quick/g")
                .await(r -> lines(r, "The quick brown fox!", "the lazy dog!", "say \"bye\"!", "The quick brown fox!"), "substituted", 10_000)
                .note("/lazy Enter ma G 'a dd: search, mark, jump away and back, delete that line")
                .mutate(r -> {
                    type(r, "/lazy");
                    enter(r);
                    type(r, "maG'add");
                }, "/lazy, ma, G, 'a, dd")
                .await(r -> lines(r, "The quick brown fox!", "say \"bye\"!", "The quick brown fox!"), "the marked line is gone", 10_000)
                .note("gg V j U: uppercase two lines in Visual line mode; G 0 v e ~: flip the case of a word in Visual mode")
                .mutate(r -> type(r, "ggVjUG0ve~"), "ggVjU, G0ve~")
                .await(r -> lines(r, "THE QUICK BROWN FOX!", "SAY \"BYE\"!", "tHE quick brown fox!"), "Visual mode changes", 10_000)
                .note("Control: :q with unsaved changes refuses (E37)")
                .mutate(r -> ex(r, "q"), ":q")
                .await(r -> shows(r, "E37: No write since last change"), "Vim refuses to quit", 10_000)
                .note(":w writes the file, :q quits")
                .mutate(r -> ex(r, "w"), ":w")
                .await(r -> shows(r, "\"notes\\.txt\" \\[New\\] 3L, 53B written"), "written", 10_000)
                .mutate(r -> ex(r, "q"), ":q")
                .await(r -> shows(r, LIVE_PROMPT), "back at the shell", 10_000)
                .await(r -> r.computer("pc").programHoldsKeys() ? "Vim still holds the keys" : null,
                        "control: at the shell Esc is the screen's again", 5_000)
                .send("pc", "cat notes.txt")
                .expect("pc", "^THE QUICK BROWN FOX!$", "line 1")
                .expect("pc", "^SAY \"BYE\"!$", "line 2")
                .expect("pc", "^tHE quick brown fox!$", "line 3")
                .expect("pc", PlayerKit.PROMPT, "and nothing else")
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ vim_windows

    private static Scenario windows() {
        return Scenario.builder("vim_windows",
                        "Vim's buffers, split windows, Ctrl+W, a vertical split, tab pages and :only")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Two files to edit")
                .send("pc", "echo alpha > a.txt")
                .expect("pc", PlayerKit.PROMPT, "a.txt")
                .send("pc", "echo beta > b.txt")
                .expect("pc", PlayerKit.PROMPT, "b.txt")
                .note("vim a.txt b.txt: both are buffers; :ls lists them")
                .send("pc", "vim a.txt b.txt")
                .await(r -> lines(r, "alpha"), "a.txt is shown", START_MS)
                .mutate(r -> ex(r, "ls"), ":ls")
                .await(r -> shows(r, "^ +1 %a +\"a\\.txt\" +line 1"), "a.txt is the current buffer", 10_000)
                .await(r -> shows(r, "^ +2 +\"b\\.txt\" +line \\d+"), "b.txt is listed", 10_000)
                .mutate(VimScenarios::enter, "Enter")
                .note(":bn: the next buffer")
                .mutate(r -> ex(r, "bn"), ":bn")
                .await(r -> lines(r, "beta"), "b.txt is shown", 10_000)
                .await(r -> absent(r, "^alpha *$"), "control: a.txt isn't", 1_000)
                .note(":split, then :e a.txt in the top window: one window on each file, each with its status line")
                .mutate(r -> {
                    ex(r, "split");
                    ex(r, "e a.txt");
                }, ":split, :e a.txt")
                .await(r -> lines(r, "alpha") == null && lines(r, "beta") == null ? null : "both files aren't shown",
                        "alpha and beta in two windows", 10_000)
                .await(r -> shows(r, "^a\\.txt +1,1 +All *$") == null && shows(r, "^b\\.txt +1,1 +All *$") == null ? null : "no status lines",
                        "a status line per window", 10_000)
                .note("Ctrl+W j: to the lower window; :vsplit c.txt there, type a line and :w")
                .mutate(r -> {
                    ctrl(r, 'w');
                    type(r, "j");
                    ex(r, "vsplit c.txt");
                    type(r, "inew file");
                    esc(r);
                }, "Ctrl+W j, :vsplit c.txt, i new file Esc")
                .waitMs(300, "Vim's Esc timeout (ttimeoutlen, 100 ms) passes")
                .await(VimScenarios::normalMode, "Esc took effect: Normal mode", 5_000)
                .mutate(r -> ex(r, "w"), ":w")
                .await(r -> shows(r, "\"c\\.txt\" \\[New\\] 1L, 9B written"), "c.txt written", 10_000)
                .await(r -> shows(r, "^new file +\\|beta"), "c.txt and b.txt side by side", 10_000)
                .note(":tabnew: a second tab page (a tab line on top); gt goes back to the first")
                .mutate(r -> ex(r, "tabnew"), ":tabnew")
                .await(r -> shows(r, "\\[No Name\\]"), "the tab line shows the new tab", 10_000)
                .await(r -> absent(r, "^alpha *$"), "the new tab is empty", 2_000)
                .mutate(r -> type(r, "gt"), "gt")
                .await(r -> lines(r, "alpha"), "back on the first tab with its windows", 10_000)
                .note(":only: one window left")
                .mutate(r -> ex(r, "only"), ":only")
                .await(r -> count(r, "^(a|b|c)\\.txt +1,1 +All *$") == 0 ? null : "status lines left",
                        "no status lines with one window", 10_000)
                .note(":qa! quits everything")
                .mutate(r -> ex(r, "qa!"), ":qa!")
                .await(r -> shows(r, LIVE_PROMPT), "back at the shell", 10_000)
                .send("pc", "cat c.txt")
                .expect("pc", "^new file$", "control: c.txt holds what was typed")
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ vim_runtime

    /** The colour (low nibble: foreground) of the cell holding {@code text}'s first character. */
    private static int colourOf(ScenarioRun r, String text) {
        TerminalBlockEntity t = r.terminal("pc");
        var d = t.getDisplay();
        String[] rows = r.screen("pc").split("\n");
        for (int y = 0; y < rows.length; y++) {
            int x = rows[y].indexOf(text);
            if (x >= 0) return d.getAttrAt(x, y) & 0x0F;
        }
        return -1;
    }

    private static Scenario runtime() {
        return Scenario.builder("vim_runtime",
                        "A ~/.vimrc from the computer's files, vi, syntax highlighting, :help and :version; controls: :! and Ctrl+Z")
                .host("pc", PC, "-")
                .asPlayer()
                .note("Write ~/.vimrc (the home folder is the computer's root): line numbers and syntax highlighting")
                .send("pc", "echo set number > .vimrc")
                .expect("pc", PlayerKit.PROMPT, ".vimrc line 1")
                .send("pc", "echo syntax on >> .vimrc")
                .expect("pc", PlayerKit.PROMPT, ".vimrc line 2")
                .send("pc", "echo \"def greet(name): return name\" > hello.py")
                .expect("pc", PlayerKit.PROMPT, "a Python file")
                .note("vi hello.py: vi is Vim; the line number comes from ~/.vimrc")
                .send("pc", "vi hello.py")
                .await(r -> shows(r, "^  1 def greet\\(name\\): return name *$"), "line 1, numbered", START_MS)
                .note(":set ft?  -> filetype=python (filetype.vim from Vim's runtime files)")
                .mutate(r -> ex(r, "set ft?"), ":set ft?")
                .await(r -> shows(r, "filetype=python"), "the filetype", 10_000)
                .note(":echo synIDattr(synID(1,1,1),'name')  -> pythonStatement: 'def' is highlighted")
                .mutate(r -> ex(r, "echo synIDattr(synID(1,1,1),'name')"), ":echo synIDattr(...)")
                .await(r -> shows(r, "^pythonStatement "), "the syntax group", 10_000)
                .await(r -> {
                    int def = colourOf(r, "def greet"), name = colourOf(r, "greet(");
                    return def >= 0 && name >= 0 && def != name ? null : "def " + def + ", greet " + name;
                }, "'def' is drawn in another colour than 'greet'", 5_000)
                .note(":help opens the main help file; :q closes it")
                .mutate(r -> ex(r, "help"), ":help")
                .await(r -> shows(r, "VIM - main help file"), "help.txt", 10_000)
                .mutate(r -> ex(r, "q"), ":q")
                .await(r -> absent(r, "VIM - main help file"), "help closed", 10_000)
                .note(":version names the port")
                .mutate(r -> ex(r, "version"), ":version")
                .await(r -> shows(r, "Modified by Evans Computer Mod \\(WASI port\\)"), "the version message", 10_000)
                .mutate(r -> {
                    type(r, "q");
                    esc(r);
                }, "q (the more-prompt), Esc")
                .note("Control: :!ls runs no shell command, it says so")
                .mutate(r -> ex(r, "!ls"), ":!ls")
                .await(r -> shows(r, "E9999: Shell commands are not available on this computer"), "the shell error", 10_000)
                .mutate(VimScenarios::enter, "Enter")
                .note("Control: Ctrl+Z doesn't suspend Vim, it says so")
                .mutate(r -> ctrl(r, 'z'), "press Ctrl+Z")
                .await(r -> shows(r, "E9998: Vim can't be suspended on this computer"), "the suspend error", 10_000)
                .await(r -> absent(r, "Stopped"), "control: no stopped job", 1_000)
                .mutate(r -> ex(r, "q"), ":q")
                .await(r -> shows(r, LIVE_PROMPT), "back at the shell", 10_000)
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ vim_resize

    private static String panel(ScenarioRun r, int cols, int rows, PanelTier tier) {
        TerminalBlockEntity t = r.terminal("pc");
        if (t == null) return "no terminal";
        if (t.getPanelTier() != tier) return "panel is " + t.getPanelTier();
        var d = t.getDisplay();
        return d.getWidth() == cols && d.getHeight() == rows ? null : "console is " + d.getWidth() + "x" + d.getHeight();
    }

    private static Scenario resize() {
        return Scenario.builder("vim_resize",
                        "Vim follows the terminal's size: a panel swap from 160x50 to 80x25 redraws it at the new size")
                .host("pc", PC, "-")
                .asPlayer()
                .note("A Terminal from a plain item is Tier 2: 160x50")
                .await(r -> panel(r, 160, 50, PanelTier.COLOR_LCD), "160x50", 10_000)
                .send("pc", "vim size.txt")
                .await(r -> shows(r, "\"size\\.txt\" \\[New\\]"), "Vim is open", START_MS)
                .mutate(r -> {
                    type(r, "iresize me");
                    esc(r);
                }, "i resize me Esc")
                .waitMs(300, "Vim's Esc timeout (ttimeoutlen, 100 ms) passes")
                .await(VimScenarios::normalMode, "Esc took effect: Normal mode", 5_000)
                .mutate(r -> ex(r, "echo &columns .. 'x' .. &lines"), ":echo &columns .. 'x' .. &lines")
                .await(r -> shows(r, "^160x50 "), "control: Vim is 160x50 before the swap", 10_000)
                .note("Right-click the screen with a Phosphor panel: the console becomes 80x25")
                .mutate(r -> r.player().use(ModItems.PHOSPHOR_PANEL.get(), r.abs(PC), Direction.NORTH, false), "install the Phosphor panel")
                .await(r -> panel(r, 80, 25, PanelTier.PHOSPHOR), "80x25", 10_000)
                .note("Vim got the new size (the terminal's resize report) and redrew: the text is still there")
                .await(r -> lines(r, "resize me"), "the buffer redrawn", 10_000)
                .mutate(r -> ex(r, "echo &columns .. 'x' .. &lines"), ":echo &columns .. 'x' .. &lines")
                .await(r -> shows(r, "^80x25 "), "Vim is 80x25", 10_000)
                .mutate(r -> ex(r, "q!"), ":q!")
                .await(r -> shows(r, LIVE_PROMPT), "back at the shell", 10_000)
                .timeLimit(60_000)
                .build();
    }
}
//?}
