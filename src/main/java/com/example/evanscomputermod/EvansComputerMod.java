package com.example.evanscomputermod;

import com.example.evanscomputermod.block.ModBlockEntities;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ModMenuTypes;
import com.example.evanscomputermod.command.WasmCommand;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
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

        // Initialize WASM manager (creates wasm-bin directory)
        WasmManager.initialize();

        // Register event handlers on the NeoForge event bus
        NeoForge.EVENT_BUS.register(this);

        LOGGER.info("Registered terminal block and components");
    }

    @SubscribeEvent
    public void onRegisterCommands(RegisterCommandsEvent event) {
        WasmCommand.register(event.getDispatcher());
    }

    public static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(MODID, path);
    }
}
