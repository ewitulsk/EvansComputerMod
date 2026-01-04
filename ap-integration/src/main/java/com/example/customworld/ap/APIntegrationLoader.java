package com.example.customworld.ap;

import com.example.customworld.ap.peripherals.PlayerDetectorHostFunctions;
import com.example.customworld.wasm.PeripheralHostRegistry;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Main entry point for the Advanced Peripherals integration module.
 * 
 * This mod provides WASM host functions that expose Advanced Peripherals
 * functionality to the terminal OS.
 * 
 * The mod requires both the core customworld mod and Advanced Peripherals
 * to be installed.
 */
@Mod(APIntegrationLoader.MODID)
public class APIntegrationLoader {
    
    public static final String MODID = "customworld_ap";
    public static final Logger LOGGER = LoggerFactory.getLogger(MODID);
    
    public APIntegrationLoader(IEventBus modEventBus) {
        LOGGER.info("Advanced Peripherals integration loading...");
        
        // Register for common setup event
        modEventBus.addListener(this::onCommonSetup);
    }
    
    /**
     * Called during common setup phase.
     * Registers all peripheral host function providers.
     */
    private void onCommonSetup(FMLCommonSetupEvent event) {
        LOGGER.info("Registering Advanced Peripherals host function providers...");
        
        // Register player detector peripheral
        PeripheralHostRegistry.register(new PlayerDetectorHostFunctions());
        
        // Future peripherals can be registered here:
        // PeripheralHostRegistry.register(new ChatBoxHostFunctions());
        // PeripheralHostRegistry.register(new MEBridgeHostFunctions());
        // PeripheralHostRegistry.register(new RSBridgeHostFunctions());
        // etc.
        
        LOGGER.info("Advanced Peripherals integration loaded successfully!");
    }
}
