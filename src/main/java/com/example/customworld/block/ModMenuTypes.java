package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import net.minecraft.world.inventory.MenuType;
import net.minecraftforge.common.extensions.IForgeMenuType;
import net.minecraftforge.registries.DeferredRegister;
import net.minecraftforge.registries.ForgeRegistries;
import net.minecraftforge.registries.RegistryObject;

/**
 * Registry for all menu types in the mod.
 */
public class ModMenuTypes {

    public static final DeferredRegister<MenuType<?>> MENUS =
            DeferredRegister.create(ForgeRegistries.MENU_TYPES, CustomWorldMod.MODID);

    public static final RegistryObject<MenuType<TerminalMenu>> TERMINAL_MENU =
            MENUS.register("terminal_menu", () ->
                    IForgeMenuType.create(TerminalMenu::new)
            );
}
