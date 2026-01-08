package com.example.customworld.stubgen;

import java.util.ArrayList;
import java.util.List;

/**
 * Represents a method exposed by a peripheral via @LuaFunction.
 */
public class MethodDefinition {
    private String javaName;        // Original Java method name
    private String luaName;         // Name exposed to Lua (may differ via annotation value)
    private List<ParameterDefinition> parameters = new ArrayList<>();
    private String returnType;
    private String description;     // Optional docstring
    private boolean mainThread;     // Whether method must run on main thread
    
    public MethodDefinition() {}
    
    public MethodDefinition(String javaName, String luaName) {
        this.javaName = javaName;
        this.luaName = luaName;
    }
    
    public String getJavaName() { return javaName; }
    public void setJavaName(String javaName) { this.javaName = javaName; }
    
    public String getLuaName() { return luaName; }
    public void setLuaName(String luaName) { this.luaName = luaName; }
    
    public List<ParameterDefinition> getParameters() { return parameters; }
    public void setParameters(List<ParameterDefinition> parameters) { this.parameters = parameters; }
    public void addParameter(ParameterDefinition param) { this.parameters.add(param); }
    
    public String getReturnType() { return returnType; }
    public void setReturnType(String returnType) { this.returnType = returnType; }
    
    public String getDescription() { return description; }
    public void setDescription(String description) { this.description = description; }
    
    public boolean isMainThread() { return mainThread; }
    public void setMainThread(boolean mainThread) { this.mainThread = mainThread; }
    
    /**
     * Converts the Lua name to Python snake_case.
     */
    public String toPythonName() {
        return camelToSnake(luaName);
    }
    
    /**
     * Converts the Lua name to Rust snake_case.
     */
    public String toRustName() {
        return camelToSnake(luaName);
    }
    
    /**
     * Gets the Python return type hint.
     */
    public String toPythonReturnType() {
        if (returnType == null || returnType.equals("void")) {
            return "None";
        }
        return switch (returnType) {
            case "int", "long", "Integer", "Long" -> "int";
            case "double", "float", "Double", "Float" -> "float";
            case "boolean", "Boolean" -> "bool";
            case "String" -> "str";
            case "int[]", "Integer[]", "List<Integer>", "String[]", "List<String>" -> "list";
            case "Map", "HashMap", "table" -> "dict";
            case "Object", "MethodResult" -> "any";
            default -> "any";
        };
    }
    
    /**
     * Gets the Rust return type.
     */
    public String toRustReturnType() {
        if (returnType == null || returnType.equals("void")) {
            return "()";
        }
        return switch (returnType) {
            case "int", "Integer" -> "i32";
            case "long", "Long" -> "i64";
            case "double", "Double" -> "f64";
            case "float", "Float" -> "f32";
            case "boolean", "Boolean" -> "bool";
            case "String" -> "String";
            case "int[]", "Integer[]", "List<Integer>" -> "Vec<i32>";
            case "String[]", "List<String>" -> "Vec<String>";
            case "Map", "HashMap", "table" -> "std::collections::HashMap<String, String>";
            case "Object", "MethodResult" -> "String"; // Return raw JSON
            default -> "String";
        };
    }
    
    private static String camelToSnake(String camel) {
        if (camel == null || camel.isEmpty()) return camel;
        StringBuilder result = new StringBuilder();
        for (int i = 0; i < camel.length(); i++) {
            char c = camel.charAt(i);
            if (Character.isUpperCase(c)) {
                if (i > 0) result.append('_');
                result.append(Character.toLowerCase(c));
            } else {
                result.append(c);
            }
        }
        return result.toString();
    }
    
    @Override
    public String toString() {
        return "MethodDefinition{luaName='" + luaName + "', params=" + parameters.size() + ", returns='" + returnType + "'}";
    }
}
