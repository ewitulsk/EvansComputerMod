package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/** Registration of the Speaker block, its item and block entity. */
public final class SpeakerContent {

    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    public static final DeferredBlock<SpeakerBlock> SPEAKER =
            BLOCKS.registerBlock("speaker", SpeakerBlock::new,
                    BlockBehaviour.Properties.of().strength(1.5f, 3.0f).sound(SoundType.WOOD));

    public static final DeferredItem<BlockItem> SPEAKER_ITEM = ITEMS.registerSimpleBlockItem("speaker", SPEAKER);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<SpeakerBlockEntity>> SPEAKER_BE =
            BLOCK_ENTITIES.register("speaker", () ->
                    //? if >=26.1 {
                    new BlockEntityType<>(SpeakerBlockEntity::new, SPEAKER.get())
                    //?} else
                    /*BlockEntityType.Builder.<SpeakerBlockEntity>of(SpeakerBlockEntity::new, SPEAKER.get()).build(null)*/
            );

    private SpeakerContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) ->
                e.registerBlockEntity(PeripheralCapability.PERIPHERAL, SPEAKER_BE.get(), (be, side) -> be.getPeripheral()));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(SPEAKER_ITEM.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() ->
                PeripheralTypes.register(SpeakerPeripheral.TYPE, "Speaker: PCM audio through /dev/audio",
                        SpeakerPeripheral.class)));
    }
}
