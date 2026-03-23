package com.example.evanscomputermod.api;

import com.example.evanscomputermod.EvansComputerMod;

import javax.annotation.Nullable;
import java.lang.reflect.Method;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Registry for {@link ComputerModule} annotated classes.
 * Scans annotations and builds metadata used by the WASM bridge to route calls
 * from Python to Java, and to auto-generate visual programming blocks.
 */
public class ComputerModuleRegistry {

    private static final Map<String, ModuleRegistration> modules = new ConcurrentHashMap<>();

    /**
     * Registers a module instance by scanning its {@link ComputerModule} and
     * {@link ComputerFunction} annotations.
     */
    public static void register(Object moduleInstance) {
        Class<?> clazz = moduleInstance.getClass();
        ComputerModule annotation = clazz.getAnnotation(ComputerModule.class);
        if (annotation == null) {
            EvansComputerMod.LOGGER.warn("Attempted to register {} which has no @ComputerModule annotation", clazz.getName());
            return;
        }

        String moduleName = annotation.value();
        if (modules.containsKey(moduleName)) {
            EvansComputerMod.LOGGER.warn("Computer module '{}' is already registered, ignoring duplicate from {}", moduleName, clazz.getName());
            return;
        }

        Map<String, MethodRegistration> methods = new LinkedHashMap<>();

        for (Method method : clazz.getMethods()) {
            ComputerFunction funcAnnotation = method.getAnnotation(ComputerFunction.class);
            if (funcAnnotation == null) continue;

            String pythonName = funcAnnotation.value().isEmpty()
                    ? camelToSnake(method.getName())
                    : funcAnnotation.value();

            // Determine parameters (skip ComputerContext if present as first param)
            Class<?>[] paramTypes = method.getParameterTypes();
            int offset = 0;
            boolean needsContext = paramTypes.length > 0 && ComputerContext.class.isAssignableFrom(paramTypes[0]);
            if (needsContext) {
                offset = 1;
            }

            ParameterInfo[] params = new ParameterInfo[paramTypes.length - offset];
            for (int i = offset; i < paramTypes.length; i++) {
                params[i - offset] = new ParameterInfo("arg" + (i - offset), paramTypes[i]);
            }

            methods.put(pythonName, new MethodRegistration(
                    pythonName,
                    funcAnnotation.description(),
                    method,
                    funcAnnotation.mainThread(),
                    needsContext,
                    params,
                    method.getReturnType()
            ));
        }

        ModuleRegistration registration = new ModuleRegistration(
                moduleName, annotation.description(), moduleInstance, methods
        );
        modules.put(moduleName, registration);
        EvansComputerMod.LOGGER.info("Registered computer module '{}' with {} functions from {}",
                moduleName, methods.size(), clazz.getSimpleName());
    }

    @Nullable
    public static ModuleRegistration getModule(String name) {
        return modules.get(name);
    }

    public static Collection<ModuleRegistration> getAllModules() {
        return Collections.unmodifiableCollection(modules.values());
    }

    public static boolean hasModules() {
        return !modules.isEmpty();
    }

    /**
     * Returns JSON metadata of all registered modules for the WASM bridge.
     */
    public static String getMetadataJson() {
        StringBuilder sb = new StringBuilder();
        sb.append("{\"modules\":{");

        boolean firstModule = true;
        for (ModuleRegistration module : modules.values()) {
            if (!firstModule) sb.append(",");
            firstModule = false;

            sb.append("\"").append(jsonEscape(module.moduleName)).append("\":{");
            sb.append("\"description\":\"").append(jsonEscape(module.description)).append("\",");
            sb.append("\"functions\":{");

            boolean firstFunc = true;
            for (MethodRegistration method : module.methods.values()) {
                if (!firstFunc) sb.append(",");
                firstFunc = false;

                sb.append("\"").append(jsonEscape(method.pythonName)).append("\":{");
                sb.append("\"description\":\"").append(jsonEscape(method.description)).append("\",");
                sb.append("\"params\":[");
                for (int i = 0; i < method.params.length; i++) {
                    if (i > 0) sb.append(",");
                    sb.append("{\"name\":\"").append(jsonEscape(method.params[i].name)).append("\",");
                    sb.append("\"type\":\"").append(javaTypeToString(method.params[i].type)).append("\"}");
                }
                sb.append("],");
                sb.append("\"returns\":\"").append(javaTypeToString(method.returnType)).append("\"");
                sb.append("}");
            }

            sb.append("}}");
        }

        sb.append("}}");
        return sb.toString();
    }

    // ==================== Data Classes ====================

    public static class ModuleRegistration {
        public final String moduleName;
        public final String description;
        public final Object instance;
        public final Map<String, MethodRegistration> methods;

        public ModuleRegistration(String moduleName, String description, Object instance, Map<String, MethodRegistration> methods) {
            this.moduleName = moduleName;
            this.description = description;
            this.instance = instance;
            this.methods = methods;
        }
    }

    public static class MethodRegistration {
        public final String pythonName;
        public final String description;
        public final Method javaMethod;
        public final boolean mainThread;
        public final boolean needsContext;
        public final ParameterInfo[] params;
        public final Class<?> returnType;

        public MethodRegistration(String pythonName, String description, Method javaMethod,
                                  boolean mainThread, boolean needsContext, ParameterInfo[] params, Class<?> returnType) {
            this.pythonName = pythonName;
            this.description = description;
            this.javaMethod = javaMethod;
            this.mainThread = mainThread;
            this.needsContext = needsContext;
            this.params = params;
            this.returnType = returnType;
        }
    }

    public static class ParameterInfo {
        public final String name;
        public final Class<?> type;

        public ParameterInfo(String name, Class<?> type) {
            this.name = name;
            this.type = type;
        }
    }

    // ==================== Utilities ====================

    static String camelToSnake(String camel) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < camel.length(); i++) {
            char c = camel.charAt(i);
            if (Character.isUpperCase(c)) {
                if (i > 0) sb.append('_');
                sb.append(Character.toLowerCase(c));
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }

    private static String javaTypeToString(Class<?> type) {
        if (type == String.class) return "string";
        if (type == int.class || type == Integer.class) return "int";
        if (type == long.class || type == Long.class) return "int";
        if (type == float.class || type == Float.class) return "float";
        if (type == double.class || type == Double.class) return "float";
        if (type == boolean.class || type == Boolean.class) return "boolean";
        if (type == void.class || type == Void.class) return "void";
        if (List.class.isAssignableFrom(type)) return "list";
        if (Map.class.isAssignableFrom(type)) return "dict";
        return "object";
    }

    private static String jsonEscape(String s) {
        if (s == null) return "";
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r")
                .replace("\t", "\\t");
    }
}
