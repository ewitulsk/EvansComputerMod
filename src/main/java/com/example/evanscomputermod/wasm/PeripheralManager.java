package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IWorldAccess;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.fml.ModList;

import javax.annotation.Nullable;
import java.lang.annotation.Annotation;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Manages discovery and caching of ComputerCraft peripherals adjacent to a terminal block.
 * Uses reflection to access CC:Tweaked APIs to maintain optional dependency status.
 */
public class PeripheralManager {
    
    private static final boolean CC_LOADED;
    private static Class<?> peripheralCapabilityClass;
    private static Class<?> iPeripheralClass;
    private static Class<?> iDynamicPeripheralClass;  // For peripherals that implement IDynamicPeripheral
    private static Class<? extends Annotation> luaFunctionAnnotation;  // For annotation-based peripherals
    private static Method getMethodNamesMethod;  // Only available on IDynamicPeripheral
    private static Method getTypeMethod;  // Available on IPeripheral
    
    static {
        CC_LOADED = ModList.get().isLoaded("computercraft");
        if (CC_LOADED) {
            try {
                // Essential classes - these must succeed for peripheral discovery to work
                peripheralCapabilityClass = Class.forName("dan200.computercraft.api.peripheral.PeripheralCapability");
                iPeripheralClass = Class.forName("dan200.computercraft.api.peripheral.IPeripheral");
                
                // getType() is on IPeripheral - this is essential
                getTypeMethod = iPeripheralClass.getMethod("getType");
                
                EvansComputerMod.LOGGER.info("CC:Tweaked integration loaded successfully");
                
                // Optional: IDynamicPeripheral for dynamic peripherals (has getMethodNames)
                try {
                    iDynamicPeripheralClass = Class.forName("dan200.computercraft.api.peripheral.IDynamicPeripheral");
                    getMethodNamesMethod = iDynamicPeripheralClass.getMethod("getMethodNames");
                    EvansComputerMod.LOGGER.debug("IDynamicPeripheral support loaded");
                } catch (ClassNotFoundException | NoSuchMethodException e) {
                    EvansComputerMod.LOGGER.debug("IDynamicPeripheral not available: {}", e.getMessage());
                }
                
                // Optional: LuaFunction annotation for annotation-based peripherals
                try {
                    @SuppressWarnings("unchecked")
                    Class<? extends Annotation> annClass = (Class<? extends Annotation>) 
                        Class.forName("dan200.computercraft.api.lua.LuaFunction");
                    luaFunctionAnnotation = annClass;
                    EvansComputerMod.LOGGER.debug("LuaFunction annotation support loaded");
                } catch (ClassNotFoundException e) {
                    EvansComputerMod.LOGGER.debug("LuaFunction annotation not available: {}", e.getMessage());
                }
                
            } catch (ClassNotFoundException | NoSuchMethodException e) {
                EvansComputerMod.LOGGER.warn("Failed to load CC:Tweaked API classes: {}", e.getMessage());
            }
        } else {
            EvansComputerMod.LOGGER.info("CC:Tweaked not installed - peripheral integration disabled");
        }
    }
    
    /**
     * Represents a discovered peripheral with its location and type info.
     */
    public static class PeripheralInfo {
        private final String name;
        private final String type;
        private final Direction side;
        private final Object peripheral; // IPeripheral instance
        
        public PeripheralInfo(String name, String type, Direction side, Object peripheral) {
            this.name = name;
            this.type = type;
            this.side = side;
            this.peripheral = peripheral;
        }
        
        public String getName() { return name; }
        public String getType() { return type; }
        public Direction getSide() { return side; }
        public Object getPeripheral() { return peripheral; }
        
        public String getSideName() {
            return switch (side) {
                case DOWN -> "bottom";
                case UP -> "top";
                case NORTH -> "back";
                case SOUTH -> "front";
                case WEST -> "left";
                case EAST -> "right";
            };
        }
    }
    
    private final IWorldAccess worldAccess;
    private final Map<String, PeripheralInfo> peripheralsByName = new HashMap<>();
    private final Map<Direction, PeripheralInfo> peripheralsBySide = new HashMap<>();

    // Counter for generating unique peripheral names when multiple of same type exist
    private final Map<String, Integer> typeCounters = new HashMap<>();

    public PeripheralManager(IWorldAccess worldAccess) {
        this.worldAccess = worldAccess;
    }
    
    /**
     * Checks if CC:Tweaked integration is available.
     */
    public static boolean isCCAvailable() {
        return CC_LOADED && peripheralCapabilityClass != null && iPeripheralClass != null && getTypeMethod != null;
    }
    
    /**
     * Scans all 6 adjacent blocks for CC peripherals and updates the cache.
     * Should be called when the terminal is initialized and when neighbors change.
     */
    public void scanPeripherals() {
        peripheralsByName.clear();
        peripheralsBySide.clear();
        typeCounters.clear();
        
        if (!isCCAvailable()) {
            EvansComputerMod.LOGGER.debug("Skipping peripheral scan - CC:Tweaked not available");
            return;
        }
        
        Level level = worldAccess.getLevel();
        BlockPos terminalPos = worldAccess.getBlockPos();
        if (level == null) {
            EvansComputerMod.LOGGER.debug("Skipping peripheral scan - no level available");
            return;
        }

        EvansComputerMod.LOGGER.debug("Starting peripheral scan at {}", terminalPos);

        for (Direction direction : Direction.values()) {
            BlockPos adjacentPos = terminalPos.relative(direction);
            BlockEntity blockEntity = level.getBlockEntity(adjacentPos);
            
            if (blockEntity != null) {
                EvansComputerMod.LOGGER.debug("Checking block entity at {} ({}): {}", 
                    adjacentPos, direction, blockEntity.getClass().getSimpleName());
                Object peripheral = getPeripheralFromBlockEntity(blockEntity, direction.getOpposite());
                if (peripheral != null) {
                    registerPeripheral(peripheral, direction);
                } else {
                    EvansComputerMod.LOGGER.debug("No peripheral found at {} ({})", adjacentPos, direction);
                }
            }
        }
        
        if (peripheralsByName.isEmpty()) {
            EvansComputerMod.LOGGER.debug("Peripheral scan complete: no peripherals found adjacent to {}", terminalPos);
        } else {
            EvansComputerMod.LOGGER.info("Peripheral scan complete: found {} peripheral(s) at {}: {}", 
                peripheralsByName.size(), terminalPos, peripheralsByName.keySet());
        }
    }
    
    /**
     * Attempts to get an IPeripheral from a block entity using CC's capability system.
     */
    @Nullable
    private Object getPeripheralFromBlockEntity(BlockEntity blockEntity, Direction side) {
        Level level = worldAccess.getLevel();
        if (level == null) return null;
        BlockPos pos = blockEntity.getBlockPos();
        BlockState state = level.getBlockState(pos);
        
        // Try the NeoForge 1.21 capability system first
        try {
            // Get the PeripheralCapability instance via reflection
            // CC:Tweaked provides: PeripheralCapability.get() -> BlockCapability<IPeripheral, Direction>
            Method getMethod = peripheralCapabilityClass.getMethod("get");
            Object capability = getMethod.invoke(null);
            
            // NeoForge 1.21 capability API:
            // Level.getCapability(BlockCapability<T, C>, BlockPos, BlockState, BlockEntity, C context) -> T
            Method getCapabilityMethod = Level.class.getMethod("getCapability", 
                net.neoforged.neoforge.capabilities.BlockCapability.class,
                BlockPos.class,
                BlockState.class,
                BlockEntity.class,
                Object.class);  // Direction is passed as the context
            
            Object peripheral = getCapabilityMethod.invoke(level, capability, pos, state, blockEntity, side);
            
            if (peripheral != null && iPeripheralClass.isInstance(peripheral)) {
                EvansComputerMod.LOGGER.debug("Found peripheral via capability at {} side {}", pos, side);
                return peripheral;
            }
        } catch (NoSuchMethodException e) {
            EvansComputerMod.LOGGER.debug("Level.getCapability method not found (expected signature), trying alternatives: {}", e.getMessage());
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("Capability lookup failed for {} side {}: {}", pos, side, e.getMessage());
        }
        
        // Fallback: check if block entity implements IPeripheral directly
        if (iPeripheralClass != null && iPeripheralClass.isInstance(blockEntity)) {
            EvansComputerMod.LOGGER.debug("Found peripheral via direct IPeripheral implementation at {}", pos);
            return blockEntity;
        }
        
        // Fallback: try looking for a getPeripheral method on the block entity
        try {
            Method getPeripheralMethod = blockEntity.getClass().getMethod("getPeripheral");
            Object result = getPeripheralMethod.invoke(blockEntity);
            if (result != null && iPeripheralClass.isInstance(result)) {
                EvansComputerMod.LOGGER.debug("Found peripheral via getPeripheral() method at {}", pos);
                return result;
            }
        } catch (NoSuchMethodException ignored) {
            // No getPeripheral method, that's fine
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("getPeripheral() call failed for {}: {}", pos, e.getMessage());
        }
        
        return null;
    }
    
    /**
     * Registers a discovered peripheral.
     */
    private void registerPeripheral(Object peripheral, Direction side) {
        try {
            String type = (String) getTypeMethod.invoke(peripheral);
            
            // Generate unique name (e.g., "chat_box_0", "chat_box_1")
            int counter = typeCounters.getOrDefault(type, 0);
            typeCounters.put(type, counter + 1);
            String name = type + "_" + counter;
            
            PeripheralInfo info = new PeripheralInfo(name, type, side, peripheral);
            peripheralsByName.put(name, info);
            peripheralsBySide.put(side, info);
            
            EvansComputerMod.LOGGER.debug("Registered peripheral: {} ({}) on side {}", name, type, side);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Failed to register peripheral: {}", e.getMessage());
        }
    }
    
    /**
     * Gets a list of all discovered peripherals.
     */
    public List<PeripheralInfo> listPeripherals() {
        return new ArrayList<>(peripheralsByName.values());
    }
    
    /**
     * Gets a peripheral by its name.
     */
    public Optional<PeripheralInfo> getPeripheral(String name) {
        return Optional.ofNullable(peripheralsByName.get(name));
    }
    
    /**
     * Gets a peripheral by its side.
     */
    public Optional<PeripheralInfo> getPeripheralBySide(Direction side) {
        return Optional.ofNullable(peripheralsBySide.get(side));
    }
    
    /**
     * Gets the method names available on a peripheral.
     * Matches CC-Tweaked's MethodSupplierImpl approach.
     * Supports both IDynamicPeripheral (with getMethodNames) and annotation-based peripherals (@LuaFunction).
     */
    public String[] getMethodNames(Object peripheral) {
        if (!isCCAvailable() || peripheral == null) {
            return new String[0];
        }
        
        List<String> methodNames = new ArrayList<>();
        
        // IMPORTANT: Scan the peripheral object's class FIRST
        // For Advanced Peripherals, @LuaFunction methods are on the peripheral class itself,
        // NOT on what getTarget() returns (getTarget() returns the owner, not the peripheral)
        EvansComputerMod.LOGGER.debug("Scanning peripheral class: {}", peripheral.getClass().getName());
        scanClassForLuaFunctions(peripheral.getClass(), methodNames);
        
        // Also scan what getTarget() returns (if different from the peripheral)
        // Some peripherals may have additional methods on the target object
        try {
            Method getTargetMethod = iPeripheralClass.getMethod("getTarget");
            Object target = getTargetMethod.invoke(peripheral);
            if (target != null && target != peripheral && target.getClass() != peripheral.getClass()) {
                EvansComputerMod.LOGGER.debug("Also scanning target class: {}", target.getClass().getName());
                scanClassForLuaFunctions(target.getClass(), methodNames);
            }
        } catch (NoSuchMethodException ignored) {
            // getTarget() might not exist in older versions
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("getTarget() failed: {}", e.getMessage());
        }
        
        // Also handle IDynamicPeripheral (CC-Tweaked does this too)
        if (iDynamicPeripheralClass != null && getMethodNamesMethod != null 
                && iDynamicPeripheralClass.isInstance(peripheral)) {
            try {
                Object result = getMethodNamesMethod.invoke(peripheral);
                if (result instanceof String[] dynamicMethods) {
                    EvansComputerMod.LOGGER.debug("Found {} methods via IDynamicPeripheral", dynamicMethods.length);
                    for (String name : dynamicMethods) {
                        if (!methodNames.contains(name)) {
                            methodNames.add(name);
                        }
                    }
                }
            } catch (Exception e) {
                EvansComputerMod.LOGGER.debug("IDynamicPeripheral.getMethodNames() failed: {}", e.getMessage());
            }
        }
        
        if (methodNames.isEmpty()) {
            EvansComputerMod.LOGGER.debug("No methods found for peripheral {}", peripheral.getClass().getSimpleName());
        } else {
            EvansComputerMod.LOGGER.debug("Found {} total methods for peripheral {}", 
                methodNames.size(), peripheral.getClass().getSimpleName());
        }
        
        return methodNames.toArray(new String[0]);
    }
    
    /**
     * Scans a class for @LuaFunction annotated methods and adds their names to the list.
     */
    private void scanClassForLuaFunctions(Class<?> klass, List<String> methodNames) {
        if (luaFunctionAnnotation == null) {
            EvansComputerMod.LOGGER.debug("LuaFunction annotation not available");
            return;
        }
        
        try {
            // Use getMethods() to get all public methods including inherited ones
            // This matches CC-Tweaked's approach: for (var method : klass.getMethods())
            for (Method method : klass.getMethods()) {
                // Get the annotation (not just check presence)
                Annotation annotation = method.getAnnotation(luaFunctionAnnotation);
                if (annotation == null) continue;
                
                // Skip static methods (CC-Tweaked does this)
                if (Modifier.isStatic(method.getModifiers())) {
                    continue;
                }
                
                // Get method names from annotation.value() or use method.getName()
                // CC-Tweaked: var names = annotation.value(); if (names.length == 0) use method.getName()
                try {
                    Method valueMethod = luaFunctionAnnotation.getMethod("value");
                    String[] names = (String[]) valueMethod.invoke(annotation);
                    
                    if (names == null || names.length == 0) {
                        // No explicit names, use the method name
                        if (!methodNames.contains(method.getName())) {
                            methodNames.add(method.getName());
                        }
                    } else {
                        // Use the explicit names from annotation
                        for (String name : names) {
                            if (!methodNames.contains(name)) {
                                methodNames.add(name);
                            }
                        }
                    }
                } catch (NoSuchMethodException e) {
                    // Annotation doesn't have value() method, just use method name
                    if (!methodNames.contains(method.getName())) {
                        methodNames.add(method.getName());
                    }
                }
            }
            
            if (!methodNames.isEmpty()) {
                EvansComputerMod.LOGGER.debug("Found methods via @LuaFunction scan on {}: {}", 
                    klass.getSimpleName(), methodNames);
            }
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("LuaFunction annotation scan failed on {}: {}", 
                klass.getSimpleName(), e.getMessage());
        }
    }
    
    /**
     * Converts the peripheral list to a JSON string.
     */
    public String listPeripheralsAsJson() {
        StringBuilder json = new StringBuilder("[");
        List<PeripheralInfo> peripherals = listPeripherals();
        
        for (int i = 0; i < peripherals.size(); i++) {
            PeripheralInfo info = peripherals.get(i);
            if (i > 0) json.append(",");
            json.append("{");
            json.append("\"name\":\"").append(escapeJson(info.getName())).append("\",");
            json.append("\"type\":\"").append(escapeJson(info.getType())).append("\",");
            json.append("\"side\":\"").append(info.getSideName()).append("\"");
            json.append("}");
        }
        
        json.append("]");
        return json.toString();
    }
    
    /**
     * Gets method names for a peripheral as a JSON array.
     */
    public String getMethodNamesAsJson(String peripheralName) {
        Optional<PeripheralInfo> infoOpt = getPeripheral(peripheralName);
        if (infoOpt.isEmpty()) {
            return "{\"ok\":false,\"error\":\"Peripheral not found: " + escapeJson(peripheralName) + "\"}";
        }
        
        String[] methods = getMethodNames(infoOpt.get().getPeripheral());
        
        StringBuilder json = new StringBuilder("{\"ok\":true,\"result\":[");
        for (int i = 0; i < methods.length; i++) {
            if (i > 0) json.append(",");
            json.append("\"").append(escapeJson(methods[i])).append("\"");
        }
        json.append("]}");
        
        return json.toString();
    }
    
    /**
     * Simple JSON string escaping.
     */
    private static String escapeJson(String s) {
        if (s == null) return "";
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r")
                .replace("\t", "\\t");
    }
}
