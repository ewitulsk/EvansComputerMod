package com.example.customworld.stubgen;

import java.lang.annotation.Annotation;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Parameter;
import java.nio.file.Path;
import java.util.*;

/**
 * Scans classes for peripheral methods annotated with @LuaFunction.
 * Can scan both CC:Tweaked peripherals and mock test peripherals.
 */
public class PeripheralScanner {
    
    private Class<? extends Annotation> luaFunctionClass;
    
    /**
     * Creates a scanner that looks for the standard CC:Tweaked @LuaFunction annotation.
     */
    public PeripheralScanner() {
        try {
            @SuppressWarnings("unchecked")
            Class<? extends Annotation> clazz = (Class<? extends Annotation>) 
                Class.forName("dan200.computercraft.api.lua.LuaFunction");
            this.luaFunctionClass = clazz;
        } catch (ClassNotFoundException e) {
            // CC:Tweaked not available, will use mock annotation
            this.luaFunctionClass = null;
        }
    }
    
    /**
     * Creates a scanner with a custom annotation class (for testing).
     */
    public PeripheralScanner(Class<? extends Annotation> annotationClass) {
        this.luaFunctionClass = annotationClass;
    }
    
    /**
     * Sets a custom annotation class to scan for.
     */
    public void setAnnotationClass(Class<? extends Annotation> annotationClass) {
        this.luaFunctionClass = annotationClass;
    }
    
    /**
     * Scans a class for @LuaFunction methods and returns a peripheral definition.
     * 
     * @param clazz The class to scan
     * @param peripheralType The type name for this peripheral (e.g., "playerDetector")
     * @return PeripheralDefinition with all discovered methods
     */
    public PeripheralDefinition scanClass(Class<?> clazz, String peripheralType) {
        PeripheralDefinition peripheral = new PeripheralDefinition(peripheralType, clazz.getName());
        
        if (luaFunctionClass == null) {
            return peripheral; // No annotation class available
        }
        
        Set<String> seenMethods = new HashSet<>();
        
        // Scan all public methods
        for (Method method : clazz.getMethods()) {
            // Skip static methods
            if (Modifier.isStatic(method.getModifiers())) {
                continue;
            }
            
            // Check for annotation
            Annotation annotation = method.getAnnotation(luaFunctionClass);
            if (annotation == null) {
                continue;
            }
            
            // Get method names from annotation (can expose multiple names)
            String[] luaNames = getAnnotationNames(annotation, method.getName());
            
            for (String luaName : luaNames) {
                if (seenMethods.contains(luaName)) {
                    continue; // Skip duplicate names
                }
                seenMethods.add(luaName);
                
                MethodDefinition methodDef = createMethodDefinition(method, luaName, annotation);
                peripheral.addMethod(methodDef);
            }
        }
        
        return peripheral;
    }
    
    /**
     * Scans multiple classes and returns all peripheral definitions.
     */
    public PeripheralDefinitions scanClasses(Map<String, Class<?>> peripheralClasses) {
        PeripheralDefinitions definitions = new PeripheralDefinitions();
        
        for (Map.Entry<String, Class<?>> entry : peripheralClasses.entrySet()) {
            PeripheralDefinition peripheral = scanClass(entry.getValue(), entry.getKey());
            if (!peripheral.getMethods().isEmpty()) {
                definitions.addPeripheral(peripheral);
            }
        }
        
        return definitions;
    }
    
    /**
     * Gets the Lua names from the annotation, or falls back to the method name.
     */
    private String[] getAnnotationNames(Annotation annotation, String defaultName) {
        try {
            Method valueMethod = annotation.annotationType().getMethod("value");
            String[] names = (String[]) valueMethod.invoke(annotation);
            if (names != null && names.length > 0 && !names[0].isEmpty()) {
                return names;
            }
        } catch (Exception ignored) {
            // Annotation doesn't have value() or error reading it
        }
        return new String[] { defaultName };
    }
    
    /**
     * Checks if the method should run on the main thread.
     */
    private boolean isMainThread(Annotation annotation) {
        try {
            Method mainThreadMethod = annotation.annotationType().getMethod("mainThread");
            return (Boolean) mainThreadMethod.invoke(annotation);
        } catch (Exception ignored) {
            return false;
        }
    }
    
    /**
     * Creates a MethodDefinition from a Java method.
     */
    private MethodDefinition createMethodDefinition(Method method, String luaName, Annotation annotation) {
        MethodDefinition def = new MethodDefinition(method.getName(), luaName);
        def.setMainThread(isMainThread(annotation));
        
        // Determine return type
        Class<?> returnType = method.getReturnType();
        def.setReturnType(javaTypeToString(returnType));
        
        // Extract parameters
        Parameter[] params = method.getParameters();
        Class<?>[] paramTypes = method.getParameterTypes();
        
        for (int i = 0; i < params.length; i++) {
            Class<?> paramType = paramTypes[i];
            
            // Skip IArguments, IComputerAccess, ILuaContext parameters
            // These are injected by CC:Tweaked and not exposed to Lua
            String typeName = paramType.getName();
            if (typeName.contains("IArguments") || 
                typeName.contains("IComputerAccess") || 
                typeName.contains("ILuaContext")) {
                continue;
            }
            
            ParameterDefinition paramDef = new ParameterDefinition();
            paramDef.setName(params[i].getName());
            paramDef.setType(javaTypeToString(paramType));
            paramDef.setOptional(isOptionalParameter(params[i]));
            
            def.addParameter(paramDef);
        }
        
        return def;
    }
    
    /**
     * Checks if a parameter is optional (has Optional type or annotation).
     */
    private boolean isOptionalParameter(Parameter param) {
        // Check for Optional<T> type
        if (param.getType().getName().contains("Optional")) {
            return true;
        }
        // Check for @Optional annotation (CC:Tweaked uses this)
        for (Annotation ann : param.getAnnotations()) {
            if (ann.annotationType().getSimpleName().equals("Optional")) {
                return true;
            }
        }
        return false;
    }
    
    /**
     * Converts a Java type to a simple string representation.
     */
    private String javaTypeToString(Class<?> type) {
        if (type == void.class || type == Void.class) {
            return "void";
        }
        if (type == int.class || type == Integer.class) {
            return "int";
        }
        if (type == long.class || type == Long.class) {
            return "long";
        }
        if (type == double.class || type == Double.class) {
            return "double";
        }
        if (type == float.class || type == Float.class) {
            return "float";
        }
        if (type == boolean.class || type == Boolean.class) {
            return "boolean";
        }
        if (type == String.class) {
            return "String";
        }
        if (type.isArray()) {
            return javaTypeToString(type.getComponentType()) + "[]";
        }
        if (List.class.isAssignableFrom(type)) {
            return "List";
        }
        if (Map.class.isAssignableFrom(type)) {
            return "Map";
        }
        // For CC:Tweaked's MethodResult, treat as Object (we'll return raw JSON)
        if (type.getSimpleName().equals("MethodResult")) {
            return "MethodResult";
        }
        return "Object";
    }
    
    /**
     * Main entry point for running as a standalone tool.
     * Usage: java PeripheralScanner <output-json-path> [class1] [class2] ...
     */
    public static void main(String[] args) {
        if (args.length < 1) {
            System.err.println("Usage: PeripheralScanner <output-json-path> [peripheral-classes...]");
            System.exit(1);
        }
        
        Path outputPath = Path.of(args[0]);
        PeripheralScanner scanner = new PeripheralScanner();
        PeripheralDefinitions definitions = new PeripheralDefinitions();
        
        // If specific classes are provided, scan them
        // Otherwise, we'd need to scan classpath (not implemented here)
        for (int i = 1; i < args.length; i++) {
            String className = args[i];
            try {
                Class<?> clazz = Class.forName(className);
                // Extract peripheral type from class name (simple heuristic)
                String type = extractPeripheralType(clazz);
                PeripheralDefinition def = scanner.scanClass(clazz, type);
                if (!def.getMethods().isEmpty()) {
                    definitions.addPeripheral(def);
                }
            } catch (ClassNotFoundException e) {
                System.err.println("Class not found: " + className);
            }
        }
        
        try {
            definitions.saveToFile(outputPath);
            System.out.println("Saved " + definitions.getPeripherals().size() + 
                             " peripheral definitions to " + outputPath);
        } catch (Exception e) {
            System.err.println("Failed to save definitions: " + e.getMessage());
            System.exit(1);
        }
    }
    
    /**
     * Extracts a peripheral type name from a class (heuristic).
     */
    private static String extractPeripheralType(Class<?> clazz) {
        String name = clazz.getSimpleName();
        // Remove common suffixes
        name = name.replaceAll("Peripheral$", "");
        name = name.replaceAll("Block$", "");
        // Convert to camelCase
        if (!name.isEmpty()) {
            name = Character.toLowerCase(name.charAt(0)) + name.substring(1);
        }
        return name;
    }
}
