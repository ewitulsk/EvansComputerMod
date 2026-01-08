package com.example.customworld.stubgen;

/**
 * Represents a parameter in a peripheral method.
 */
public class ParameterDefinition {
    private String name;
    private String type;
    private boolean optional;
    
    public ParameterDefinition() {}
    
    public ParameterDefinition(String name, String type, boolean optional) {
        this.name = name;
        this.type = type;
        this.optional = optional;
    }
    
    public String getName() { return name; }
    public void setName(String name) { this.name = name; }
    
    public String getType() { return type; }
    public void setType(String type) { this.type = type; }
    
    public boolean isOptional() { return optional; }
    public void setOptional(boolean optional) { this.optional = optional; }
    
    /**
     * Converts this parameter to a Python type hint.
     */
    public String toPythonType() {
        String pyType = switch (type) {
            case "int", "long", "Integer", "Long" -> "int";
            case "double", "float", "Double", "Float" -> "float";
            case "boolean", "Boolean" -> "bool";
            case "String" -> "str";
            case "int[]", "Integer[]", "List<Integer>" -> "list";
            case "String[]", "List<String>" -> "list";
            case "Map", "HashMap", "table" -> "dict";
            default -> "any";
        };
        return optional ? "Optional[" + pyType + "]" : pyType;
    }
    
    /**
     * Converts this parameter to a Rust type.
     */
    public String toRustType() {
        String rustType = switch (type) {
            case "int", "Integer" -> "i32";
            case "long", "Long" -> "i64";
            case "double", "Double" -> "f64";
            case "float", "Float" -> "f32";
            case "boolean", "Boolean" -> "bool";
            case "String" -> "String";
            case "int[]", "Integer[]", "List<Integer>" -> "Vec<i32>";
            case "String[]", "List<String>" -> "Vec<String>";
            case "Map", "HashMap", "table" -> "std::collections::HashMap<String, String>";
            default -> "String"; // Default to string for unknown types
        };
        return optional ? "Option<" + rustType + ">" : rustType;
    }
    
    /**
     * Converts the parameter name to snake_case for Python/Rust.
     */
    public String toSnakeCaseName() {
        return camelToSnake(name);
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
        return "ParameterDefinition{name='" + name + "', type='" + type + "', optional=" + optional + "}";
    }
}
