package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.core.registries.Registries;
import net.minecraft.tags.TagKey;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Smart cards and the Card Reader/Writer (CELLULAR_V2.md, "Primitives"). The SIM card
 * item and its components are registered with cellular ({@code CellularContent}).
 */
public final class CardContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MenuType<?>> MENUS = DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);

    /** Items a Card Reader/Writer takes (SIM cards; keycards later). */
    public static final TagKey<Item> CARDS = TagKey.create(Registries.ITEM, EvansComputerMod.id("cards"));

    public static final DeferredBlock<CardReaderBlock> CARD_READER = BLOCKS.registerBlock("card_reader", CardReaderBlock::new,
            BlockBehaviour.Properties.of().strength(2f, 6f).sound(SoundType.METAL).requiresCorrectToolForDrops());
    public static final DeferredItem<BlockItem> CARD_READER_ITEM = ITEMS.registerSimpleBlockItem("card_reader", CARD_READER);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<CardReaderBlockEntity>> CARD_READER_BE =
            BLOCK_ENTITIES.register("card_reader", () -> BlockEntityType.Builder.of(CardReaderBlockEntity::new, CARD_READER.get()).build(null));
    public static final DeferredHolder<MenuType<?>, MenuType<CardReaderMenu>> CARD_READER_MENU =
            MENUS.register("card_reader", () -> new MenuType<>(CardReaderMenu::new, FeatureFlags.DEFAULT_FLAGS));

    private CardContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        MENUS.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, CARD_READER_BE.get(), (be, side) -> be.peripheral());
            e.registerBlockEntity(Capabilities.ItemHandler.BLOCK, CARD_READER_BE.get(), (be, side) -> be.automation());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(CARD_READER_ITEM.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() ->
                PeripheralTypes.register(CardReaderPeripheral.TYPE,
                        "Card Reader/Writer: read and write SIM cards (present, identity, is_blank, write, respond, eject, status)",
                        CardReaderPeripheral.class)));
        if (FMLEnvironment.dist.isClient()) com.example.evanscomputermod.card.client.CardClient.register(modBus);
    }
}
//?}
