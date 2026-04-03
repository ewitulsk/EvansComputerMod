package com.example.evanscomputermod.wasm;

import com.example.evanscomputermod.EvansComputerMod;

import org.jspecify.annotations.Nullable;
import java.lang.reflect.Array;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Converts between JSON and ComputerCraft Lua types for WASM integration.
 * Uses reflection to access CC APIs to maintain optional dependency status.
 * 
 * Type mappings:
 * - JSON null -> Lua nil
 * - JSON boolean -> Lua boolean
 * - JSON number -> Lua number (double)
 * - JSON string -> Lua string
 * - JSON array -> Lua table (array)
 * - JSON object -> Lua table (map)
 */
public class LuaWasmTypeConverter {
    
    private static Class<?> iArgumentsClass;
    private static Class<?> methodResultClass;
    private static Class<?> objectArgumentsClass;
    private static Method methodResultOfMethod;
    private static Method methodResultGetResultMethod;
    private static Constructor<?> objectArgumentsConstructor;
    
    static {
        try {
            iArgumentsClass = Class.forName("dan200.computercraft.api.lua.IArguments");
            methodResultClass = Class.forName("dan200.computercraft.api.lua.MethodResult");
            objectArgumentsClass = Class.forName("dan200.computercraft.api.lua.ObjectArguments");
            
            methodResultOfMethod = methodResultClass.getMethod("of", Object[].class);
            methodResultGetResultMethod = methodResultClass.getMethod("getResult");
            objectArgumentsConstructor = objectArgumentsClass.getConstructor(Object[].class);
            
            EvansComputerMod.LOGGER.debug("LuaWasmTypeConverter initialized successfully");
        } catch (Exception e) {
            EvansComputerMod.LOGGER.debug("CC:Tweaked Lua types not available: {}", e.getMessage());
        }
    }
    
    /**
     * Checks if type conversion is available.
     */
    public static boolean isAvailable() {
        return iArgumentsClass != null && objectArgumentsConstructor != null;
    }
    
    /**
     * Parses a JSON string into Java objects suitable for CC Lua calls.
     * Returns an array of objects that can be passed to IArguments.
     */
    public static Object[] parseJsonToArgs(String json) {
        if (json == null || json.trim().isEmpty() || json.equals("[]") || json.equals("null")) {
            return new Object[0];
        }
        
        try {
            json = json.trim();
            
            // Handle array of arguments
            if (json.startsWith("[")) {
                return parseJsonArray(json);
            }
            
            // Single argument
            return new Object[] { parseJsonValue(json) };
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Failed to parse JSON arguments: {}", e.getMessage());
            return new Object[0];
        }
    }
    
    /**
     * Parses a JSON array into an array of Java objects.
     */
    private static Object[] parseJsonArray(String json) {
        List<Object> result = new ArrayList<>();
        json = json.trim();
        
        // Remove outer brackets
        if (json.startsWith("[") && json.endsWith("]")) {
            json = json.substring(1, json.length() - 1).trim();
        }
        
        if (json.isEmpty()) {
            return new Object[0];
        }
        
        // Simple parser - handles basic cases
        int depth = 0;
        int start = 0;
        boolean inString = false;
        boolean escaped = false;
        
        for (int i = 0; i < json.length(); i++) {
            char c = json.charAt(i);
            
            if (escaped) {
                escaped = false;
                continue;
            }
            
            if (c == '\\') {
                escaped = true;
                continue;
            }
            
            if (c == '"' && depth == 0) {
                inString = !inString;
            }
            
            if (!inString) {
                if (c == '[' || c == '{') depth++;
                else if (c == ']' || c == '}') depth--;
                else if (c == ',' && depth == 0) {
                    String element = json.substring(start, i).trim();
                    result.add(parseJsonValue(element));
                    start = i + 1;
                }
            }
        }
        
        // Add last element
        if (start < json.length()) {
            String element = json.substring(start).trim();
            if (!element.isEmpty()) {
                result.add(parseJsonValue(element));
            }
        }
        
        return result.toArray();
    }
    
    /**
     * Parses a single JSON value into the appropriate Java type.
     */
    @Nullable
    private static Object parseJsonValue(String json) {
        if (json == null) return null;
        json = json.trim();
        
        if (json.isEmpty() || json.equals("null")) {
            return null;
        }
        
        if (json.equals("true")) {
            return Boolean.TRUE;
        }
        
        if (json.equals("false")) {
            return Boolean.FALSE;
        }
        
        // String
        if (json.startsWith("\"") && json.endsWith("\"")) {
            return unescapeJsonString(json.substring(1, json.length() - 1));
        }
        
        // Number
        if (json.matches("-?\\d+(\\.\\d+)?([eE][+-]?\\d+)?")) {
            try {
                if (json.contains(".") || json.contains("e") || json.contains("E")) {
                    return Double.parseDouble(json);
                } else {
                    long val = Long.parseLong(json);
                    if (val >= Integer.MIN_VALUE && val <= Integer.MAX_VALUE) {
                        return (int) val;
                    }
                    return val;
                }
            } catch (NumberFormatException e) {
                return json;
            }
        }
        
        // Array -> List (CC converts to table)
        if (json.startsWith("[")) {
            Object[] arr = parseJsonArray(json);
            List<Object> list = new ArrayList<>();
            for (Object obj : arr) {
                list.add(obj);
            }
            return list;
        }
        
        // Object -> Map (CC converts to table)
        if (json.startsWith("{")) {
            return parseJsonObject(json);
        }
        
        // Default to string
        return json;
    }
    
    /**
     * Parses a JSON object into a Map.
     */
    private static Map<String, Object> parseJsonObject(String json) {
        Map<String, Object> result = new HashMap<>();
        json = json.trim();
        
        if (json.startsWith("{") && json.endsWith("}")) {
            json = json.substring(1, json.length() - 1).trim();
        }
        
        if (json.isEmpty()) {
            return result;
        }
        
        int depth = 0;
        int start = 0;
        boolean inString = false;
        boolean escaped = false;
        
        for (int i = 0; i < json.length(); i++) {
            char c = json.charAt(i);
            
            if (escaped) {
                escaped = false;
                continue;
            }
            
            if (c == '\\') {
                escaped = true;
                continue;
            }
            
            if (c == '"' && depth == 0) {
                inString = !inString;
            }
            
            if (!inString) {
                if (c == '[' || c == '{') depth++;
                else if (c == ']' || c == '}') depth--;
                else if (c == ',' && depth == 0) {
                    parseKeyValue(json.substring(start, i).trim(), result);
                    start = i + 1;
                }
            }
        }
        
        if (start < json.length()) {
            parseKeyValue(json.substring(start).trim(), result);
        }
        
        return result;
    }
    
    /**
     * Parses a key:value pair into the result map.
     */
    private static void parseKeyValue(String kv, Map<String, Object> result) {
        int colonIdx = -1;
        boolean inString = false;
        boolean escaped = false;
        
        for (int i = 0; i < kv.length(); i++) {
            char c = kv.charAt(i);
            
            if (escaped) {
                escaped = false;
                continue;
            }
            
            if (c == '\\') {
                escaped = true;
                continue;
            }
            
            if (c == '"') {
                inString = !inString;
            }
            
            if (c == ':' && !inString) {
                colonIdx = i;
                break;
            }
        }
        
        if (colonIdx > 0) {
            String key = kv.substring(0, colonIdx).trim();
            String value = kv.substring(colonIdx + 1).trim();
            
            // Remove quotes from key
            if (key.startsWith("\"") && key.endsWith("\"")) {
                key = unescapeJsonString(key.substring(1, key.length() - 1));
            }
            
            result.put(key, parseJsonValue(value));
        }
    }
    
    /**
     * Unescapes a JSON string.
     */
    private static String unescapeJsonString(String s) {
        if (s == null) return null;
        
        StringBuilder result = new StringBuilder();
        boolean escaped = false;
        
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            
            if (escaped) {
                switch (c) {
                    case 'n' -> result.append('\n');
                    case 'r' -> result.append('\r');
                    case 't' -> result.append('\t');
                    case '\\' -> result.append('\\');
                    case '"' -> result.append('"');
                    case 'u' -> {
                        if (i + 4 < s.length()) {
                            try {
                                int code = Integer.parseInt(s.substring(i + 1, i + 5), 16);
                                result.append((char) code);
                                i += 4;
                            } catch (NumberFormatException e) {
                                result.append(c);
                            }
                        } else {
                            result.append(c);
                        }
                    }
                    default -> result.append(c);
                }
                escaped = false;
            } else if (c == '\\') {
                escaped = true;
            } else {
                result.append(c);
            }
        }
        
        return result.toString();
    }
    
    /**
     * Converts a CC MethodResult to a JSON response string.
     */
    public static String methodResultToJson(Object methodResult) {
        if (methodResult == null) {
            return "{\"ok\":true,\"result\":null}";
        }
        
        try {
            // Get the result array from MethodResult
            Object[] results = (Object[]) methodResultGetResultMethod.invoke(methodResult);
            
            if (results == null || results.length == 0) {
                return "{\"ok\":true,\"result\":null}";
            }
            
            StringBuilder json = new StringBuilder("{\"ok\":true,\"result\":");
            if (results.length == 1) {
                json.append(objectToJson(results[0]));
            } else {
                json.append("[");
                for (int i = 0; i < results.length; i++) {
                    if (i > 0) json.append(",");
                    json.append(objectToJson(results[i]));
                }
                json.append("]");
            }
            json.append("}");
            
            return json.toString();
        } catch (Exception e) {
            return "{\"ok\":false,\"error\":\"" + escapeJson(e.getMessage()) + "\"}";
        }
    }
    
    /**
     * Converts a Java object to JSON.
     */
    public static String objectToJson(Object obj) {
        if (obj == null) {
            return "null";
        }
        
        if (obj instanceof Boolean) {
            return obj.toString();
        }
        
        if (obj instanceof Number) {
            return obj.toString();
        }
        
        if (obj instanceof String s) {
            return "\"" + escapeJson(s) + "\"";
        }
        
        if (obj instanceof Map<?, ?> map) {
            StringBuilder json = new StringBuilder("{");
            boolean first = true;
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                if (!first) json.append(",");
                first = false;
                json.append("\"").append(escapeJson(String.valueOf(entry.getKey()))).append("\":");
                json.append(objectToJson(entry.getValue()));
            }
            json.append("}");
            return json.toString();
        }
        
        if (obj instanceof Iterable<?> iter) {
            StringBuilder json = new StringBuilder("[");
            boolean first = true;
            for (Object item : iter) {
                if (!first) json.append(",");
                first = false;
                json.append(objectToJson(item));
            }
            json.append("]");
            return json.toString();
        }
        
        if (obj.getClass().isArray()) {
            StringBuilder json = new StringBuilder("[");
            int length = Array.getLength(obj);
            for (int i = 0; i < length; i++) {
                if (i > 0) json.append(",");
                json.append(objectToJson(Array.get(obj, i)));
            }
            json.append("]");
            return json.toString();
        }
        
        // Default: convert to string
        return "\"" + escapeJson(obj.toString()) + "\"";
    }
    
    /**
     * Creates an IArguments instance from an array of objects.
     */
    @Nullable
    public static Object createArguments(Object[] args) {
        if (!isAvailable()) {
            return null;
        }
        
        try {
            return objectArgumentsConstructor.newInstance((Object) args);
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Failed to create IArguments: {}", e.getMessage());
            return null;
        }
    }
    
    /**
     * Creates an error response JSON.
     */
    public static String errorJson(String message) {
        return "{\"ok\":false,\"error\":\"" + escapeJson(message) + "\"}";
    }
    
    /**
     * Creates a success response JSON with result.
     */
    public static String successJson(Object result) {
        return "{\"ok\":true,\"result\":" + objectToJson(result) + "}";
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
