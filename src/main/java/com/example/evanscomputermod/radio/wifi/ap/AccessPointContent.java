package com.example.evanscomputermod.radio.wifi.ap;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.radio.api.RadioCapabilities;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.common.extensions.IMenuTypeExtension;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.event.entity.player.PlayerInteractEvent;
import net.neoforged.neoforge.event.server.ServerStoppedEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Wi-Fi Access Point (lane 3B): the {@code access_point} block, its block
 * entity (a layer-2 bridge between the cable segment it sits on and the 2.4 /
 * 5 GHz radio medium, running {@code AccessPointCore}), the settings / status
 * GUI and its packets, and the virtual stations scenarios use.
 */
public final class AccessPointContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MenuType<?>> MENUS = DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);

    public static final DeferredBlock<AccessPointBlock> ACCESS_POINT = BLOCKS.registerBlock("access_point",
            AccessPointBlock::new,
            BlockBehaviour.Properties.of().strength(2.0f, 6.0f).sound(SoundType.METAL).requiresCorrectToolForDrops().noOcclusion()
                    .lightLevel(s -> s.getValue(AccessPointBlock.ACTIVE) ? 3 : 0));
    public static final DeferredItem<BlockItem> ACCESS_POINT_ITEM = ITEMS.registerSimpleBlockItem("access_point", ACCESS_POINT);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<AccessPointBlockEntity>> ACCESS_POINT_BE =
            BLOCK_ENTITIES.register("access_point", () -> BlockEntityType.Builder
                    .of(AccessPointBlockEntity::new, ACCESS_POINT.get()).build(null));
    public static final DeferredHolder<MenuType<?>, MenuType<AccessPointMenu>> ACCESS_POINT_MENU =
            MENUS.register("access_point", () -> IMenuTypeExtension.create(AccessPointMenu::new));

    private AccessPointContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        MENUS.register(modBus);
        modBus.addListener((RegisterPayloadHandlersEvent e) -> ApPackets.register(e));
        modBus.addListener((RegisterCapabilitiesEvent e) -> e.registerBlockEntity(RadioCapabilities.ENDPOINT, ACCESS_POINT_BE.get(),
                (be, side) -> be.endpoint()));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(ACCESS_POINT_ITEM.get());
        });
        // Sneak + wrench: vanilla skips the block's own use while sneaking with an item, so catch the click first.
        NeoForge.EVENT_BUS.addListener((PlayerInteractEvent.RightClickBlock e) -> {
            if (!e.getEntity().isShiftKeyDown() || !AccessPointBlock.isWrench(e.getItemStack())) return;
            if (!(e.getLevel().getBlockEntity(e.getPos()) instanceof AccessPointBlockEntity be)) return;
            if (!e.getLevel().isClientSide()) be.factoryReset(e.getEntity());   // refuses (with a message) unless allowed
            e.setCanceled(true);
            e.setCancellationResult(net.minecraft.world.InteractionResult.sidedSuccess(e.getLevel().isClientSide()));
        });
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Post e) -> VirtualStations.tick());
        NeoForge.EVENT_BUS.addListener((ServerStoppedEvent e) -> {
            VirtualStations.clear();
            AccessPointBlockEntity.clearLive();
        });
        if (FMLEnvironment.dist.isClient())
            com.example.evanscomputermod.radio.wifi.ap.client.AccessPointClient.register(modBus);
    }
}
//?}
