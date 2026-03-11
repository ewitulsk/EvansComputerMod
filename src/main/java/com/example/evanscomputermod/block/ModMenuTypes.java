package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.inventory.MenuType;
import net.neoforged.neoforge.common.extensions.IMenuTypeExtension;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Registry for all menu types in the mod.
 */
public class ModMenuTypes {
    
    public static final DeferredRegister<MenuType<?>> MENUS =
            DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);
    
    public static final DeferredHolder<MenuType<?>, MenuType<TerminalMenu>> TERMINAL_MENU =
            MENUS.register("terminal_menu", () ->
                    IMenuTypeExtension.create(TerminalMenu::new)
            );
}
