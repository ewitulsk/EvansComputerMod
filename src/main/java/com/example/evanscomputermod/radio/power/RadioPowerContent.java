package com.example.evanscomputermod.radio.power;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.radio.RadioConfig;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.common.conditions.ICondition;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;
import net.neoforged.neoforge.registries.NeoForgeRegistries;

/** Radio power hardware: the Burner Generator (amplifiers register here too). */
public final class RadioPowerContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MapCodec<? extends ICondition>> CONDITIONS =
            DeferredRegister.create(NeoForgeRegistries.Keys.CONDITION_CODECS, EvansComputerMod.MODID);

    public static final DeferredBlock<BurnerGeneratorBlock> BURNER_GENERATOR = BLOCKS.registerBlock("burner_generator",
            BurnerGeneratorBlock::new,
            BlockBehaviour.Properties.of().strength(3.5f).sound(SoundType.METAL).requiresCorrectToolForDrops()
                    .lightLevel(s -> s.getValue(BurnerGeneratorBlock.LIT) ? 13 : 0));
    public static final DeferredItem<BlockItem> BURNER_GENERATOR_ITEM = ITEMS.registerSimpleBlockItem("burner_generator", BURNER_GENERATOR);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<BurnerGeneratorBlockEntity>> BURNER_GENERATOR_BE =
            BLOCK_ENTITIES.register("burner_generator", () -> BlockEntityType.Builder
                    .of(BurnerGeneratorBlockEntity::new, BURNER_GENERATOR.get()).build(null));

    public static final DeferredHolder<MapCodec<? extends ICondition>, MapCodec<ConfigEnabledCondition>> FEATURE_ENABLED =
            CONDITIONS.register("radio_feature_enabled", () -> ConfigEnabledCondition.CODEC);

    private RadioPowerContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        CONDITIONS.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(Capabilities.EnergyStorage.BLOCK, BURNER_GENERATOR_BE.get(), (be, side) -> be.energy());
            e.registerBlockEntity(Capabilities.ItemHandler.BLOCK, BURNER_GENERATOR_BE.get(), (be, side) -> be.fuel());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            // Left out of the tab when disabled (it is also hidden from recipe viewers by having no recipe).
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey() && RadioConfig.burnerGeneratorEnabled())
                e.accept(BURNER_GENERATOR_ITEM.get());
        });
    }
}
//?}
