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
import com.example.evanscomputermod.module.InstalledModules;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import net.minecraft.world.level.block.state.BlockState;
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

    /**
     * The Terminal recipe takes any display panel and the crafted Terminal
     * carries that panel (control: a glass pane in the panel slot crafts
     * nothing). A Terminal item's model state ({@link TerminalBlock#itemState},
     * what the item renders) equals the state it places as: its panel, its bay
     * cards and its modules; an item without a panel shows the Tier 2 default.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".item")
    public static void terminal_item_matches_placed(GameTestHelper h) {
        String[] failure = {null};
        TestDriver.drive(h, NS, "terminal_item_matches_placed", () -> {
            try {
                var level = h.getLevel();
                BlockState def = ModBlocks.TERMINAL_BLOCK.get().defaultBlockState();
                var panels = java.util.List.of(ModItems.PHOSPHOR_PANEL.get(), ModItems.AMBER_PHOSPHOR_PANEL.get(),
                        ModItems.COLOR_LCD_PANEL.get(), ModItems.HD_PANEL.get());
                java.util.List<ItemStack> crafted = new java.util.ArrayList<>();
                for (var panel : panels) {
                    ItemStack out = craftTerminal(level, new ItemStack(panel));
                    InstalledPanel p = out.get(ModDataComponents.INSTALLED_PANEL.get());
                    check(out.is(ModBlocks.TERMINAL_BLOCK.get().asItem()) && p != null && p.panel().is(panel),
                            "a Terminal crafted with " + panel + " should carry it, got " + out + " " + p);
                    check(TerminalBlock.itemState(def, out).getValue(TerminalBlock.PANEL) == PanelVisual.of(new ItemStack(panel)),
                            "the crafted Terminal item should show " + panel);
                    crafted.add(out);
                }
                check(craftTerminal(level, new ItemStack(net.minecraft.world.item.Items.GLASS_PANE)).isEmpty(),
                        "control: a glass pane in the panel slot must not craft a Terminal");
                check(TerminalBlock.itemState(def, new ItemStack(ModBlocks.TERMINAL_BLOCK.get())).getValue(TerminalBlock.PANEL)
                        == PanelVisual.COLOR_LCD, "an item without a panel shows the Tier 2 panel it places with");

                // A Terminal with an HD panel, a left bay card and a Wired Sensor Module in its first slot.
                ItemStack stack = crafted.get(3).copy();
                java.util.List<ItemStack> slots = new java.util.ArrayList<>(java.util.Collections.nCopies(4, ItemStack.EMPTY));
                slots.set(0, new ItemStack(com.example.evanscomputermod.sensor.SensorContent.WIRED_SENSOR_MODULE.get()));
                stack.set(ModDataComponents.INSTALLED_MODULES.get(), new InstalledModules(1 << ModuleBays.LEFT, slots));
                BlockState shown = TerminalBlock.itemState(def, stack);
                check(shown.getValue(TerminalBlock.LEFT_BAY) && !shown.getValue(TerminalBlock.RIGHT_BAY), "the item shows its left bay card");
                check(shown.getValue(TerminalBlock.SLOTS[0]) == ModuleSlotVisual.WIRED_SENSOR, "the item shows its module");
                check(shown.getValue(TerminalBlock.SLOTS[2]) == ModuleSlotVisual.EMPTY, "control: an empty slot shows nothing");

                // Placed the way a BlockItem places it: the block shows the same parts.
                for (ItemStack item : java.util.List.of(stack, crafted.get(1), new ItemStack(ModBlocks.TERMINAL_BLOCK.get()))) {
                    BlockPos pos = h.absolutePos(new BlockPos(2, 2, 2));
                    level.setBlock(pos, def, net.minecraft.world.level.block.Block.UPDATE_ALL);
                    def.getBlock().setPlacedBy(level, pos, level.getBlockState(pos), null, item);
                    BlockState placed = level.getBlockState(pos);
                    check(placed == TerminalBlock.itemState(def, item),
                            "placed " + placed + " differs from the item's " + TerminalBlock.itemState(def, item));
                    level.removeBlock(pos, false);
                }
                return true;
            } catch (AssertionError | RuntimeException e) {
                failure[0] = e.toString();
                return false;
            }
        }, () -> failure[0]);
    }

    private static ItemStack craftTerminal(net.minecraft.server.level.ServerLevel level, ItemStack panel) {
        ItemStack i = new ItemStack(net.minecraft.world.item.Items.IRON_INGOT);
        var input = net.minecraft.world.item.crafting.CraftingInput.of(3, 3, java.util.List.of(
                i, panel, i,
                i, new ItemStack(net.minecraft.world.item.Items.REDSTONE_BLOCK), i,
                i, new ItemStack(net.minecraft.world.item.Items.GOLD_BLOCK), i));
        return level.getRecipeManager().getRecipeFor(net.minecraft.world.item.crafting.RecipeType.CRAFTING, input, level)
                .map(r -> r.value().assemble(input, level.registryAccess()))
                .orElse(ItemStack.EMPTY);
    }

    private static void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }
}
//?}
