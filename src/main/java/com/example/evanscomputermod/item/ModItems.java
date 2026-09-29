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
}
