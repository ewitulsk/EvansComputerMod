package com.example.evanscomputermod.radio.sdr;

//? if <=1.21.1 {
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

/** The three SDR blocks and their shared block entity. */
public final class RadioSdrContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    private static BlockBehaviour.Properties props() {
        return BlockBehaviour.Properties.of().strength(1.5f, 3f).sound(SoundType.METAL).noOcclusion();
    }

    public static final DeferredBlock<SdrBlock> SDR_BASIC = BLOCKS.registerBlock("sdr_basic", p -> new SdrBlock(SdrTier.BASIC, p), props());
    public static final DeferredBlock<SdrBlock> SDR_STANDARD = BLOCKS.registerBlock("sdr_standard", p -> new SdrBlock(SdrTier.STANDARD, p), props());
    public static final DeferredBlock<SdrBlock> SDR_ADVANCED = BLOCKS.registerBlock("sdr_advanced", p -> new SdrBlock(SdrTier.ADVANCED, p), props());
    public static final DeferredItem<BlockItem> SDR_BASIC_ITEM = ITEMS.registerSimpleBlockItem("sdr_basic", SDR_BASIC);
    public static final DeferredItem<BlockItem> SDR_STANDARD_ITEM = ITEMS.registerSimpleBlockItem("sdr_standard", SDR_STANDARD);
    public static final DeferredItem<BlockItem> SDR_ADVANCED_ITEM = ITEMS.registerSimpleBlockItem("sdr_advanced", SDR_ADVANCED);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<SdrBlockEntity>> SDR_BE =
            BLOCK_ENTITIES.register("sdr", () -> BlockEntityType.Builder
                    .of(SdrBlockEntity::new, SDR_BASIC.get(), SDR_STANDARD.get(), SDR_ADVANCED.get()).build(null));

    private RadioSdrContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, SDR_BE.get(), (be, side) -> be.getPeripheral());
            e.registerBlockEntity(com.example.evanscomputermod.radio.api.RadioCapabilities.ENDPOINT, SDR_BE.get(), (be, side) -> be.endpoint());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) {
                e.accept(SDR_BASIC_ITEM.get());
                e.accept(SDR_STANDARD_ITEM.get());
                e.accept(SDR_ADVANCED_ITEM.get());
            }
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(SdrPeripheral.TYPE,
                "Software-defined radio: raw IQ through /dev/sdr", SdrPeripheral.class)));
    }
}
//?}
