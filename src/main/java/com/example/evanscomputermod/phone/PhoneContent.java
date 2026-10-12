package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.mojang.serialization.Codec;
import net.minecraft.core.UUIDUtil;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.config.ModConfig;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.common.extensions.IMenuTypeExtension;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

import java.util.UUID;

/**
 * The ePhone (GUI roadmap P5, hardware layer): the item and its data
 * components, the held-item host ({@link PhoneServer}, {@link PhoneHost}), the
 * slot screen, the Charging Dock, payloads and configs. docs/gui/EPHONE.md
 * describes the design.
 */
public final class PhoneContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MenuType<?>> MENUS = DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /** The phone's computer (given on first use). */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<UUID>> PHONE_ID =
            COMPONENTS.register("phone_id", () -> DataComponentType.<UUID>builder()
                    .persistent(UUIDUtil.CODEC).networkSynchronized(UUIDUtil.STREAM_CODEC).build());
    /** Battery charge in hundredths of a percent (0..10000). */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<Integer>> BATTERY =
            COMPONENTS.register("phone_battery", () -> DataComponentType.<Integer>builder()
                    .persistent(Codec.intRange(0, 10_000)).networkSynchronized(ByteBufCodecs.VAR_INT).build());
    /** Panel, modules and SIM. */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<PhoneHardware>> HARDWARE =
            COMPONENTS.register("phone_hardware", () -> DataComponentType.<PhoneHardware>builder()
                    .persistent(PhoneHardware.CODEC).networkSynchronized(PhoneHardware.STREAM_CODEC).build());

    public static final DeferredItem<Item> EPHONE = ITEMS.registerItem("ephone", p -> new EPhoneItem(p.stacksTo(1)));

    public static final DeferredBlock<ChargingDockBlock> CHARGING_DOCK = BLOCKS.registerBlock("charging_dock",
            ChargingDockBlock::new, BlockBehaviour.Properties.of().strength(1.5f, 6f).sound(SoundType.METAL).noOcclusion()
                    .lightLevel(s -> s.getValue(ChargingDockBlock.ACTIVE) ? 3 : 0));
    public static final DeferredItem<BlockItem> CHARGING_DOCK_ITEM = ITEMS.registerSimpleBlockItem("charging_dock", CHARGING_DOCK);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<ChargingDockBlockEntity>> CHARGING_DOCK_BE =
            BLOCK_ENTITIES.register("charging_dock", () -> BlockEntityType.Builder
                    .of(ChargingDockBlockEntity::new, CHARGING_DOCK.get()).build(null));

    public static final DeferredHolder<MenuType<?>, MenuType<PhoneMenu>> PHONE_MENU =
            MENUS.register("ephone", () -> IMenuTypeExtension.create(PhoneMenu::new));

    private PhoneContent() {}

    public static void register(IEventBus modBus, ModContainer container) {
        COMPONENTS.register(modBus);
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        MENUS.register(modBus);
        container.registerConfig(ModConfig.Type.SERVER, PhoneConfig.SPEC, "evanscomputermod-phone-server.toml");
        container.registerConfig(ModConfig.Type.CLIENT, PhoneClientConfig.SPEC, "evanscomputermod-phone-client.toml");
        modBus.addListener(PhoneContent::onPayloads);
        modBus.addListener((RegisterCapabilitiesEvent e) -> e.registerBlockEntity(Capabilities.EnergyStorage.BLOCK,
                CHARGING_DOCK_BE.get(), (be, side) -> be.energy()));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
            e.accept(EPHONE.get());
            e.accept(CHARGING_DOCK_ITEM.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(PhonePeripheral.TYPE,
                "ePhone: battery, power state, display; in-hand hotkeys as phone_hotkey events", PhonePeripheral.class)));
        PhoneApps.install();
        NeoForge.EVENT_BUS.addListener(PhoneServer::onServerTick);
        NeoForge.EVENT_BUS.addListener(PhoneServer::onLogout);
        NeoForge.EVENT_BUS.addListener(PhoneServer::onServerStopping);
        if (FMLEnvironment.dist.isClient()) com.example.evanscomputermod.phone.client.PhoneClient.register(modBus);
    }

    private static void onPayloads(RegisterPayloadHandlersEvent e) {
        var r = e.registrar("1");
        r.playToServer(PhonePackets.Hotkey.TYPE, PhonePackets.Hotkey.STREAM_CODEC, PhoneServer::handleHotkey);
        r.playToServer(PhonePackets.Prefs.TYPE, PhonePackets.Prefs.STREAM_CODEC, PhoneServer::handlePrefs);
        r.playToClient(PhonePackets.Status.TYPE, PhonePackets.Status.STREAM_CODEC,
                (p, ctx) -> ctx.enqueueWork(() -> com.example.evanscomputermod.phone.client.PhoneClient.onStatus(p)));
    }
}
//?}
