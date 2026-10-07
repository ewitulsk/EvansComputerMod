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
}
