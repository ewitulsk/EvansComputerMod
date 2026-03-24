package com.example.evanscomputermod;

import com.example.evanscomputermod.block.ModBlockEntities;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.block.ModMenuTypes;
import com.example.evanscomputermod.api.RegisterComputerModulesEvent;
import com.example.evanscomputermod.command.WasmCommand;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.computer.TapBridge;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.Mod;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.event.server.ServerStartedEvent;
import net.neoforged.neoforge.event.server.ServerStoppingEvent;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

@Mod(EvansComputerMod.MODID)
public class EvansComputerMod {
    public static final String MODID = "evanscomputermod";
    public static final Logger LOGGER = LoggerFactory.getLogger(EvansComputerMod.class);

    public EvansComputerMod(IEventBus modEventBus) {
        LOGGER.info("Initializing Evans Computer Mod");

        // Register blocks and block items
        ModBlocks.BLOCKS.register(modEventBus);
        ModBlocks.BLOCK_ITEMS.register(modEventBus);

        // Register block entities
        ModBlockEntities.BLOCK_ENTITIES.register(modEventBus);

        // Register menu types
        ModMenuTypes.MENUS.register(modEventBus);

        // Register creative tabs
        ModCreativeTabs.CREATIVE_TABS.register(modEventBus);

        // Initialize WASM manager (creates wasm-bin directory)
        WasmManager.initialize();

        // Register event handlers on the NeoForge event bus
        NeoForge.EVENT_BUS.register(this);

        // Listen for common setup to fire module registration event
        modEventBus.addListener(this::onCommonSetup);

        LOGGER.info("Registered terminal block and components");
    }

    private void onCommonSetup(FMLCommonSetupEvent event) {
        event.enqueueWork(() -> {
            LOGGER.info("Firing RegisterComputerModulesEvent for third-party mod integration");
            NeoForge.EVENT_BUS.post(new RegisterComputerModulesEvent());
        });
    }

    @SubscribeEvent
    public void onRegisterCommands(RegisterCommandsEvent event) {
        WasmCommand.register(event.getDispatcher());
    }

    @SubscribeEvent
    public void onServerStarted(ServerStartedEvent event) {
        NetworkHub.init();
        LOGGER.info("Network hub started");

        // Try to open TAP bridge for real internet access
        String tapDevice = System.getProperty("evanscomputermod.tap", "tap0");
        if (!"none".equals(tapDevice)) {
            try {
                TapBridge tap = new TapBridge(tapDevice, NetworkHub.getInstance());
                NetworkHub.getInstance().setTapBridge(tap);
                LOGGER.info("TAP bridge connected: {}", tapDevice);
            } catch (Exception e) {
                LOGGER.warn("TAP bridge unavailable ({}): {} — internet access disabled, LAN networking still works",
                    tapDevice, e.getMessage());
            }
        }
    }

    @SubscribeEvent
    public void onServerStopping(ServerStoppingEvent event) {
        NetworkHub.shutdown();
        LOGGER.info("Network hub stopped");
    }

    public static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(MODID, path);
    }
}
