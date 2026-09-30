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
    public static final DeferredItem<Item> ALWAYS_ON_MODULE = ITEMS.registerItem("always_on_module",
            props -> new com.example.evanscomputermod.module.AlwaysOnModuleItem(props.stacksTo(16)));
}
