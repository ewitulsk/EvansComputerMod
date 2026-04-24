package com.example.evanscomputermod.entity;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.MobCategory;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

public class ModEntities {

    public static final DeferredRegister<EntityType<?>> ENTITIES =
            DeferredRegister.create(Registries.ENTITY_TYPE, EvansComputerMod.MODID);

    public static final DeferredHolder<EntityType<?>, EntityType<MissileEntity>> MISSILE =
            ENTITIES.register("missile", () ->
                    EntityType.Builder.<MissileEntity>of(MissileEntity::new, MobCategory.MISC)
                            .sized(0.5f, 1.5f)
                            .clientTrackingRange(128)
                            .updateInterval(1)
                            //? if >=26.1 {
                            .build(ResourceKey.create(Registries.ENTITY_TYPE, EvansComputerMod.id("missile")))
                            //?} else
                            /*.build(EvansComputerMod.id("missile").toString())*/
            );
}
