package com.example.evanscomputermod.storage;

//? if <=1.21.1 {

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.module.ModuleInteraction;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.device.DecoderBlock;
import com.example.evanscomputermod.storage.device.DecoderBlockEntity;
import com.example.evanscomputermod.storage.device.DriveBlock;
import com.example.evanscomputermod.storage.device.DriveBlockEntity;
import com.example.evanscomputermod.storage.device.DriveMenu;
import com.example.evanscomputermod.storage.device.EncoderBlock;
import com.example.evanscomputermod.storage.device.EncoderBlockEntity;
import com.example.evanscomputermod.storage.device.StorageModule;
import com.example.evanscomputermod.storage.item.CellSummary;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.item.StorageModuleItem;
import net.minecraft.core.UUIDUtil;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.tags.TagKey;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModLoadingContext;
import net.neoforged.fml.config.ModConfig;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.common.extensions.IMenuTypeExtension;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

import java.util.EnumMap;
import java.util.Map;
import java.util.UUID;

/**
 * Item storage (1.21.1): Storage Cells, the Drive, Item Encoder and Decoder,
 * and the bay Storage Module. What cells hold lives in the world's
 * {@link com.example.evanscomputermod.storage.ledger.StorageLedger}; see
 * {@code docs/item-storage.md}.
 */
public final class StorageContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MenuType<?>> MENUS =
            DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);

    /** Items the Encoder refuses (for pack makers: e.g. backpacks that hide their contents). */
    public static final TagKey<Item> STORAGE_BLACKLIST =
            TagKey.create(Registries.ITEM, EvansComputerMod.id("storage_blacklist"));

    // ------------------------------------------------------------ components

    /** A Storage Cell's id in the ledger (given on first mount). */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<UUID>> CELL_ID =
            COMPONENTS.register("cell_id", () -> DataComponentType.<UUID>builder()
                    .persistent(UUIDUtil.CODEC)
                    .networkSynchronized(UUIDUtil.STREAM_CODEC)
                    .build());

    /** Last known fill of a cell, for tooltips. */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<CellSummary>> CELL_SUMMARY =
            COMPONENTS.register("cell_summary", () -> DataComponentType.<CellSummary>builder()
                    .persistent(CellSummary.CODEC)
                    .networkSynchronized(CellSummary.STREAM_CODEC)
                    .build());

    // ------------------------------------------------------------ items

    public static final Map<CellTier, DeferredItem<Item>> CELLS = new EnumMap<>(CellTier.class);
    public static final Map<CellTier, DeferredItem<Item>> COMPONENTS_BY_TIER = new EnumMap<>(CellTier.class);

    static {
        for (CellTier tier : CellTier.values()) {
            CELLS.put(tier, ITEMS.registerItem("storage_cell_" + tier.label(),
                    props -> new StorageCellItem(tier, props.stacksTo(1))));
            COMPONENTS_BY_TIER.put(tier, ITEMS.registerSimpleItem("storage_component_" + tier.label()));
        }
    }

    public static final DeferredItem<Item> STORAGE_MODULE = ITEMS.registerItem("storage_module",
            props -> new StorageModuleItem(props.stacksTo(1)));

    // ------------------------------------------------------------ blocks

    private static BlockBehaviour.Properties deviceProps() {
        return BlockBehaviour.Properties.of().strength(2.5f, 6.0f).sound(SoundType.METAL).requiresCorrectToolForDrops();
    }

    public static final DeferredBlock<DriveBlock> DRIVE = BLOCKS.registerBlock("storage_drive", DriveBlock::new, deviceProps());
    public static final DeferredBlock<EncoderBlock> ENCODER = BLOCKS.registerBlock("item_encoder", EncoderBlock::new, deviceProps());
    public static final DeferredBlock<DecoderBlock> DECODER = BLOCKS.registerBlock("item_decoder", DecoderBlock::new, deviceProps());

    public static final DeferredItem<BlockItem> DRIVE_ITEM = ITEMS.registerSimpleBlockItem("storage_drive", DRIVE);
    public static final DeferredItem<BlockItem> ENCODER_ITEM = ITEMS.registerSimpleBlockItem("item_encoder", ENCODER);
    public static final DeferredItem<BlockItem> DECODER_ITEM = ITEMS.registerSimpleBlockItem("item_decoder", DECODER);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<DriveBlockEntity>> DRIVE_BE =
            BLOCK_ENTITIES.register("storage_drive", () -> BlockEntityType.Builder
                    .of(DriveBlockEntity::new, DRIVE.get()).build(null));
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<EncoderBlockEntity>> ENCODER_BE =
            BLOCK_ENTITIES.register("item_encoder", () -> BlockEntityType.Builder
                    .of(EncoderBlockEntity::new, ENCODER.get()).build(null));
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<DecoderBlockEntity>> DECODER_BE =
            BLOCK_ENTITIES.register("item_decoder", () -> BlockEntityType.Builder
                    .of(DecoderBlockEntity::new, DECODER.get()).build(null));

    public static final DeferredHolder<MenuType<?>, MenuType<DriveMenu>> DRIVE_MENU =
            MENUS.register("storage_drive", () -> IMenuTypeExtension.create(DriveMenu::new));

    private StorageContent() {
    }

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        COMPONENTS.register(modBus);
        MENUS.register(modBus);
        ModLoadingContext.get().getActiveContainer().registerConfig(ModConfig.Type.SERVER, StorageConfig.SPEC);
        modBus.addListener(StorageContent::onRegisterCapabilities);
        modBus.addListener(StorageContent::onCreativeTab);
        modBus.addListener(StorageContent::onCommonSetup);
        StorageEvents.register();
        if (FMLEnvironment.dist.isClient()) {
            com.example.evanscomputermod.storage.client.StorageClient.register(modBus);
        }
    }

    private static void onRegisterCapabilities(RegisterCapabilitiesEvent event) {
        event.registerBlockEntity(PeripheralCapability.PERIPHERAL, DRIVE_BE.get(), (be, side) -> be.peripheral());
        event.registerBlockEntity(PeripheralCapability.PERIPHERAL, ENCODER_BE.get(), (be, side) -> be.peripheral());
        event.registerBlockEntity(PeripheralCapability.PERIPHERAL, DECODER_BE.get(), (be, side) -> be.peripheral());
        event.registerBlockEntity(Capabilities.ItemHandler.BLOCK, ENCODER_BE.get(), (be, side) -> be.itemHandler());
        event.registerBlockEntity(Capabilities.ItemHandler.BLOCK, DECODER_BE.get(), (be, side) -> be.itemHandler());
    }

    private static void onCreativeTab(BuildCreativeModeTabContentsEvent event) {
        if (event.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
        event.accept(DRIVE_ITEM.get());
        event.accept(ENCODER_ITEM.get());
        event.accept(DECODER_ITEM.get());
        event.accept(STORAGE_MODULE.get());
        for (CellTier tier : CellTier.values()) event.accept(CELLS.get(tier).get());
        for (CellTier tier : CellTier.values()) event.accept(COMPONENTS_BY_TIER.get(tier).get());
    }

    private static void onCommonSetup(FMLCommonSetupEvent event) {
        ModuleInteraction.registerInsertable(stack -> stack.getItem() instanceof StorageCellItem);
        PeripheralTypes.register(DriveBlockEntity.TYPE, "Storage Drive (10 Storage Cells)", DriveBlockEntity.Peripheral.class);
        PeripheralTypes.register(StorageModule.TYPE, "Storage Module (one Storage Cell in a bay)", StorageModule.class);
        PeripheralTypes.register(EncoderBlockEntity.TYPE, "Item Encoder (items in, ledger credit)", EncoderBlockEntity.Peripheral.class);
        PeripheralTypes.register(DecoderBlockEntity.TYPE, "Item Decoder (ledger debit, items out)", DecoderBlockEntity.Peripheral.class);
    }
}
//?}
