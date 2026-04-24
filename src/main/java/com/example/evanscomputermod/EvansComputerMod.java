package com.example.evanscomputermod;

import com.example.evanscomputermod.block.ModBlockEntities;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.block.ModMenuTypes;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.api.RegisterComputerModulesEvent;
import com.example.evanscomputermod.command.WasmCommand;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.MissileComputerModule;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.computer.TapBridge;
import com.example.evanscomputermod.entity.ModEntities;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.levelgen.Heightmap;
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

        // Register missile items, entities
        ModItems.ITEMS.register(modEventBus);
        ModEntities.ENTITIES.register(modEventBus);

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
            // Register built-in computer modules
            com.example.evanscomputermod.api.ComputerModuleRegistry.register(new MissileComputerModule());

            LOGGER.info("Firing RegisterComputerModulesEvent for third-party mod integration");
            NeoForge.EVENT_BUS.post(new RegisterComputerModulesEvent());

            probeFfmpegNativeLoad();
        });
    }

    private static void probeFfmpegNativeLoad() {
        // Probe 1: load the global helper class (triggers Loader.load of the
        // native libavformat shared library via javacpp's static initializer).
        try {
            int version = org.bytedeco.ffmpeg.global.avformat.avformat_version();
            int major = (version >> 16) & 0xFF;
            int minor = (version >> 8) & 0xFF;
            int patch = version & 0xFF;
            LOGGER.info("FFmpeg libavformat loaded: version {}.{}.{} (raw 0x{})",
                    major, minor, patch, Integer.toHexString(version));
        } catch (Throwable t) {
            LOGGER.error("FFmpeg libavformat native load failed — video playback will not work", t);
            return;
        }
        // Probe 2: make sure the struct classes used by VideoDecoder are
        // resolvable by the mod class loader. If jarJar/fat-jar bundling
        // regressed and left only the global helpers behind, VideoDecoder
        // would later fail with NoClassDefFoundError on a non-kernel thread
        // which is hard to debug — catch it here instead.
        try {
            Class.forName("org.bytedeco.ffmpeg.avformat.AVFormatContext");
            Class.forName("org.bytedeco.ffmpeg.avcodec.AVCodecContext");
            Class.forName("org.bytedeco.ffmpeg.avutil.AVFrame");
            Class.forName("org.bytedeco.ffmpeg.swscale.SwsContext");
            LOGGER.info("FFmpeg struct classes visible on mod class loader");
        } catch (Throwable t) {
            LOGGER.error("FFmpeg struct classes NOT visible on mod class loader —"
                    + " the mod jar is missing bytedeco's ffmpeg/javacpp classes"
                    + " (check the fat-jar merge in build.gradle)", t);
        }
    }

    @SubscribeEvent
    public void onRegisterCommands(RegisterCommandsEvent event) {
        WasmCommand.register(event.getDispatcher());
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
        generateInternetGateway(event.getServer());
    }

    @SubscribeEvent
    public void onServerStopping(ServerStoppingEvent event) {
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
