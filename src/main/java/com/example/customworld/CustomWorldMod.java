package com.example.customworld;

import com.example.customworld.block.ModBlockEntities;
import com.example.customworld.block.ModBlocks;
import com.example.customworld.block.ModMenuTypes;
import com.example.customworld.command.TeleportDimensionCommand;
import com.example.customworld.command.WasmCommand;
import com.example.customworld.wasm.WasmManager;
import com.example.customworld.world.CustomChunkGenerator;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;
import net.minecraft.world.level.chunk.ChunkGenerator;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

@Mod(CustomWorldMod.MODID)
public class CustomWorldMod {
    public static final String MODID = "customworld";
    public static final Logger LOGGER = LoggerFactory.getLogger(CustomWorldMod.class);

    // Deferred register for chunk generators
    public static final DeferredRegister<MapCodec<? extends ChunkGenerator>> CHUNK_GENERATORS =
            DeferredRegister.create(Registries.CHUNK_GENERATOR, MODID);

    // Register our custom chunk generator
    public static final DeferredHolder<MapCodec<? extends ChunkGenerator>, MapCodec<CustomChunkGenerator>> CUSTOM_CHUNK_GENERATOR =
            CHUNK_GENERATORS.register("custom_chunk_generator", () -> CustomChunkGenerator.CODEC);

    public CustomWorldMod(IEventBus modEventBus) {
        LOGGER.info("Initializing Custom World Generation Mod");

        // Register chunk generators
        CHUNK_GENERATORS.register(modEventBus);
        
        // Register blocks and block items
        ModBlocks.BLOCKS.register(modEventBus);
        ModBlocks.BLOCK_ITEMS.register(modEventBus);
        
        // Register block entities
        ModBlockEntities.BLOCK_ENTITIES.register(modEventBus);
        
        // Register menu types
        ModMenuTypes.MENUS.register(modEventBus);

        // Initialize WASM manager (creates wasm-bin directory)
        WasmManager.initialize();

        // Register event handlers on the NeoForge event bus
        NeoForge.EVENT_BUS.register(this);
        
        LOGGER.info("Registered terminal block and components");
    }

    @SubscribeEvent
    public void onRegisterCommands(RegisterCommandsEvent event) {
        TeleportDimensionCommand.register(event.getDispatcher());
        WasmCommand.register(event.getDispatcher());
    }

    public static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(MODID, path);
    }
}
