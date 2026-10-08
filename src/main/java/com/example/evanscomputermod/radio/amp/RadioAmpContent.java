package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.radio.hazard.RadioHazardContent;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Radio power and hazards (lanes 4D + 5C): the three amplifier tiers, the
 * antenna tuner, and (through {@link RadioHazardContent}) the RF meter,
 * gamerules and hazard listeners. The transmit chain itself is
 * {@link ExciterLink}, which the SDR owns.
 */
public final class RadioAmpContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    private static BlockBehaviour.Properties props() {
        return BlockBehaviour.Properties.of().strength(3f, 6f).sound(SoundType.METAL).requiresCorrectToolForDrops().noOcclusion();
    }

    public static final DeferredBlock<AmplifierBlock> AMPLIFIER_100W = BLOCKS.registerBlock("amplifier_100w",
            p -> new AmplifierBlock(AmpModel.Tier.W100, p), props());
    public static final DeferredBlock<AmplifierBlock> AMPLIFIER_1KW = BLOCKS.registerBlock("amplifier_1kw",
            p -> new AmplifierBlock(AmpModel.Tier.KW1, p), props());
    public static final DeferredBlock<AmplifierBlock> AMPLIFIER_10KW = BLOCKS.registerBlock("amplifier_10kw",
            p -> new AmplifierBlock(AmpModel.Tier.KW10, p), props());
    public static final DeferredBlock<TunerBlock> ANTENNA_TUNER = BLOCKS.registerBlock("antenna_tuner", TunerBlock::new, props());

    public static final DeferredItem<BlockItem> AMPLIFIER_100W_ITEM = ITEMS.registerSimpleBlockItem("amplifier_100w", AMPLIFIER_100W);
    public static final DeferredItem<BlockItem> AMPLIFIER_1KW_ITEM = ITEMS.registerSimpleBlockItem("amplifier_1kw", AMPLIFIER_1KW);
    public static final DeferredItem<BlockItem> AMPLIFIER_10KW_ITEM = ITEMS.registerSimpleBlockItem("amplifier_10kw", AMPLIFIER_10KW);
    public static final DeferredItem<BlockItem> ANTENNA_TUNER_ITEM = ITEMS.registerSimpleBlockItem("antenna_tuner", ANTENNA_TUNER);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<AmplifierBlockEntity>> AMPLIFIER_BE =
            BLOCK_ENTITIES.register("amplifier", () -> BlockEntityType.Builder
                    .of(AmplifierBlockEntity::new, AMPLIFIER_100W.get(), AMPLIFIER_1KW.get(), AMPLIFIER_10KW.get()).build(null));
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<TunerBlockEntity>> TUNER_BE =
            BLOCK_ENTITIES.register("antenna_tuner", () -> BlockEntityType.Builder.of(TunerBlockEntity::new, ANTENNA_TUNER.get()).build(null));

    private RadioAmpContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        RadioHazardContent.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(Capabilities.EnergyStorage.BLOCK, AMPLIFIER_BE.get(), (be, side) -> be.energy());
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, AMPLIFIER_BE.get(), (be, side) -> be.peripheral());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) {
                e.accept(AMPLIFIER_100W_ITEM.get());
                e.accept(AMPLIFIER_1KW_ITEM.get());
                e.accept(AMPLIFIER_10KW_ITEM.get());
                e.accept(ANTENNA_TUNER_ITEM.get());
            }
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(AmplifierPeripheral.TYPE,
                "RF power amplifier: output, SWR, reflected power, temperature, FE draw", AmplifierPeripheral.class)));
    }
}
//?}
