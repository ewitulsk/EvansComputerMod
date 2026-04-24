package com.example.evanscomputermod.item;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.missile.MissileTier;
//? if >=26.1 {
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
//?}
import net.minecraft.world.item.Item;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/** Registry for standalone (non-block) items. */
public class ModItems {

    public static final DeferredRegister.Items ITEMS =
            DeferredRegister.createItems(EvansComputerMod.MODID);

    //? if >=26.1 {
    public static final DeferredItem<MissileItem> MISSILE_MRBM =
            ITEMS.register("missile_mrbm", id -> new MissileItem(MissileTier.MRBM,
                    new Item.Properties().stacksTo(1).setId(ResourceKey.create(Registries.ITEM, id))));

    public static final DeferredItem<MissileItem> MISSILE_LRBM =
            ITEMS.register("missile_lrbm", id -> new MissileItem(MissileTier.LRBM,
                    new Item.Properties().stacksTo(1).setId(ResourceKey.create(Registries.ITEM, id))));

    public static final DeferredItem<MissileItem> MISSILE_ICBM =
            ITEMS.register("missile_icbm", id -> new MissileItem(MissileTier.ICBM,
                    new Item.Properties().stacksTo(1).setId(ResourceKey.create(Registries.ITEM, id))));
    //?} else {
    /*public static final DeferredItem<MissileItem> MISSILE_MRBM =
            ITEMS.register("missile_mrbm",
                    () -> new MissileItem(MissileTier.MRBM, new Item.Properties().stacksTo(1)));

    public static final DeferredItem<MissileItem> MISSILE_LRBM =
            ITEMS.register("missile_lrbm",
                    () -> new MissileItem(MissileTier.LRBM, new Item.Properties().stacksTo(1)));

    public static final DeferredItem<MissileItem> MISSILE_ICBM =
            ITEMS.register("missile_icbm",
                    () -> new MissileItem(MissileTier.ICBM, new Item.Properties().stacksTo(1)));*/
    //?}
}
