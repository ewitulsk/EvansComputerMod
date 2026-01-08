package com.example.customworld.wasm;

import com.example.customworld.CustomWorldMod;
import net.minecraft.server.MinecraftServer;

import javax.annotation.Nullable;
import java.lang.annotation.Annotation;
import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Invokes methods on CC:Tweaked peripherals using reflection.
 * Handles both @LuaFunction annotated methods and IDynamicPeripheral implementations.
 */
@SuppressWarnings("unchecked")
public class PeripheralMethodInvoker {
    
    private static Class<? extends Annotation> luaFunctionAnnotationClass;
    private static Class<?> iDynamicPeripheralClass;
    private static Class<?> iComputerAccessClass;
    private static Class<?> iLuaContextClass;
    private static Class<?> iArgumentsClass;
    private static Class<?> methodResultClass;
    private static Method dynamicCallMethod;
    private static Method dynamicGetMethodNamesMethod;
    
    // Cached method lookups for peripherals
    private final Map<Class<?>, Map<String, Method>> methodCache = new HashMap<>();
    
    static {
        try {
            luaFunctionAnnotationClass = (Class<? extends Annotation>) Class.forName("dan200.computercraft.api.lua.LuaFunction");
            iDynamicPeripheralClass = Class.forName("dan200.computercraft.api.peripheral.IDynamicPeripheral");
            iComputerAccessClass = Class.forName("dan200.computercraft.api.peripheral.IComputerAccess");
            iLuaContextClass = Class.forName("dan200.computercraft.api.lua.ILuaContext");
            iArgumentsClass = Class.forName("dan200.computercraft.api.lua.IArguments");
            methodResultClass = Class.forName("dan200.computercraft.api.lua.MethodResult");
            
            dynamicCallMethod = iDynamicPeripheralClass.getMethod("callMethod", 
                iComputerAccessClass, iLuaContextClass, int.class, iArgumentsClass);
            dynamicGetMethodNamesMethod = iDynamicPeripheralClass.getMethod("getMethodNames");
            
            CustomWorldMod.LOGGER.debug("PeripheralMethodInvoker initialized successfully");
        } catch (Exception e) {
            CustomWorldMod.LOGGER.debug("CC:Tweaked APIs not available for method invocation: {}", e.getMessage());
        }
    }
    
    /**
     * Checks if method invocation is available.
     */
    public static boolean isAvailable() {
        return luaFunctionAnnotationClass != null;
    }
    
    /**
     * Invokes a method on a peripheral with JSON arguments, returning a JSON result.
     * 
     * @param peripheral The peripheral object (IPeripheral instance)
     * @param methodName The name of the method to call
     * @param jsonArgs JSON string containing the arguments array
     * @param server The Minecraft server (for main thread execution)
     * @return JSON string with the result or error
     */
    public String invokeMethod(Object peripheral, String methodName, String jsonArgs, @Nullable MinecraftServer server) {
        if (!isAvailable()) {
            return LuaWasmTypeConverter.errorJson("CC:Tweaked integration not available");
        }
        
        if (peripheral == null) {
            return LuaWasmTypeConverter.errorJson("Peripheral is null");
        }
        
        try {
            // Parse JSON arguments
            Object[] args = LuaWasmTypeConverter.parseJsonToArgs(jsonArgs);
            
            // IMPORTANT: Try @LuaFunction annotated methods FIRST
            // Advanced Peripherals' BasePeripheral implements IDynamicPeripheral for plugin methods,
            // but the main methods (like getOnlinePlayers) are @LuaFunction annotations on the class.
            // IDynamicPeripheral.getMethodNames() only returns plugin methods, not annotated methods.
            String result = invokeAnnotatedMethod(peripheral, methodName, args, server);
            
            // If annotated method was found, return its result
            if (!result.contains("\"error\":\"Method not found:")) {
                return result;
            }
            
            // Fall back to dynamic peripheral method if not found as annotated
            if (iDynamicPeripheralClass != null && iDynamicPeripheralClass.isInstance(peripheral)) {
                CustomWorldMod.LOGGER.debug("Method {} not found as @LuaFunction, trying IDynamicPeripheral", methodName);
                return invokeDynamicMethod(peripheral, methodName, args, server);
            }
            
            // Method not found anywhere
            return result;
            
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error invoking peripheral method {}: {}", methodName, e.getMessage());
            return LuaWasmTypeConverter.errorJson("Invocation error: " + e.getMessage());
        }
    }
    
    /**
     * Invokes a method on a dynamic peripheral.
     */
    private String invokeDynamicMethod(Object peripheral, String methodName, Object[] args, @Nullable MinecraftServer server) {
        try {
            // Get method names to find the index
            String[] methodNames = (String[]) dynamicGetMethodNamesMethod.invoke(peripheral);
            int methodIndex = -1;
            for (int i = 0; i < methodNames.length; i++) {
                if (methodNames[i].equals(methodName)) {
                    methodIndex = i;
                    break;
                }
            }
            
            if (methodIndex < 0) {
                return LuaWasmTypeConverter.errorJson("Method not found: " + methodName);
            }
            
            // Create IArguments from parsed args
            Object iArguments = LuaWasmTypeConverter.createArguments(args);
            if (iArguments == null) {
                return LuaWasmTypeConverter.errorJson("Failed to create arguments");
            }
            
            // Invoke the method (we pass null for IComputerAccess and ILuaContext)
            // This works for most peripherals but some may require actual context
            final int finalMethodIndex = methodIndex;
            Object result = executeOnServerThread(() -> {
                try {
                    return dynamicCallMethod.invoke(peripheral, null, null, finalMethodIndex, iArguments);
                } catch (Exception e) {
                    throw new RuntimeException(e);
                }
            }, server);
            
            return LuaWasmTypeConverter.methodResultToJson(result);
            
        } catch (Exception e) {
            CustomWorldMod.LOGGER.error("Error invoking dynamic method {}: {}", methodName, e.getMessage());
            return LuaWasmTypeConverter.errorJson("Dynamic invocation error: " + e.getMessage());
        }
    }
    
    /**
     * Invokes an @LuaFunction annotated method.
     */
    private String invokeAnnotatedMethod(Object peripheral, String methodName, Object[] args, @Nullable MinecraftServer server) {
        try {
            // Look up the method (cached)
            Method method = findLuaMethod(peripheral.getClass(), methodName);
            if (method == null) {
                return LuaWasmTypeConverter.errorJson("Method not found: " + methodName);
            }
            
            // Check if method needs to run on main thread
            boolean mainThread = false;
            Annotation luaFuncAnnotation = method.getAnnotation(luaFunctionAnnotationClass);
            if (luaFuncAnnotation != null) {
                try {
                    Method mainThreadMethod = luaFunctionAnnotationClass.getMethod("mainThread");
                    mainThread = (Boolean) mainThreadMethod.invoke(luaFuncAnnotation);
                } catch (Exception ignored) {
                    // No mainThread attribute or error reading it
                }
            }
            
            // Prepare arguments based on method signature
            Object[] invokeArgs = prepareInvokeArgs(method, args);
            
            // Invoke the method
            Object result;
            if (mainThread && server != null) {
                final Method finalMethod = method;
                final Object[] finalArgs = invokeArgs;
                result = executeOnServerThread(() -> {
                    try {
                        return finalMethod.invoke(peripheral, finalArgs);
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                }, server);
            } else {
                result = method.invoke(peripheral, invokeArgs);
            }
            
            // Handle MethodResult
            if (result != null && methodResultClass.isInstance(result)) {
                return LuaWasmTypeConverter.methodResultToJson(result);
            }
            
            // Direct return value
            return LuaWasmTypeConverter.successJson(result);
            
        } catch (Exception e) {
            Throwable cause = e.getCause() != null ? e.getCause() : e;
            CustomWorldMod.LOGGER.error("Error invoking annotated method {}: {}", methodName, cause.getMessage());
            return LuaWasmTypeConverter.errorJson("Invocation error: " + cause.getMessage());
        }
    }
    
    /**
     * Finds a @LuaFunction method by name in a class (cached).
     */
    @Nullable
    private Method findLuaMethod(Class<?> clazz, String methodName) {
        // Check cache
        Map<String, Method> classCache = methodCache.computeIfAbsent(clazz, k -> new HashMap<>());
        
        if (classCache.containsKey(methodName)) {
            return classCache.get(methodName);
        }
        
        // Search for method with @LuaFunction annotation
        for (Method method : clazz.getMethods()) {
            if (!method.getName().equals(methodName)) {
                continue;
            }
            
            if (method.isAnnotationPresent(luaFunctionAnnotationClass)) {
                classCache.put(methodName, method);
                return method;
            }
        }
        
        // Also check for methods in superclasses and interfaces
        for (Method method : clazz.getDeclaredMethods()) {
            if (!method.getName().equals(methodName)) {
                continue;
            }
            
            if (method.isAnnotationPresent(luaFunctionAnnotationClass)) {
                method.setAccessible(true);
                classCache.put(methodName, method);
                return method;
            }
        }
        
        classCache.put(methodName, null);
        return null;
    }
    
    /**
     * Prepares arguments for method invocation based on method signature.
     */
    private Object[] prepareInvokeArgs(Method method, Object[] args) {
        Class<?>[] paramTypes = method.getParameterTypes();
        
        // If method takes IArguments, wrap args
        if (paramTypes.length == 1 && iArgumentsClass.isAssignableFrom(paramTypes[0])) {
            Object iArguments = LuaWasmTypeConverter.createArguments(args);
            return new Object[] { iArguments };
        }
        
        // If method takes no params
        if (paramTypes.length == 0) {
            return new Object[0];
        }
        
        // Try to match args to param types
        Object[] result = new Object[paramTypes.length];
        for (int i = 0; i < paramTypes.length; i++) {
            if (i < args.length) {
                result[i] = convertArg(args[i], paramTypes[i]);
            } else {
                result[i] = getDefaultValue(paramTypes[i]);
            }
        }
        
        return result;
    }
    
    /**
     * Converts an argument to the expected type.
     */
    @Nullable
    private Object convertArg(Object arg, Class<?> targetType) {
        if (arg == null) {
            return getDefaultValue(targetType);
        }
        
        if (targetType.isAssignableFrom(arg.getClass())) {
            return arg;
        }
        
        // Handle primitive type conversions
        if (targetType == int.class || targetType == Integer.class) {
            if (arg instanceof Number n) return n.intValue();
            if (arg instanceof String s) return Integer.parseInt(s);
        }
        
        if (targetType == long.class || targetType == Long.class) {
            if (arg instanceof Number n) return n.longValue();
            if (arg instanceof String s) return Long.parseLong(s);
        }
        
        if (targetType == double.class || targetType == Double.class) {
            if (arg instanceof Number n) return n.doubleValue();
            if (arg instanceof String s) return Double.parseDouble(s);
        }
        
        if (targetType == float.class || targetType == Float.class) {
            if (arg instanceof Number n) return n.floatValue();
            if (arg instanceof String s) return Float.parseFloat(s);
        }
        
        if (targetType == boolean.class || targetType == Boolean.class) {
            if (arg instanceof Boolean b) return b;
            if (arg instanceof String s) return Boolean.parseBoolean(s);
        }
        
        if (targetType == String.class) {
            return arg.toString();
        }
        
        return arg;
    }
    
    /**
     * Gets the default value for a type.
     */
    @Nullable
    private Object getDefaultValue(Class<?> type) {
        if (type == boolean.class) return false;
        if (type == int.class) return 0;
        if (type == long.class) return 0L;
        if (type == double.class) return 0.0;
        if (type == float.class) return 0.0f;
        if (type == byte.class) return (byte) 0;
        if (type == short.class) return (short) 0;
        if (type == char.class) return '\0';
        return null;
    }
    
    /**
     * Executes a task on the server main thread and waits for the result.
     */
    private <T> T executeOnServerThread(java.util.function.Supplier<T> task, @Nullable MinecraftServer server) {
        if (server == null) {
            return task.get();
        }
        
        // If we're already on the server thread, just execute
        if (server.isSameThread()) {
            return task.get();
        }
        
        // Execute on server thread and wait for result
        CountDownLatch latch = new CountDownLatch(1);
        AtomicReference<T> result = new AtomicReference<>();
        AtomicReference<Throwable> error = new AtomicReference<>();
        
        server.execute(() -> {
            try {
                result.set(task.get());
            } catch (Throwable t) {
                error.set(t);
            } finally {
                latch.countDown();
            }
        });
        
        try {
            // Wait up to 30 seconds for the result
            if (!latch.await(30, TimeUnit.SECONDS)) {
                throw new RuntimeException("Timeout waiting for server thread execution");
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new RuntimeException("Interrupted while waiting for server thread execution");
        }
        
        if (error.get() != null) {
            if (error.get() instanceof RuntimeException re) {
                throw re;
            }
            throw new RuntimeException(error.get());
        }
        
        return result.get();
    }
}
