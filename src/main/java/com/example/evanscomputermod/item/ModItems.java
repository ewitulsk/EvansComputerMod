package com.example.evanscomputermod.item;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.world.item.Item;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Registry for non-block items in the mod.
 */
public class ModItems {

    public static final DeferredRegister.Items ITEMS =
            DeferredRegister.createItems(EvansComputerMod.MODID);

    public static final DeferredItem<Item> INTERFACE_PROBE =
            ITEMS.registerItem("interface_probe",
                    props -> new InterfaceProbeItem(props.stacksTo(1)));

    /** Opens a module bay on a computer (two per computer). */
    public static final DeferredItem<Item> MODULE_EXPANSION_CARD =
            ITEMS.registerItem("module_expansion_card",
                    props -> new ModuleExpansionCardItem(props.stacksTo(16)));

    /** Wireless Xbox Controller: keyboard keys -> controller buttons for a paired computer. */
    public static final DeferredItem<Item> WIRELESS_CONTROLLER =
            ITEMS.registerItem("wireless_controller",
                    props -> new com.example.evanscomputermod.controller.WirelessControllerItem(props.stacksTo(1)));
    public static final DeferredItem<Item> ALWAYS_ON_MODULE = ITEMS.registerItem("always_on_module",
            props -> new com.example.evanscomputermod.module.AlwaysOnModuleItem(props.stacksTo(16)));

    // Display panels (computer/display/PanelTier): one item per tier, plus the
    // amber colour of the Tier 1 phosphor panel.
    public static final DeferredItem<DisplayPanelItem> PHOSPHOR_PANEL = ITEMS.registerItem("phosphor_panel",
            props -> new DisplayPanelItem(props.stacksTo(16),
                    com.example.evanscomputermod.computer.display.PanelTier.PHOSPHOR,
                    com.example.evanscomputermod.computer.display.PanelTier.Tone.GREEN));
    public static final DeferredItem<DisplayPanelItem> AMBER_PHOSPHOR_PANEL = ITEMS.registerItem("amber_phosphor_panel",
            props -> new DisplayPanelItem(props.stacksTo(16),
                    com.example.evanscomputermod.computer.display.PanelTier.PHOSPHOR,
                    com.example.evanscomputermod.computer.display.PanelTier.Tone.AMBER));
    public static final DeferredItem<DisplayPanelItem> COLOR_LCD_PANEL = ITEMS.registerItem("color_lcd_panel",
            props -> new DisplayPanelItem(props.stacksTo(16),
                    com.example.evanscomputermod.computer.display.PanelTier.COLOR_LCD,
                    com.example.evanscomputermod.computer.display.PanelTier.Tone.GREEN));
    public static final DeferredItem<DisplayPanelItem> HD_PANEL = ITEMS.registerItem("hd_panel",
            props -> new DisplayPanelItem(props.stacksTo(16),
                    com.example.evanscomputermod.computer.display.PanelTier.HD,
                    com.example.evanscomputermod.computer.display.PanelTier.Tone.GREEN));
}
