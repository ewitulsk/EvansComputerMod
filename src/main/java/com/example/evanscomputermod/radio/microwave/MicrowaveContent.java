package com.example.evanscomputermod.radio.microwave;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.radio.api.RadioCapabilities;
import com.example.evanscomputermod.radio.microwave.dish.DishBlock;
import com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity;
import com.example.evanscomputermod.radio.microwave.dish.DishPeripheral;
import com.example.evanscomputermod.radio.microwave.dish.DishSize;
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

/** Phase 7: the three dishes and the microwave radio. */
public final class MicrowaveContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    private static BlockBehaviour.Properties props() {
        return BlockBehaviour.Properties.of().strength(2f, 6f).sound(SoundType.METAL).noOcclusion().requiresCorrectToolForDrops();
    }

    public static final DeferredBlock<DishBlock> DISH_SMALL = BLOCKS.registerBlock(DishSize.SMALL.id, p -> new DishBlock(DishSize.SMALL, p), props());
    public static final DeferredBlock<DishBlock> DISH_MEDIUM = BLOCKS.registerBlock(DishSize.MEDIUM.id, p -> new DishBlock(DishSize.MEDIUM, p), props());
    public static final DeferredBlock<DishBlock> DISH_LARGE = BLOCKS.registerBlock(DishSize.LARGE.id, p -> new DishBlock(DishSize.LARGE, p), props());
    public static final DeferredBlock<MicrowaveRadioBlock> MICROWAVE_RADIO = BLOCKS.registerBlock("microwave_radio", MicrowaveRadioBlock::new, props());

    public static final DeferredItem<BlockItem> DISH_SMALL_ITEM = ITEMS.registerSimpleBlockItem(DishSize.SMALL.id, DISH_SMALL);
    public static final DeferredItem<BlockItem> DISH_MEDIUM_ITEM = ITEMS.registerSimpleBlockItem(DishSize.MEDIUM.id, DISH_MEDIUM);
    public static final DeferredItem<BlockItem> DISH_LARGE_ITEM = ITEMS.registerSimpleBlockItem(DishSize.LARGE.id, DISH_LARGE);
    public static final DeferredItem<BlockItem> MICROWAVE_RADIO_ITEM = ITEMS.registerSimpleBlockItem("microwave_radio", MICROWAVE_RADIO);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<DishBlockEntity>> DISH_BE =
            BLOCK_ENTITIES.register("dish", () -> BlockEntityType.Builder
                    .of(DishBlockEntity::new, DISH_SMALL.get(), DISH_MEDIUM.get(), DISH_LARGE.get()).build(null));
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<MicrowaveRadioBlockEntity>> RADIO_BE =
            BLOCK_ENTITIES.register("microwave_radio", () -> BlockEntityType.Builder
                    .of(MicrowaveRadioBlockEntity::new, MICROWAVE_RADIO.get()).build(null));

    private MicrowaveContent() {}

    public static DishBlock dish(DishSize size) {
        return switch (size) {
            case SMALL -> DISH_SMALL.get();
            case MEDIUM -> DISH_MEDIUM.get();
            case LARGE -> DISH_LARGE.get();
        };
    }

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, RADIO_BE.get(), (be, side) -> be.getPeripheral());
            e.registerBlockEntity(RadioCapabilities.ENDPOINT, RADIO_BE.get(), (be, side) -> be.link());
            // Any part of a dish answers for the whole dish.
            e.registerBlock(PeripheralCapability.PERIPHERAL, (level, pos, state, be, side) -> {
                DishBlockEntity d = DishBlock.controllerEntity(level, pos);
                return d == null ? null : d.getPeripheral();
            }, DISH_SMALL.get(), DISH_MEDIUM.get(), DISH_LARGE.get());
            e.registerBlock(RadioCapabilities.ENDPOINT, (level, pos, state, be, side) -> {
                DishBlockEntity d = DishBlock.controllerEntity(level, pos);
                return d == null ? null : d.radio();
            }, DISH_SMALL.get(), DISH_MEDIUM.get(), DISH_LARGE.get());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) {
                e.accept(MICROWAVE_RADIO_ITEM.get());
                e.accept(DISH_SMALL_ITEM.get());
                e.accept(DISH_MEDIUM_ITEM.get());
                e.accept(DISH_LARGE_ITEM.get());
            }
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> {
            PeripheralTypes.register(MicrowaveRadioPeripheral.TYPE,
                    "Microwave radio: point-to-point Ethernet bridge over a dish (10/24/60 GHz)", MicrowaveRadioPeripheral.class);
            PeripheralTypes.register(DishPeripheral.TYPE, "Parabolic dish: aim by yaw/pitch, aim_at a point, align on a signal", DishPeripheral.class);
        }));
    }
}
//?}
