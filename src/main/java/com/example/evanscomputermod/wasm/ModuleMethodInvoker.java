package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.*;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import net.minecraft.server.MinecraftServer;

/**
 * Invokes methods registered via {@link ComputerModule} annotations.
 * Handles JSON argument parsing, ComputerContext injection, type conversion,
 * and main-thread scheduling.
 */
public class ModuleMethodInvoker {

    /**
     * Invokes a registered module method.
     *
     * @param host       The computer host (for ComputerContext)
     * @param moduleName The Python module name
     * @param methodName The Python function name
     * @param argsJson   JSON array of arguments
     * @return JSON result string
     */
    public String invokeMethod(IComputerHost host, String moduleName, String methodName, String argsJson) {
        ComputerModuleRegistry.ModuleRegistration module = ComputerModuleRegistry.getModule(moduleName);
        if (module == null) {
            return errorJson("Module not found: " + moduleName);
        }

        ComputerModuleRegistry.MethodRegistration methodReg = module.methods.get(methodName);
        if (methodReg == null) {
            return errorJson("Method not found: " + moduleName + "." + methodName);
        }

        try {
            // Parse JSON arguments
            Object[] rawArgs = parseJsonArgs(argsJson);

            // Build actual argument array
            Method javaMethod = methodReg.javaMethod;
            Object[] callArgs;

            if (methodReg.needsContext) {
                callArgs = new Object[methodReg.params.length + 1];
                callArgs[0] = new ComputerContext(host);
                for (int i = 0; i < methodReg.params.length; i++) {
                    callArgs[i + 1] = convertArg(i < rawArgs.length ? rawArgs[i] : null, methodReg.params[i].type);
                }
            } else {
                callArgs = new Object[methodReg.params.length];
                for (int i = 0; i < methodReg.params.length; i++) {
                    callArgs[i] = convertArg(i < rawArgs.length ? rawArgs[i] : null, methodReg.params[i].type);
                }
            }

            // Execute (optionally on main thread)
            Object result;
            if (methodReg.mainThread) {
                result = executeOnMainThread(host, module.instance, javaMethod, callArgs);
            } else {
                result = javaMethod.invoke(module.instance, callArgs);
            }

            return successJson(result);

        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            EvansComputerMod.LOGGER.error("Error invoking {}.{}", moduleName, methodName, cause);
            return errorJson(cause != null ? cause.getMessage() : e.getMessage());
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Error invoking {}.{}", moduleName, methodName, e);
            return errorJson(e.getMessage());
        }
    }

    private Object executeOnMainThread(IComputerHost host, Object instance, Method method, Object[] args)
            throws Exception {
        MinecraftServer server = host.getServer();
        if (server == null) {
            // No server available, execute directly
            return method.invoke(instance, args);
        }

        CompletableFuture<Object> future = new CompletableFuture<>();
        server.execute(() -> {
            try {
                future.complete(method.invoke(instance, args));
            } catch (Exception e) {
                future.completeExceptionally(e);
            }
        });

        try {
            return future.get(10, TimeUnit.SECONDS);
        } catch (ExecutionException e) {
            throw (Exception) e.getCause();
        } catch (TimeoutException e) {
            throw new RuntimeException("Main thread execution timed out");
        }
    }

    // ==================== JSON Argument Parsing ====================

    /**
     * Parses a JSON array string into an Object array.
     * Supports strings, numbers, booleans, and null.
     */
    static Object[] parseJsonArgs(String json) {
        if (json == null || json.isEmpty()) return new Object[0];
        json = json.trim();
        if (!json.startsWith("[") || !json.endsWith("]")) return new Object[0];

        String inner = json.substring(1, json.length() - 1).trim();
        if (inner.isEmpty()) return new Object[0];

        List<Object> args = new ArrayList<>();
        int i = 0;
        while (i < inner.length()) {
            char c = inner.charAt(i);
            if (c == ' ' || c == ',') {
                i++;
                continue;
            }

            if (c == '"') {
                // String value
                int end = findStringEnd(inner, i);
                args.add(unescapeJsonString(inner.substring(i + 1, end)));
                i = end + 1;
            } else if (c == 't' && inner.startsWith("true", i)) {
                args.add(Boolean.TRUE);
                i += 4;
            } else if (c == 'f' && inner.startsWith("false", i)) {
                args.add(Boolean.FALSE);
                i += 5;
            } else if (c == 'n' && inner.startsWith("null", i)) {
                args.add(null);
                i += 4;
            } else if (c == '-' || (c >= '0' && c <= '9')) {
                // Number
                int end = i + 1;
                boolean isFloat = false;
                while (end < inner.length()) {
                    char nc = inner.charAt(end);
                    if (nc == '.' || nc == 'e' || nc == 'E') isFloat = true;
                    if (nc != '.' && nc != '-' && nc != '+' && nc != 'e' && nc != 'E' && (nc < '0' || nc > '9')) break;
                    end++;
                }
                String numStr = inner.substring(i, end);
                if (isFloat) {
                    args.add(Double.parseDouble(numStr));
                } else {
                    long val = Long.parseLong(numStr);
                    if (val >= Integer.MIN_VALUE && val <= Integer.MAX_VALUE) {
                        args.add((int) val);
                    } else {
                        args.add(val);
                    }
                }
                i = end;
            } else {
                i++;
            }
        }

        return args.toArray();
    }

    private static int findStringEnd(String s, int start) {
        for (int i = start + 1; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\\') {
                i++; // skip escaped char
            } else if (c == '"') {
                return i;
            }
        }
        return s.length() - 1;
    }

    private static String unescapeJsonString(String s) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\\' && i + 1 < s.length()) {
                char next = s.charAt(i + 1);
                switch (next) {
                    case '"': sb.append('"'); i++; break;
                    case '\\': sb.append('\\'); i++; break;
                    case 'n': sb.append('\n'); i++; break;
                    case 'r': sb.append('\r'); i++; break;
                    case 't': sb.append('\t'); i++; break;
                    default: sb.append(c); break;
                }
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }

    // ==================== Type Conversion ====================

    static Object convertArg(Object value, Class<?> targetType) {
        if (value == null) {
            if (targetType.isPrimitive()) {
                return getDefaultPrimitive(targetType);
            }
            return null;
        }

        if (targetType == String.class) {
            return value.toString();
        }
        if (targetType == int.class || targetType == Integer.class) {
            if (value instanceof Number) return ((Number) value).intValue();
            return Integer.parseInt(value.toString());
        }
        if (targetType == long.class || targetType == Long.class) {
            if (value instanceof Number) return ((Number) value).longValue();
            return Long.parseLong(value.toString());
        }
        if (targetType == float.class || targetType == Float.class) {
            if (value instanceof Number) return ((Number) value).floatValue();
            return Float.parseFloat(value.toString());
        }
        if (targetType == double.class || targetType == Double.class) {
            if (value instanceof Number) return ((Number) value).doubleValue();
            return Double.parseDouble(value.toString());
        }
        if (targetType == boolean.class || targetType == Boolean.class) {
            if (value instanceof Boolean) return value;
            return Boolean.parseBoolean(value.toString());
        }

        return value;
    }

    private static Object getDefaultPrimitive(Class<?> type) {
        if (type == int.class) return 0;
        if (type == long.class) return 0L;
        if (type == float.class) return 0.0f;
        if (type == double.class) return 0.0;
        if (type == boolean.class) return false;
        return 0;
    }

    // ==================== JSON Result Serialization ====================

    static String successJson(Object result) {
        StringBuilder sb = new StringBuilder();
        sb.append("{\"ok\":true,\"result\":");
        appendJsonValue(sb, result);
        sb.append("}");
        return sb.toString();
    }

    static String errorJson(String message) {
        return "{\"ok\":false,\"error\":\"" + jsonEscape(message) + "\"}";
    }

    private static void appendJsonValue(StringBuilder sb, Object value) {
        if (value == null) {
            sb.append("null");
        } else if (value instanceof String) {
            sb.append("\"").append(jsonEscape((String) value)).append("\"");
        } else if (value instanceof Boolean) {
            sb.append(value);
        } else if (value instanceof Number) {
            sb.append(value);
        } else if (value instanceof List<?> list) {
            sb.append("[");
            for (int i = 0; i < list.size(); i++) {
                if (i > 0) sb.append(",");
                appendJsonValue(sb, list.get(i));
            }
            sb.append("]");
        } else if (value instanceof Map<?, ?> map) {
            sb.append("{");
            boolean first = true;
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                if (!first) sb.append(",");
                first = false;
                sb.append("\"").append(jsonEscape(entry.getKey().toString())).append("\":");
                appendJsonValue(sb, entry.getValue());
            }
            sb.append("}");
        } else {
            sb.append("\"").append(jsonEscape(value.toString())).append("\"");
        }
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
