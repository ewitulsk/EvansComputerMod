package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.card.applet.BankPublic;
import com.example.evanscomputermod.card.applet.BankSecret;
import com.example.evanscomputermod.card.applet.NfcPublic;
import com.example.evanscomputermod.card.applet.NfcSecret;
import com.example.evanscomputermod.card.nfc.NfcModuleItem;
import com.example.evanscomputermod.card.nfc.NfcReaderBlock;
import com.example.evanscomputermod.card.nfc.NfcReaderBlockEntity;
import com.example.evanscomputermod.card.nfc.NfcReaderPeripheral;
import com.example.evanscomputermod.card.printer.CardPrinterBlock;
import com.example.evanscomputermod.card.printer.CardPrinterBlockEntity;
import com.example.evanscomputermod.card.printer.CardPrinterMenu;
import com.example.evanscomputermod.card.printer.CardPrinterPeripheral;
import com.mojang.serialization.Codec;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.codec.ByteBufCodecs;
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
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

import java.util.Map;

/**
 * Cards and their hardware (docs/cards/CARDS.md): the bank / NFC / hybrid card items and
 * their components, the Card Reader/Writer, the Card Printer, the NFC Reader and the
 * reader and NFC bay modules. The SIM card item and its two components are registered
 * with cellular ({@code CellularContent}); it is a {@link CardItem} of kind SIM.
 */
public final class CardContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MenuType<?>> MENUS = DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /** Items a Card Reader/Writer and the printer take (every card kind). */
    public static final TagKey<Item> CARDS = TagKey.create(Registries.ITEM, EvansComputerMod.id("cards"));
    /** Blocks that shield near-field radio (on top of the metal attenuation the medium already applies). */
    public static final TagKey<net.minecraft.world.level.block.Block> NF_SHIELD =
            TagKey.create(Registries.BLOCK, EvansComputerMod.id("nf_shield"));

    // ------------------------------------------------------------ components

    public static final DeferredHolder<DataComponentType<?>, DataComponentType<CardFaces>> CARD_FACES =
            COMPONENTS.register("card_faces", CardFaces::componentType);
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<Map<String, String>>> CARD_PRINT =
            COMPONENTS.register("card_print", () -> DataComponentType.<Map<String, String>>builder()
                    .persistent(Codec.unboundedMap(Codec.STRING, Codec.STRING))
                    .networkSynchronized(ByteBufCodecs.map(java.util.LinkedHashMap::new, ByteBufCodecs.STRING_UTF8, ByteBufCodecs.STRING_UTF8)).build());
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<Stripe>> CARD_STRIPE =
            COMPONENTS.register("card_stripe", Stripe::componentType);
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<Long>> CARD_UID =
            COMPONENTS.register("card_uid", () -> DataComponentType.<Long>builder().persistent(Codec.LONG).networkSynchronized(ByteBufCodecs.VAR_LONG).build());
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<BankPublic>> BANK_APPLET =
            COMPONENTS.register("bank_applet", BankPublic::componentType);
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<BankSecret>> BANK_SECRET =
            COMPONENTS.register("bank_secret", BankSecret::componentType);
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<NfcPublic>> NFC_APPLET =
            COMPONENTS.register("nfc_applet", NfcPublic::componentType);
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<NfcSecret>> NFC_SECRET =
            COMPONENTS.register("nfc_secret", NfcSecret::componentType);

    // ------------------------------------------------------------ items

    public static final DeferredItem<Item> BANK_CARD = ITEMS.registerItem("bank_card", p -> new CardItem(CardKind.BANK, p.stacksTo(16)));
    public static final DeferredItem<Item> NFC_CARD = ITEMS.registerItem("nfc_card", p -> new CardItem(CardKind.NFC, p.stacksTo(16)));
    public static final DeferredItem<Item> HYBRID_CARD = ITEMS.registerItem("hybrid_card", p -> new CardItem(CardKind.HYBRID, p.stacksTo(16)));
    public static final DeferredItem<Item> CARD_READER_MODULE = ITEMS.registerItem("card_reader_module", p -> new CardReaderModuleItem(p.stacksTo(1)));
    public static final DeferredItem<Item> NFC_MODULE = ITEMS.registerItem("nfc_module", p -> new NfcModuleItem(p.stacksTo(1)));

    private static BlockBehaviour.Properties metal() {
        return BlockBehaviour.Properties.of().strength(2f, 6f).sound(SoundType.METAL).requiresCorrectToolForDrops();
    }

    // ------------------------------------------------------------ blocks

    public static final DeferredBlock<CardReaderBlock> CARD_READER = BLOCKS.registerBlock("card_reader", CardReaderBlock::new, metal());
    public static final DeferredItem<BlockItem> CARD_READER_ITEM = ITEMS.registerSimpleBlockItem("card_reader", CARD_READER);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<CardReaderBlockEntity>> CARD_READER_BE =
            BLOCK_ENTITIES.register("card_reader", () -> BlockEntityType.Builder.of(CardReaderBlockEntity::new, CARD_READER.get()).build(null));
    public static final DeferredHolder<MenuType<?>, MenuType<CardReaderMenu>> CARD_READER_MENU =
            MENUS.register("card_reader", () -> new MenuType<>(CardReaderMenu::new, FeatureFlags.DEFAULT_FLAGS));

    public static final DeferredBlock<CardPrinterBlock> CARD_PRINTER = BLOCKS.registerBlock("card_printer", CardPrinterBlock::new, metal());
    public static final DeferredItem<BlockItem> CARD_PRINTER_ITEM = ITEMS.registerSimpleBlockItem("card_printer", CARD_PRINTER);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<CardPrinterBlockEntity>> CARD_PRINTER_BE =
            BLOCK_ENTITIES.register("card_printer", () -> BlockEntityType.Builder.of(CardPrinterBlockEntity::new, CARD_PRINTER.get()).build(null));
    public static final DeferredHolder<MenuType<?>, MenuType<CardPrinterMenu>> CARD_PRINTER_MENU =
            MENUS.register("card_printer", () -> new MenuType<>(CardPrinterMenu::new, FeatureFlags.DEFAULT_FLAGS));

    public static final DeferredBlock<NfcReaderBlock> NFC_READER = BLOCKS.registerBlock("nfc_reader", NfcReaderBlock::new,
            BlockBehaviour.Properties.of().strength(1.5f, 6f).sound(SoundType.METAL).noOcclusion());
    public static final DeferredItem<BlockItem> NFC_READER_ITEM = ITEMS.registerSimpleBlockItem("nfc_reader", NFC_READER);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<NfcReaderBlockEntity>> NFC_READER_BE =
            BLOCK_ENTITIES.register("nfc_reader", () -> BlockEntityType.Builder.of(NfcReaderBlockEntity::new, NFC_READER.get()).build(null));

    private CardContent() {}

    public static void register(IEventBus modBus) {
        COMPONENTS.register(modBus);
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        MENUS.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, CARD_READER_BE.get(), (be, side) -> be.peripheral());
            e.registerBlockEntity(Capabilities.ItemHandler.BLOCK, CARD_READER_BE.get(), (be, side) -> be.automation());
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, CARD_PRINTER_BE.get(), (be, side) -> be.peripheral());
            e.registerBlockEntity(Capabilities.ItemHandler.BLOCK, CARD_PRINTER_BE.get(), (be, side) -> be.automation(side));
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, NFC_READER_BE.get(), (be, side) -> be.peripheral());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
            e.accept(CARD_READER_ITEM.get());
            e.accept(CARD_PRINTER_ITEM.get());
            e.accept(NFC_READER_ITEM.get());
            e.accept(CARD_READER_MODULE.get());
            e.accept(NFC_MODULE.get());
            e.accept(BANK_CARD.get());
            e.accept(NFC_CARD.get());
            e.accept(HYBRID_CARD.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> {
            PeripheralTypes.register(CardReaderPeripheral.TYPE,
                    "Card Reader/Writer: a card in the slot (present, identity, status, eject; read_stripe/write_stripe;"
                            + " write_bank, verify_pin, sign; nfc_auth/nfc_read/nfc_write/nfc_set_key; write, respond for SIMs)",
                    CardReaderPeripheral.class);
            PeripheralTypes.register(CardPrinterPeripheral.TYPE,
                    "Card Printer: paints a card's faces, prints fields and encodes the stripe, consuming dye (present, status, print, cost, eject)",
                    CardPrinterPeripheral.class);
            PeripheralTypes.register(NfcReaderPeripheral.TYPE,
                    "NFC Reader: energises and talks to cards in its near field (set_field, poll, select, auth, read, write, set_key, raw, rssi)",
                    NfcReaderPeripheral.class);
        }));
        NeoForge.EVENT_BUS.register(StripeHazard.class);
        com.example.evanscomputermod.card.nfc.CardEndpointTracker.register();
        if (FMLEnvironment.dist.isClient()) com.example.evanscomputermod.card.client.CardClient.register(modBus);
    }
}
//?}
