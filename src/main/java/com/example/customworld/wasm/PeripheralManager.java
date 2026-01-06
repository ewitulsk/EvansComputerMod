package com.example.customworld.wasm;

import com.example.customworld.CustomWorldMod;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.fml.ModList;

import javax.annotation.Nullable;
import java.lang.annotation.Annotation;
import java.lang.reflect.Method;
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
                
                CustomWorldMod.LOGGER.info("CC:Tweaked integration loaded successfully");
                
                // Optional: IDynamicPeripheral for dynamic peripherals (has getMethodNames)
                try {
                    iDynamicPeripheralClass = Class.forName("dan200.computercraft.api.peripheral.IDynamicPeripheral");
                    getMethodNamesMethod = iDynamicPeripheralClass.getMethod("getMethodNames");
                    CustomWorldMod.LOGGER.debug("IDynamicPeripheral support loaded");
                } catch (ClassNotFoundException | NoSuchMethodException e) {
                    CustomWorldMod.LOGGER.debug("IDynamicPeripheral not available: {}", e.getMessage());
                }
                
                // Optional: LuaFunction annotation for annotation-based peripherals
                try {
                    @SuppressWarnings("unchecked")
                    Class<? extends Annotation> annClass = (Class<? extends Annotation>) 
                        Class.forName("dan200.computercraft.api.lua.LuaFunction");
                    luaFunctionAnnotation = annClass;
                    CustomWorldMod.LOGGER.debug("LuaFunction annotation support loaded");
                } catch (ClassNotFoundException e) {
                    CustomWorldMod.LOGGER.debug("LuaFunction annotation not available: {}", e.getMessage());
                }
                
            } catch (ClassNotFoundException | NoSuchMethodException e) {
                CustomWorldMod.LOGGER.warn("Failed to load CC:Tweaked API classes: {}", e.getMessage());
            }
        } else {
            CustomWorldMod.LOGGER.info("CC:Tweaked not installed - peripheral integration disabled");
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
    
    private final BlockPos terminalPos;
    private final Level level;
    private final Map<String, PeripheralInfo> peripheralsByName = new HashMap<>();
    private final Map<Direction, PeripheralInfo> peripheralsBySide = new HashMap<>();
    
    // Counter for generating unique peripheral names when multiple of same type exist
    private final Map<String, Integer> typeCounters = new HashMap<>();
    
    public PeripheralManager(BlockPos terminalPos, Level level) {
        this.terminalPos = terminalPos;
        this.level = level;
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
            CustomWorldMod.LOGGER.debug("Skipping peripheral scan - CC:Tweaked not available");
            return;
        }
        
        CustomWorldMod.LOGGER.debug("Starting peripheral scan at {}", terminalPos);
        
        for (Direction direction : Direction.values()) {
            BlockPos adjacentPos = terminalPos.relative(direction);
            BlockEntity blockEntity = level.getBlockEntity(adjacentPos);
            
            if (blockEntity != null) {
                CustomWorldMod.LOGGER.debug("Checking block entity at {} ({}): {}", 
                    adjacentPos, direction, blockEntity.getClass().getSimpleName());
                Object peripheral = getPeripheralFromBlockEntity(blockEntity, direction.getOpposite());
                if (peripheral != null) {
                    registerPeripheral(peripheral, direction);
                } else {
                    CustomWorldMod.LOGGER.debug("No peripheral found at {} ({})", adjacentPos, direction);
                }
            }
        }
        
        if (peripheralsByName.isEmpty()) {
            CustomWorldMod.LOGGER.debug("Peripheral scan complete: no peripherals found adjacent to {}", terminalPos);
        } else {
            CustomWorldMod.LOGGER.info("Peripheral scan complete: found {} peripheral(s) at {}: {}", 
                peripheralsByName.size(), terminalPos, peripheralsByName.keySet());
        }
    }
    
    /**
     * Attempts to get an IPeripheral from a block entity using CC's capability system.
     */
    @Nullable
    private Object getPeripheralFromBlockEntity(BlockEntity blockEntity, Direction side) {
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
                CustomWorldMod.LOGGER.debug("Found peripheral via capability at {} side {}", pos, side);
                return peripheral;
            }
        } catch (NoSuchMethodException e) {
            CustomWorldMod.LOGGER.debug("Level.getCapability method not found (expected signature), trying alternatives: {}", e.getMessage());
        } catch (Exception e) {
            CustomWorldMod.LOGGER.debug("Capability lookup failed for {} side {}: {}", pos, side, e.getMessage());
        }
        
        // Fallback: check if block entity implements IPeripheral directly
        if (iPeripheralClass != null && iPeripheralClass.isInstance(blockEntity)) {
            CustomWorldMod.LOGGER.debug("Found peripheral via direct IPeripheral implementation at {}", pos);
            return blockEntity;
        }
        
        // Fallback: try looking for a getPeripheral method on the block entity
        try {
            Method getPeripheralMethod = blockEntity.getClass().getMethod("getPeripheral");
            Object result = getPeripheralMethod.invoke(blockEntity);
            if (result != null && iPeripheralClass.isInstance(result)) {
                CustomWorldMod.LOGGER.debug("Found peripheral via getPeripheral() method at {}", pos);
                return result;
            }
        } catch (NoSuchMethodException ignored) {
            // No getPeripheral method, that's fine
        } catch (Exception e) {
            CustomWorldMod.LOGGER.debug("getPeripheral() call failed for {}: {}", pos, e.getMessage());
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
            
            CustomWorldMod.LOGGER.debug("Registered peripheral: {} ({}) on side {}", name, type, side);
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Failed to register peripheral: {}", e.getMessage());
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
     * Supports both IDynamicPeripheral (with getMethodNames) and annotation-based peripherals (@LuaFunction).
     */
    public String[] getMethodNames(Object peripheral) {
        if (!isCCAvailable() || peripheral == null) {
            return new String[0];
        }
        
        // First, try IDynamicPeripheral.getMethodNames() if the peripheral implements it
        if (iDynamicPeripheralClass != null && getMethodNamesMethod != null 
                && iDynamicPeripheralClass.isInstance(peripheral)) {
            try {
                Object result = getMethodNamesMethod.invoke(peripheral);
                if (result instanceof String[] methods) {
                    CustomWorldMod.LOGGER.debug("Got {} methods via IDynamicPeripheral", methods.length);
                    return methods;
                }
            } catch (Exception e) {
                CustomWorldMod.LOGGER.debug("IDynamicPeripheral.getMethodNames() failed: {}", e.getMessage());
            }
        }
        
        // Fallback: scan for @LuaFunction annotated methods
        if (luaFunctionAnnotation != null) {
            try {
                List<String> methodNames = new ArrayList<>();
                
                // Get the target object that provides the methods
                // IPeripheral.getTarget() returns the object that has the @LuaFunction methods
                Object target = peripheral;
                try {
                    Method getTargetMethod = iPeripheralClass.getMethod("getTarget");
                    Object targetResult = getTargetMethod.invoke(peripheral);
                    if (targetResult != null) {
                        target = targetResult;
                    }
                } catch (NoSuchMethodException ignored) {
                    // getTarget() might not exist in older versions
                }
                
                // Scan the target object's class and all its superclasses for @LuaFunction methods
                Class<?> clazz = target.getClass();
                while (clazz != null && clazz != Object.class) {
                    for (Method method : clazz.getDeclaredMethods()) {
                        if (method.isAnnotationPresent(luaFunctionAnnotation)) {
                            String name = method.getName();
                            if (!methodNames.contains(name)) {
                                methodNames.add(name);
                            }
                        }
                    }
                    // Also check interfaces
                    for (Class<?> iface : clazz.getInterfaces()) {
                        for (Method method : iface.getDeclaredMethods()) {
                            if (method.isAnnotationPresent(luaFunctionAnnotation)) {
                                String name = method.getName();
                                if (!methodNames.contains(name)) {
                                    methodNames.add(name);
                                }
                            }
                        }
                    }
                    clazz = clazz.getSuperclass();
                }
                
                if (!methodNames.isEmpty()) {
                    CustomWorldMod.LOGGER.debug("Got {} methods via @LuaFunction annotation scan", methodNames.size());
                    return methodNames.toArray(new String[0]);
                }
            } catch (Exception e) {
                CustomWorldMod.LOGGER.debug("LuaFunction annotation scan failed: {}", e.getMessage());
            }
        }
        
        CustomWorldMod.LOGGER.debug("No methods found for peripheral of type {}", peripheral.getClass().getSimpleName());
        return new String[0];
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
