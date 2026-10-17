package com.example.evanscomputermod;

import com.example.evanscomputermod.block.ModBlockEntities;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.block.ModMenuTypes;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.command.WasmCommand;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.computer.TapBridge;
import com.example.evanscomputermod.api.wasm.WasmRuntime;
import com.example.evanscomputermod.api.wasm.WasmRuntimeRegistry;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.levelgen.Heightmap;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.common.Mod;
import net.neoforged.fml.config.ModConfig;
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

    public EvansComputerMod(IEventBus modEventBus, ModContainer modContainer) {
        LOGGER.info("Initializing Evans Computer Mod");

        modContainer.registerConfig(ModConfig.Type.COMMON, EcmConfig.SPEC);
        modContainer.registerConfig(ModConfig.Type.SERVER, com.example.evanscomputermod.radio.RadioConfig.SPEC);
        modContainer.registerConfig(ModConfig.Type.SERVER, com.example.evanscomputermod.computer.DnsConfig.SPEC,
                "evanscomputermod-dns-server.toml");

        // Register blocks and block items
        ModBlocks.BLOCKS.register(modEventBus);
        //? if <=1.21.1 {
        com.example.evanscomputermod.worldgen.TechWorldgen.register(modEventBus);
        if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("radio"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.RadioServerChecks::tick);
        else if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("screen"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.ScreenServerChecks::tick);
        else if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("gui"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.GuiServerChecks::tick);
        else if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("phone"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.PhoneServerChecks::tick);
        else if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("upload"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.UploadServerChecks::tick);
        else if (com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("cards"))
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.CardsServerChecks::tick);
        else
            NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.TechServerChecks::tick);
        //?}
        ModBlocks.BLOCK_ITEMS.register(modEventBus);

        // Register custom items and their data components
        ModItems.ITEMS.register(modEventBus);
        com.example.evanscomputermod.module.ModDataComponents.COMPONENTS.register(modEventBus);

        // Register block entities
        ModBlockEntities.BLOCK_ENTITIES.register(modEventBus);

        // Register menu types
        ModMenuTypes.MENUS.register(modEventBus);

        // Register creative tabs
        ModCreativeTabs.CREATIVE_TABS.register(modEventBus);

        // Speaker block (audio device for computers).
        com.example.evanscomputermod.speaker.SpeakerContent.register(modEventBus);

        // Optional Create integration (Redstone Link module / interface block).
        // CreateCompat is only classloaded when Create is present.
        //? if <=1.21.1 {
        if (net.neoforged.fml.ModList.get().isLoaded("create")) {
            com.example.evanscomputermod.compat.create.CreateCompat.register(modEventBus);
        }
        // Lidar Sensor, Sensor Wire and the Wired Sensor Module.
        com.example.evanscomputermod.sensor.SensorContent.register(modEventBus);
        // Radio & Wireless (docs/radio/RADIO_WIRELESS_SPEC.md).
        com.example.evanscomputermod.radio.RadioContent.register(modEventBus);
        // ePhone hardware (docs/gui/EPHONE.md).
        com.example.evanscomputermod.phone.PhoneContent.register(modEventBus, modContainer);
        //?}

        // Initialize WASM manager (creates wasm-bin directory)
        WasmManager.initialize();

        // Register event handlers on the NeoForge event bus
        NeoForge.EVENT_BUS.register(this);
        // /ecm scenario: debug scenarios that run on the server tick.
        NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.command.ScenarioCommand::onServerTick);
        NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.command.ScenarioCommand::onServerStopping);
        // GUI client rendering: a player who leaves no longer holds scene capabilities or images.
        NeoForge.EVENT_BUS.addListener((net.neoforged.neoforge.event.entity.player.PlayerEvent.PlayerLoggedOutEvent e) ->
                com.example.evanscomputermod.computer.ui.UiClients.forget(e.getEntity().getUUID()));

        // Common setup: bind the WASM runtime
        modEventBus.addListener(this::onCommonSetup);

        // In-world GameTests (only run when the gameTestServer enables them).
        //? if >=26.1 {
        com.example.evanscomputermod.testing.NetworkGameTests.register(modEventBus);
        com.example.evanscomputermod.testing.SwitchGameTests.register(modEventBus);
        //?}

        LOGGER.info("Registered terminal block and components");
    }

    private void onCommonSetup(FMLCommonSetupEvent event) {
        event.enqueueWork(() -> {
            // Pick a WASM runtime via ServiceLoader. Main mod always provides
            // Chicory (pure Java); the optional Wasmtime sidecar registers a
            // higher-priority provider when installed.
            try {
                WasmRuntime runtime = WasmRuntimeRegistry.select();
                WasmManager.bind(runtime);
                LOGGER.info("Bound WASM runtime: {}", runtime.providerName());
            } catch (Throwable t) {
                LOGGER.error("Failed to select a WASM runtime — computers will not function", t);
            }

            // Video support comes from the optional evanscomputermod-ffmpeg sidecar (logs which).
            com.example.evanscomputermod.api.video.VideoBackends.get();

            com.example.evanscomputermod.api.peripheral.PeripheralTypes.register(
                    com.example.evanscomputermod.controller.ControllerPeripheral.TYPE,
                    "Wireless Xbox Controller",
                    com.example.evanscomputermod.controller.ControllerPeripheral.class);
        });
    }

    @SubscribeEvent
    public void onRegisterCommands(RegisterCommandsEvent event) {
        WasmCommand.register(event.getDispatcher());
        //? if <=1.21.1 {
        com.example.evanscomputermod.command.TechNetworkCommand.register(event.getDispatcher());
        com.example.evanscomputermod.testing.TechServerChecks.register(event.getDispatcher());
        com.example.evanscomputermod.testing.RadioServerChecks.register(event.getDispatcher());
        com.example.evanscomputermod.testing.ScreenServerChecks.register(event.getDispatcher());
        com.example.evanscomputermod.testing.GuiServerChecks.register(event.getDispatcher());
        com.example.evanscomputermod.testing.PhoneServerChecks.register(event.getDispatcher());
        com.example.evanscomputermod.testing.UploadServerChecks.register(event.getDispatcher());
        com.example.evanscomputermod.testing.CardsServerChecks.register(event.getDispatcher());
        //?}
        com.example.evanscomputermod.command.ScenarioCommand.register(event.getDispatcher());
        com.example.evanscomputermod.command.GuiStatsCommand.register(event.getDispatcher());
    }

    /** Cable topology changes are applied once per tick (coalesced invalidations). */
    @SubscribeEvent
    public void onServerTick(net.neoforged.neoforge.event.tick.ServerTickEvent.Post event) {
        CableNetworkManager.tick();
    }

    /** Before any world loads: resolve computers' storage root on the server thread (see ComputerStorage). */
    @SubscribeEvent
    public void onServerAboutToStart(net.neoforged.neoforge.event.server.ServerAboutToStartEvent event) {
        com.example.evanscomputermod.computer.ComputerStorage.bind(event.getServer());
    }

    @SubscribeEvent
    public void onServerStarted(ServerStartedEvent event) {
        NetworkHub.init();
        LOGGER.info("Network hub started");

        CableNetworkManager.init(event.getServer());
        LOGGER.info("Cable network manager started");

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

        // Generate Internet Gateway and cable column at (0, 0, 0) if not already present
        if(System.getProperty("os.name","").toLowerCase(java.util.Locale.ROOT).contains("windows")
                && Boolean.parseBoolean(System.getProperty("evanscomputermod.internetProxy","true"))) {
            NetworkHub.getInstance().enableInternetProxy();LOGGER.info("Windows socket internet gateway enabled at 10.0.0.1");
        }
        generateInternetGateway(event.getServer());
        //? if <=1.21.1 {
        com.example.evanscomputermod.computer.WorldNetwork.start(event.getServer().overworld());
        //?}
    }

    @SubscribeEvent
    public void onServerStopping(ServerStoppingEvent event) {

        //? if <=1.21.1 {
        com.example.evanscomputermod.computer.WorldNetwork.get(event.getServer().overworld()).stop();
        //?}
        com.example.evanscomputermod.computer.ComputerHost.shutdown(event.getServer());
        CableNetworkManager.shutdown();
        LOGGER.info("Cable network manager stopped");

        NetworkHub.shutdown();
        LOGGER.info("Network hub stopped");
    }

    /**
     * Places the Internet Gateway block at (0, 0, 0) and a cable column from y=1 to the surface.
     * Only runs once — skips if the gateway is already present.
     */
    private void generateInternetGateway(net.minecraft.server.MinecraftServer server) {
        ServerLevel overworld = server.getLevel(Level.OVERWORLD);
        if (overworld == null) return;

        BlockPos gatewayPos = new BlockPos(0, 0, 0);
        if (overworld.getBlockState(gatewayPos).getBlock() instanceof com.example.evanscomputermod.block.InternetGatewayBlock) {
            LOGGER.info("Internet Gateway already present at (0, 0, 0)");
            return;
        }

        // Place the gateway
        overworld.setBlock(gatewayPos, ModBlocks.INTERNET_GATEWAY.get().defaultBlockState(), 3);
        LOGGER.info("Placed Internet Gateway at (0, 0, 0)");

        // Place cable column from y=1 up to the surface with correct connection states
        int surfaceY = overworld.getHeight(Heightmap.Types.WORLD_SURFACE, 0, 0);
        for (int y = 1; y <= surfaceY; y++) {
            BlockPos cablePos = new BlockPos(0, y, 0);
            // Compute connections: DOWN always connects (gateway or cable below), UP connects if not at top
            net.minecraft.world.level.block.state.BlockState cableState = ModBlocks.NETWORK_CABLE.get().defaultBlockState()
                    .setValue(NetworkCableBlock.DOWN, true)
                    .setValue(NetworkCableBlock.UP, y < surfaceY);
            overworld.setBlock(cablePos, cableState, 3);
        }
        LOGGER.info("Placed cable column from y=1 to y={}", surfaceY);
    }

    public static Identifier id(String path) {
        return Identifier.fromNamespaceAndPath(MODID, path);
    }
}
