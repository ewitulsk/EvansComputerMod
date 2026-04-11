package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Registers a custom creative mode tab for the mod.
 */
public class ModCreativeTabs {

    public static final DeferredRegister<CreativeModeTab> CREATIVE_TABS =
            DeferredRegister.create(Registries.CREATIVE_MODE_TAB, EvansComputerMod.MODID);

    public static final DeferredHolder<CreativeModeTab, CreativeModeTab> TAB =
            CREATIVE_TABS.register("tab", () -> CreativeModeTab.builder()
                    .title(Component.translatable("itemGroup." + EvansComputerMod.MODID))
                    .icon(() -> new ItemStack(ModBlocks.TERMINAL_BLOCK.get()))
                    .displayItems((params, output) -> {
                        output.accept(ModBlocks.TERMINAL_BLOCK_ITEM.get());
                        output.accept(ModBlocks.SWITCH_BLOCK_ITEM.get());
                        output.accept(ModBlocks.NETWORK_CABLE_ITEM.get());
                        output.accept(ModBlocks.INTERFACE_BLOCK_ITEM.get());
                        output.accept(ModBlocks.INTERNET_GATEWAY_ITEM.get());
                    })
                    .build()
            );
}
