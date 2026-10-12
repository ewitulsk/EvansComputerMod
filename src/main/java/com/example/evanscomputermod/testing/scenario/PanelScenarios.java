package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.item.ModItems;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.ItemLike;
import net.minecraft.world.phys.Vec3;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * Display panels and tiered Screens (docs/gui/ROADMAP.md P1), built and
 * operated by a player ({@link ScenarioPlayer}): the same definitions run as
 * GameTests ({@code ecm_panel}) and from {@code /ecm scenario spawn}.
 *
 * <ul>
 *   <li>{@code panel_swap}: a running Terminal (placed from a plain item:
 *       Tier 2, 160x50) gets a Phosphor panel (80x25), then an HD panel
 *       (160x50), by right-clicking its screen; the shell resizes with no
 *       reboot (same computer instance, commands still run). Sneak-right-click
 *       with an empty hand takes the panel out: the computer keeps running
 *       headless (control: a command still runs).</li>
 *   <li>{@code panel_wall}: a 2x2 Screen wall of three HD Screens and one
 *       Phosphor Screen runs at Tier 1 (2 x 64 by 2 x 36 = 128x72) and names
 *       the Phosphor Screen as holding it back; right-clicking the wall says so.
 *       Swapping that block for an HD Screen lifts the wall to Tier 3 (512x288,
 *       nothing holding it back).</li>
 *   <li>{@code panel_touch}: a 2x1 HD wall; {@code mousetest screen} captures
 *       the pointer on the Screen; a click on the wall's top-left block, a
 *       quarter in from its top-left corner, arrives as {@code mouse_poll}
 *       events at pixel (64, 36) of the 512x144 wall, marked as a Screen touch.
 *       Control: a sneaking click is not a touch.</li>
 * </ul>
 */
public final class PanelScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    /** The computer, facing north. Screens go on its west side, facing west. */
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos S00 = new BlockPos(-1, 1, 0);   // touches the computer
    private static final BlockPos S01 = new BlockPos(-1, 1, 1);
    private static final BlockPos S10 = new BlockPos(-1, 2, 0);
    private static final BlockPos S11 = new BlockPos(-1, 2, 1);

    /** The computer instance seen before the swaps (a reboot would replace it). */
    private static final Map<ScenarioRun, ComputerInstance> BOOTED = new WeakHashMap<>();

    static {
        for (Scenario s : List.of(swap(), wall(), touch())) ALL.put(s.name, s);
    }

    private PanelScenarios() {
    }

    // ------------------------------------------------------------ panel_swap

    private static Scenario swap() {
        return Scenario.builder("panel_swap",
                        "swap display panels on a running Terminal: the shell resizes without a reboot")
                .host("pc", PC, "-")
                .asPlayer()
                .note("A Terminal from a plain item is Tier 2 (Color LCD): 160x50")
                .await(r -> grid(r, 160, 50, PanelTier.COLOR_LCD), "Tier 2 panel, 160x50 console", 10_000)
                .mutate(r -> BOOTED.put(r, r.terminal("pc").getComputer()), "remember the running computer")
                .note("Right-click the screen with a Phosphor panel (Tier 1)")
                .mutate(r -> usePanel(r, ModItems.PHOSPHOR_PANEL.get()), "install the Phosphor panel")
                .await(r -> grid(r, 80, 25, PanelTier.PHOSPHOR), "the console is 80x25", 10_000)
                .send("pc", "echo tier1_shell_alive")
                .expect("pc", "^tier1_shell_alive", "the shell still runs at 80x25")
                .note("Swap in an HD panel (Tier 3)")
                .mutate(r -> usePanel(r, ModItems.HD_PANEL.get()), "install the HD panel")
                .await(r -> grid(r, 160, 50, PanelTier.HD), "the console is 160x50 again", 10_000)
                .send("pc", "echo tier3_shell_alive")
                .expect("pc", "^tier3_shell_alive", "the shell still runs at 160x50")
                .note("Sneak-right-click the screen with an empty hand: the panel comes out, headless")
                .mutate(r -> r.player().useEmpty(r.abs(PC), Direction.NORTH, true), "take the panel out")
                .await(r -> r.terminal("pc").getPanelTier() == null ? sameComputer(r) : "the panel is still in",
                        "no panel, same computer", 5_000)
                .send("pc", "echo headless_alive")
                .expect("pc", "^headless_alive", "the headless computer keeps running (control)")
                .timeLimit(60_000)
                .build();
    }

    private static void usePanel(ScenarioRun r, ItemLike panel) {
        r.player().use(panel, r.abs(PC), Direction.NORTH, false);
    }

    /** Null when the console is {@code cols x rows} at {@code tier} on the computer that booted. */
    private static String grid(ScenarioRun r, int cols, int rows, PanelTier tier) {
        TerminalBlockEntity t = r.terminal("pc");
        if (t == null) return "no terminal";
        if (t.getPanelTier() != tier) return "panel is " + t.getPanelTier();
        var d = t.getDisplay();
        if (d.getWidth() != cols || d.getHeight() != rows) return "console is " + d.getWidth() + "x" + d.getHeight();
        return sameComputer(r);
    }

    private static String sameComputer(ScenarioRun r) {
        ComputerInstance before = BOOTED.get(r);
        ComputerInstance now = r.terminal("pc").getComputer();
        if (now == null) return "the computer is off";
        return before == null || before == now ? null : "the computer was rebooted";
    }

    // ------------------------------------------------------------ panel_wall

    private static Scenario wall() {
        return Scenario.builder("panel_wall",
                        "a mixed 2x2 Screen wall runs at its lowest tier and names the block holding it back")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(new Wall(List.of(S00, S01, S10), List.of(S11)))
                .note("Three HD Screens and one Phosphor Screen: the wall runs at Tier 1")
                .await(r -> wallIs(r, PanelTier.PHOSPHOR, 128, 72, r.abs(S11)), "Tier 1 wall, 128x72, held back by the Phosphor Screen", 10_000)
                .mutate(r -> r.player().useEmpty(r.abs(S00), Direction.WEST, false), "right-click the wall")
                .await(r -> said(r, "held back by the Phosphor Screen"), "the wall names the Phosphor Screen", 5_000)
                .note("Replace the Phosphor Screen with an HD Screen: the wall is Tier 3")
                .mutate(r -> {
                    r.player().breakBlock(r.abs(S11), Direction.WEST);
                    r.player().place(ModBlocks.HD_SCREEN.get(), r.abs(S11), Direction.WEST,
                            s -> s.getValue(ScreenBlock.FACING) == Direction.WEST);
                }, "swap the Phosphor Screen for an HD Screen")
                .await(r -> wallIs(r, PanelTier.HD, 512, 288, null), "Tier 3 wall, 512x288, nothing holding it back", 10_000)
                .timeLimit(60_000)
                .build();
    }

    private static String wallIs(ScenarioRun r, PanelTier tier, int w, int h, BlockPos limitedBy) {
        TerminalBlockEntity t = r.terminal("pc");
        var info = t == null ? null : t.getScreenClusterInfo();
        if (info == null) return "no wall";
        if (info.cols() != 2 || info.rows() != 2) return "wall is " + info.cols() + "x" + info.rows() + " blocks";
        if (info.tier() != tier) return "wall runs at " + info.tier();
        if (info.gfxWidth() != w || info.gfxHeight() != h) return "wall is " + info.gfxWidth() + "x" + info.gfxHeight();
        if (!java.util.Objects.equals(info.limitedBy(), limitedBy)) return "held back by " + info.limitedBy();
        var mode = t.getComputer() != null ? t.getComputer().bridgeGfxModes(ComputerInstance.GFX_TARGET_SCREEN) : null;
        if (mode == null || mode[0] != tier.tier() || mode[3] != w || mode[4] != h) {
            return "gfx_modes reports " + (mode == null ? "nothing" : java.util.Arrays.toString(mode));
        }
        return null;
    }

    private static String said(ScenarioRun r, String text) {
        for (String s : r.player().heard()) if (s.contains(text)) return null;
        return "heard " + r.player().heard();
    }

    /** Screens placed by the player against the computer's west side, bottom row first. */
    private record Wall(List<BlockPos> hd, List<BlockPos> phosphor) implements Scenario.Decor {
        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new java.util.ArrayList<>(hd);
            all.addAll(phosphor);
            return all;
        }

        @Override
        public void build(ScenarioRun run) {
            List<BlockPos> order = new java.util.ArrayList<>(footprint());
            order.sort(java.util.Comparator.comparingInt(BlockPos::getY));
            for (BlockPos p : order) {
                ItemLike item = phosphor.contains(p) ? ModBlocks.PHOSPHOR_SCREEN.get() : ModBlocks.HD_SCREEN.get();
                run.player().place(item, run.abs(p), Direction.WEST, s -> s.getValue(ScreenBlock.FACING) == Direction.WEST);
            }
        }
    }

    // ------------------------------------------------------------ panel_touch

    private static Scenario touch() {
        return Scenario.builder("panel_touch",
                        "click a Tier 3 Screen wall: the program gets mouse_poll events at that pixel")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(new Wall(List.of(S00, S01), List.of()))
                .await(r -> {
                    var info = r.terminal("pc").getScreenClusterInfo();
                    return info != null && info.tier() == PanelTier.HD && info.gfxWidth() == 512 && info.gfxHeight() == 144
                            ? null : "wall " + info;
                }, "a 2x1 HD wall at 512x144", 10_000)
                .note("Control: a sneaking click on the wall is not a touch")
                .send("pc", "mousetest screen")
                .expect("pc", "touch the Screen \\(512x144\\)", "mousetest owns the Screen and captured the pointer",
                        "mousetest: ")
                .mutate(r -> click(r, true, 0.75, 0.75), "sneak-click the wall (would be pixel 192, 108)")
                .mutate(r -> click(r, false, 0.25, 0.25), "click the wall a quarter in from its top-left corner")
                .expect("pc", "^touch down 64 36 buttons=0x81", "a touch-down at pixel (64, 36), marked from the Screen",
                        "^touch down (?!64 36 )")
                .expect("pc", "^touch up 64 36 buttons=0x80", "and the touch-up")
                .timeLimit(60_000)
                .build();
    }

    /** Right-click the west face of the wall's top-left block, ({@code fu}, {@code fv}) in from that corner. */
    private static void click(ScenarioRun r, boolean sneaking, double fu, double fv) {
        BlockPos b = r.abs(S00);
        // Facing west, u runs south (+z): the top-left block is S00 (lowest z, the only row).
        Vec3 hit = new Vec3(b.getX(), b.getY() + 1 - fv, b.getZ() + fu);
        r.player().use(net.minecraft.world.item.ItemStack.EMPTY, b, Direction.WEST, hit, sneaking);
    }
}
//?}
