package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.PanelVisual;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.module.InstalledPanel;
import com.example.evanscomputermod.module.ModDataComponents;
import com.example.evanscomputermod.testing.scenario.PanelScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Display panels and tiered Screens, namespace {@code ecm_panel}
 * (docs/gui/ROADMAP.md P1 "Done when"). The scenario tests are the same
 * definitions {@code /ecm scenario spawn panel_*} builds (PanelScenarios).
 */
@GameTestHolder(PanelTests.NS)
@PrefixGameTestTemplate(false)
public final class PanelTests {
    static final String NS = "ecm_panel";
    private static final String STRUCTURE = "gametest_empty";

    /** Swapping T2 -> T1 -> T3 on a running Terminal resizes the shell with no reboot. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".swap")
    public static void panel_swap(GameTestHelper h) {
        TestDriver.scenario(h, NS, PanelScenarios.ALL.get("panel_swap"), true);
    }

    /** A 2x2 wall of mixed tiers reports the lowest tier's resolution and names the block holding it back. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wall")
    public static void panel_wall(GameTestHelper h) {
        TestDriver.scenario(h, NS, PanelScenarios.ALL.get("panel_wall"), true);
    }

    /** Clicking a Tier 3 wall delivers a mouse_poll event with the right pixel. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".touch")
    public static void panel_touch(GameTestHelper h) {
        TestDriver.scenario(h, NS, PanelScenarios.ALL.get("panel_touch"), true);
    }

    /**
     * Old worlds: a Terminal saved before panels (no {@code panel} tag) loads
     * at Tier 2 and a pre-tier Screen ({@code screen_block}) is Tier 2. A saved
     * empty slot stays headless (control). A broken Terminal's item carries its
     * panel, and the new Terminal recipe crafts one with a Tier 1 panel.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".migration")
    public static void old_world_is_tier2(GameTestHelper h) {
        BlockPos pos = new BlockPos(2, 2, 2);
        h.setBlock(pos, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.NORTH));
        String[] failure = {null};
        boolean[] done = {false};
        TestDriver.drive(h, NS, "old_world_is_tier2", () -> {
            if (done[0]) return true;
            try {
                var reg = h.getLevel().registryAccess();
                TerminalBlockEntity te = (TerminalBlockEntity) h.getBlockEntity(pos);
                CompoundTag saved = te.saveWithoutMetadata(reg);
                check(saved.contains("panel"), "a Terminal saves its panel");

                // Pre-panel save: no "panel" tag.
                CompoundTag old = saved.copy();
                old.remove("panel");
                te.installPanel(new ItemStack(ModItems.HD_PANEL.get()));
                te.loadWithComponents(old, reg);
                check(te.getPanelTier() == PanelTier.COLOR_LCD, "an old Terminal should load as Tier 2, got " + te.getPanelTier());
                check(ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().getValue(TerminalBlock.PANEL) == PanelVisual.COLOR_LCD,
                        "an old block state (no panel property) should show the Tier 2 panel");

                // Control: an empty slot saved as such stays empty.
                te.installPanel(ItemStack.EMPTY);
                CompoundTag headless = te.saveWithoutMetadata(reg);
                te.installPanel(new ItemStack(ModItems.PHOSPHOR_PANEL.get()));
                te.loadWithComponents(headless, reg);
                check(te.getPanelTier() == null, "a saved headless Terminal must stay headless");

                // Screens: the pre-tier block is Tier 2, the new ones 1 and 3.
                check(ScreenBlock.tierOf(ModBlocks.SCREEN_BLOCK.get().defaultBlockState()) == PanelTier.COLOR_LCD, "screen_block is Tier 2");
                check(ScreenBlock.tierOf(ModBlocks.PHOSPHOR_SCREEN.get().defaultBlockState()) == PanelTier.PHOSPHOR, "phosphor_screen is Tier 1");
                check(ScreenBlock.tierOf(ModBlocks.HD_SCREEN.get().defaultBlockState()) == PanelTier.HD, "hd_screen is Tier 3");

                // The panel rides on the dropped item.
                te.installPanel(new ItemStack(ModItems.AMBER_PHOSPHOR_PANEL.get()));
                var drops = net.minecraft.world.level.block.Block.getDrops(h.getLevel().getBlockState(h.absolutePos(pos)),
                        h.getLevel(), h.absolutePos(pos), te);
                InstalledPanel carried = drops.isEmpty() ? null : drops.get(0).get(ModDataComponents.INSTALLED_PANEL.get());
                check(carried != null && carried.panel().is(ModItems.AMBER_PHOSPHOR_PANEL.get()),
                        "the dropped Terminal should carry its amber panel, got " + carried);
                check(h.getLevel().getBlockState(h.absolutePos(pos)).getValue(TerminalBlock.PANEL) == PanelVisual.AMBER,
                        "the block shows the amber panel");

                // New Terminals are crafted with a Tier 1 panel.
                var recipe = h.getLevel().getRecipeManager()
                        .byKey(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_block"));
                check(recipe.isPresent(), "no terminal_block recipe");
                ItemStack result = recipe.get().value().getResultItem(reg);
                InstalledPanel crafted = result.get(ModDataComponents.INSTALLED_PANEL.get());
                check(crafted != null && crafted.panel().is(ModItems.PHOSPHOR_PANEL.get()),
                        "a crafted Terminal should come with a Phosphor panel, got " + crafted);
                done[0] = true;
                return true;
            } catch (AssertionError | RuntimeException e) {
                failure[0] = e.toString();
                return false;
            }
        }, () -> failure[0]);
    }

    private static void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }
}
//?}
