package com.example.customworld.stubgen;

import java.util.ArrayList;
import java.util.List;

/**
 * Represents a CC:Tweaked peripheral with its type and available methods.
 */
public class PeripheralDefinition {
    private String type;            // Peripheral type (e.g., "playerDetector")
    private String className;       // Fully qualified Java class name
    private String description;     // Optional description
    private List<MethodDefinition> methods = new ArrayList<>();
    
    public PeripheralDefinition() {}
    
    public PeripheralDefinition(String type, String className) {
        this.type = type;
        this.className = className;
    }
    
    public String getType() { return type; }
    public void setType(String type) { this.type = type; }
    
    public String getClassName() { return className; }
    public void setClassName(String className) { this.className = className; }
    
    public String getDescription() { return description; }
    public void setDescription(String description) { this.description = description; }
    
    public List<MethodDefinition> getMethods() { return methods; }
    public void setMethods(List<MethodDefinition> methods) { this.methods = methods; }
    public void addMethod(MethodDefinition method) { this.methods.add(method); }
    
    /**
     * Converts the peripheral type to a Python class name (PascalCase).
     */
    public String toPythonClassName() {
        return snakeToPascal(type);
    }
    
    /**
     * Converts the peripheral type to a Python module name (snake_case).
     */
    public String toPythonModuleName() {
        return camelToSnake(type);
    }
    
    /**
     * Converts the peripheral type to a Rust struct name (PascalCase).
     */
    public String toRustStructName() {
        return snakeToPascal(type);
    }
    
    /**
     * Converts the peripheral type to a Rust module name (snake_case).
     */
    public String toRustModuleName() {
        return camelToSnake(type);
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
    
    private static String snakeToPascal(String snake) {
        if (snake == null || snake.isEmpty()) return snake;
        StringBuilder result = new StringBuilder();
        boolean capitalizeNext = true;
        for (int i = 0; i < snake.length(); i++) {
            char c = snake.charAt(i);
            if (c == '_') {
                capitalizeNext = true;
            } else if (capitalizeNext) {
                result.append(Character.toUpperCase(c));
                capitalizeNext = false;
            } else {
                result.append(c);
            }
        }
        return result.toString();
    }
    
    @Override
    public String toString() {
        return "PeripheralDefinition{type='" + type + "', methods=" + methods.size() + "}";
    }
}
