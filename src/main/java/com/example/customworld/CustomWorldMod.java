package com.example.customworld;

import com.example.customworld.block.ModBlockEntities;
import com.example.customworld.block.ModBlocks;
import com.example.customworld.block.ModMenuTypes;
import com.example.customworld.command.TeleportDimensionCommand;
import com.example.customworld.command.WasmCommand;
import com.example.customworld.network.ModNetwork;
import com.example.customworld.wasm.WasmManager;
import com.example.customworld.world.CustomChunkGenerator;
import com.mojang.serialization.Codec;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraftforge.common.MinecraftForge;
import net.minecraftforge.event.RegisterCommandsEvent;
import net.minecraftforge.eventbus.api.IEventBus;
import net.minecraftforge.eventbus.api.SubscribeEvent;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.javafmlmod.FMLJavaModLoadingContext;
import net.minecraftforge.registries.DeferredRegister;
import net.minecraftforge.registries.RegistryObject;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

@Mod(CustomWorldMod.MODID)
public class CustomWorldMod {
    public static final String MODID = "customworld";
    public static final Logger LOGGER = LoggerFactory.getLogger(CustomWorldMod.class);

    // Deferred register for chunk generators
    public static final DeferredRegister<Codec<? extends ChunkGenerator>> CHUNK_GENERATORS =
            DeferredRegister.create(Registries.CHUNK_GENERATOR, MODID);

    // Register our custom chunk generator
    public static final RegistryObject<Codec<CustomChunkGenerator>> CUSTOM_CHUNK_GENERATOR =
            CHUNK_GENERATORS.register("custom_chunk_generator", () -> CustomChunkGenerator.CODEC);

    public CustomWorldMod() {
        LOGGER.info("Initializing Custom World Generation Mod");

        IEventBus modEventBus = FMLJavaModLoadingContext.get().getModEventBus();

        // Register chunk generators
        CHUNK_GENERATORS.register(modEventBus);

        // Register blocks and block items
        ModBlocks.BLOCKS.register(modEventBus);
        ModBlocks.BLOCK_ITEMS.register(modEventBus);

        // Register block entities
        ModBlockEntities.BLOCK_ENTITIES.register(modEventBus);

        // Register menu types
        ModMenuTypes.MENUS.register(modEventBus);

        // Register network packets
        ModNetwork.register();

        // Initialize WASM manager (creates wasm-bin directory)
        WasmManager.initialize();

        // Register event handlers on the Forge event bus
        MinecraftForge.EVENT_BUS.register(this);

        LOGGER.info("Registered terminal block and components");
    }

    @SubscribeEvent
    public void onRegisterCommands(RegisterCommandsEvent event) {
        TeleportDimensionCommand.register(event.getDispatcher());
        WasmCommand.register(event.getDispatcher());
    }

    public static ResourceLocation id(String path) {
        return new ResourceLocation(MODID, path);
    }
}
