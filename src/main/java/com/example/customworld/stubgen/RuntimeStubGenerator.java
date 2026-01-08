package com.example.customworld.stubgen;

import com.example.customworld.CustomWorldMod;
import net.neoforged.fml.ModList;

import java.lang.annotation.Annotation;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Generates Python peripheral stubs at runtime by scanning CC:Tweaked peripherals.
 * 
 * This class discovers peripheral types as they are encountered during gameplay
 * and generates Python wrapper classes for them. The generated stubs are cached
 * and can be copied to terminal filesystems.
 */
public class RuntimeStubGenerator {
    
    private static final boolean CC_LOADED;
    private static Class<? extends Annotation> luaFunctionAnnotation;
    
    // Cache of generated Python stubs by peripheral type
    private static final Map<String, String> generatedStubs = new ConcurrentHashMap<>();
    
    // Cache of peripheral definitions for types we've seen
    private static final Map<String, PeripheralDefinition> peripheralDefinitions = new ConcurrentHashMap<>();
    
    // The Python stub generator instance
    private static final PythonStubGenerator pythonGenerator = new PythonStubGenerator();
    
    // Scanner configured with the CC:Tweaked annotation
    private static PeripheralScanner scanner;
    
    static {
        CC_LOADED = ModList.get().isLoaded("computercraft");
        if (CC_LOADED) {
            try {
                @SuppressWarnings("unchecked")
                Class<? extends Annotation> annClass = (Class<? extends Annotation>) 
                    Class.forName("dan200.computercraft.api.lua.LuaFunction");
                luaFunctionAnnotation = annClass;
                scanner = new PeripheralScanner(luaFunctionAnnotation);
                CustomWorldMod.LOGGER.info("RuntimeStubGenerator initialized with CC:Tweaked support");
            } catch (ClassNotFoundException e) {
                CustomWorldMod.LOGGER.warn("LuaFunction annotation not available: {}", e.getMessage());
            }
        } else {
            CustomWorldMod.LOGGER.info("CC:Tweaked not installed - RuntimeStubGenerator disabled");
        }
    }
    
    /**
     * Checks if runtime stub generation is available.
     */
    public static boolean isAvailable() {
        return CC_LOADED && luaFunctionAnnotation != null && scanner != null;
    }
    
    /**
     * Registers a peripheral type and generates its Python stub.
     * Called when a new peripheral type is discovered.
     * 
     * @param peripheralType The peripheral type name (e.g., "playerDetector")
     * @param peripheralClass The class of the peripheral (IPeripheral implementation)
     * @return The generated Python stub, or null if generation failed
     */
    public static String registerPeripheral(String peripheralType, Class<?> peripheralClass) {
        if (!isAvailable()) {
            return null;
        }
        
        // Check if we already have this type
        if (generatedStubs.containsKey(peripheralType)) {
            return generatedStubs.get(peripheralType);
        }
        
        try {
            // Scan the peripheral class for methods
            PeripheralDefinition definition = scanner.scanClass(peripheralClass, peripheralType);
            
            if (definition.getMethods().isEmpty()) {
                CustomWorldMod.LOGGER.debug("No methods found for peripheral type: {}", peripheralType);
                return null;
            }
            
            // Store the definition
            peripheralDefinitions.put(peripheralType, definition);
            
            // Generate the Python stub
            PeripheralDefinitions defs = new PeripheralDefinitions();
            defs.addPeripheral(definition);
            String stub = pythonGenerator.generateSingleFile(defs);
            
            // Cache it
            generatedStubs.put(peripheralType, stub);
            
            CustomWorldMod.LOGGER.info("Generated Python stub for peripheral: {} ({} methods)", 
                    peripheralType, definition.getMethods().size());
            
            return stub;
            
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Failed to generate stub for peripheral {}: {}", 
                    peripheralType, e.getMessage());
            return null;
        }
    }
    
    /**
     * Registers a peripheral from an IPeripheral instance.
     * Extracts the type and class from the peripheral object.
     * 
     * @param peripheral The IPeripheral instance
     * @return The generated Python stub, or null if generation failed
     */
    public static String registerPeripheral(Object peripheral) {
        if (!isAvailable() || peripheral == null) {
            return null;
        }
        
        try {
            // Get the peripheral type using reflection
            Class<?> iPeripheralClass = Class.forName("dan200.computercraft.api.peripheral.IPeripheral");
            Method getTypeMethod = iPeripheralClass.getMethod("getType");
            String peripheralType = (String) getTypeMethod.invoke(peripheral);
            
            if (peripheralType == null || peripheralType.isEmpty()) {
                return null;
            }
            
            // Check if we already have this type
            if (generatedStubs.containsKey(peripheralType)) {
                return generatedStubs.get(peripheralType);
            }
            
            // Register using the peripheral's class
            return registerPeripheral(peripheralType, peripheral.getClass());
            
        } catch (Exception e) {
            CustomWorldMod.LOGGER.debug("Failed to register peripheral: {}", e.getMessage());
            return null;
        }
    }
    
    /**
     * Gets the cached Python stub for a peripheral type.
     * 
     * @param peripheralType The peripheral type name
     * @return The Python stub, or null if not generated
     */
    public static String getStub(String peripheralType) {
        return generatedStubs.get(peripheralType);
    }
    
    /**
     * Gets all generated Python stubs as a single combined file.
     * 
     * @return Combined Python file with all peripheral wrappers
     */
    public static String getAllStubs() {
        if (peripheralDefinitions.isEmpty()) {
            return null;
        }
        
        PeripheralDefinitions defs = new PeripheralDefinitions();
        for (PeripheralDefinition def : peripheralDefinitions.values()) {
            defs.addPeripheral(def);
        }
        
        return pythonGenerator.generateSingleFile(defs);
    }
    
    /**
     * Gets the Python stub for a specific peripheral type as a standalone file.
     * 
     * @param peripheralType The peripheral type name
     * @return The Python file content, or null if not found
     */
    public static String getStubFile(String peripheralType) {
        PeripheralDefinition def = peripheralDefinitions.get(peripheralType);
        if (def == null) {
            return null;
        }
        
        PeripheralDefinitions defs = new PeripheralDefinitions();
        defs.addPeripheral(def);
        return pythonGenerator.generateSingleFile(defs);
    }
    
    /**
     * Gets all registered peripheral types.
     * 
     * @return Set of peripheral type names
     */
    public static Set<String> getRegisteredTypes() {
        return Collections.unmodifiableSet(peripheralDefinitions.keySet());
    }
    
    /**
     * Gets the peripheral definition for a type.
     * 
     * @param peripheralType The peripheral type name
     * @return The definition, or null if not found
     */
    public static PeripheralDefinition getDefinition(String peripheralType) {
        return peripheralDefinitions.get(peripheralType);
    }
    
    /**
     * Generates individual Python module files for each peripheral type.
     * Returns a map of filename -> content.
     * 
     * @return Map of Python filenames to their content
     */
    public static Map<String, String> generateIndividualStubs() {
        Map<String, String> files = new HashMap<>();
        
        if (!isAvailable() || peripheralDefinitions.isEmpty()) {
            return files;
        }
        
        // Generate __init__.py that imports all peripherals
        StringBuilder initPy = new StringBuilder();
        initPy.append("\"\"\"Auto-generated CC:Tweaked peripheral wrappers.\n\n");
        initPy.append("Available peripherals:\n");
        for (String type : peripheralDefinitions.keySet()) {
            PeripheralDefinition def = peripheralDefinitions.get(type);
            initPy.append("  - ").append(def.toPythonClassName()).append("\n");
        }
        initPy.append("\"\"\"\n\n");
        
        for (PeripheralDefinition def : peripheralDefinitions.values()) {
            String moduleName = def.toPythonModuleName();
            String className = def.toPythonClassName();
            
            // Add import to __init__.py
            initPy.append("from ").append(moduleName).append(" import ").append(className).append("\n");
            
            // Generate individual file
            PeripheralDefinitions singleDef = new PeripheralDefinitions();
            singleDef.addPeripheral(def);
            String content = pythonGenerator.generateSingleFile(singleDef);
            files.put(moduleName + ".py", content);
        }
        
        // Add __all__ to __init__.py
        initPy.append("\n__all__ = [\n");
        for (PeripheralDefinition def : peripheralDefinitions.values()) {
            initPy.append("    '").append(def.toPythonClassName()).append("',\n");
        }
        initPy.append("]\n");
        
        files.put("__init__.py", initPy.toString());
        
        return files;
    }
    
    /**
     * Clears all cached stubs. Useful for testing or if peripherals change.
     */
    public static void clearCache() {
        generatedStubs.clear();
        peripheralDefinitions.clear();
        CustomWorldMod.LOGGER.debug("RuntimeStubGenerator cache cleared");
    }
    
    /**
     * Gets statistics about the stub generator.
     * 
     * @return A string with statistics
     */
    public static String getStats() {
        int totalMethods = peripheralDefinitions.values().stream()
                .mapToInt(d -> d.getMethods().size())
                .sum();
        
        return String.format("RuntimeStubGenerator: %d peripherals, %d methods",
                peripheralDefinitions.size(), totalMethods);
    }
}
