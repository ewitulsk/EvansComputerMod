package com.example.customworld.stubgen;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Container for all discovered peripheral definitions.
 * Supports JSON serialization for interchange between build phases.
 */
public class PeripheralDefinitions {
    private String generatedAt;
    private String version = "1.0";
    private List<PeripheralDefinition> peripherals = new ArrayList<>();
    
    public PeripheralDefinitions() {
        this.generatedAt = java.time.Instant.now().toString();
    }
    
    public String getGeneratedAt() { return generatedAt; }
    public void setGeneratedAt(String generatedAt) { this.generatedAt = generatedAt; }
    
    public String getVersion() { return version; }
    public void setVersion(String version) { this.version = version; }
    
    public List<PeripheralDefinition> getPeripherals() { return peripherals; }
    public void setPeripherals(List<PeripheralDefinition> peripherals) { this.peripherals = peripherals; }
    public void addPeripheral(PeripheralDefinition peripheral) { this.peripherals.add(peripheral); }
    
    /**
     * Serializes to JSON format.
     */
    public String toJson() {
        StringBuilder json = new StringBuilder();
        json.append("{\n");
        json.append("  \"generatedAt\": \"").append(escapeJson(generatedAt)).append("\",\n");
        json.append("  \"version\": \"").append(escapeJson(version)).append("\",\n");
        json.append("  \"peripherals\": [\n");
        
        for (int i = 0; i < peripherals.size(); i++) {
            PeripheralDefinition p = peripherals.get(i);
            json.append("    {\n");
            json.append("      \"type\": \"").append(escapeJson(p.getType())).append("\",\n");
            json.append("      \"className\": \"").append(escapeJson(p.getClassName())).append("\",\n");
            if (p.getDescription() != null) {
                json.append("      \"description\": \"").append(escapeJson(p.getDescription())).append("\",\n");
            }
            json.append("      \"methods\": [\n");
            
            List<MethodDefinition> methods = p.getMethods();
            for (int j = 0; j < methods.size(); j++) {
                MethodDefinition m = methods.get(j);
                json.append("        {\n");
                json.append("          \"javaName\": \"").append(escapeJson(m.getJavaName())).append("\",\n");
                json.append("          \"luaName\": \"").append(escapeJson(m.getLuaName())).append("\",\n");
                if (m.getDescription() != null) {
                    json.append("          \"description\": \"").append(escapeJson(m.getDescription())).append("\",\n");
                }
                json.append("          \"returnType\": \"").append(escapeJson(m.getReturnType() != null ? m.getReturnType() : "void")).append("\",\n");
                json.append("          \"mainThread\": ").append(m.isMainThread()).append(",\n");
                json.append("          \"parameters\": [\n");
                
                List<ParameterDefinition> params = m.getParameters();
                for (int k = 0; k < params.size(); k++) {
                    ParameterDefinition param = params.get(k);
                    json.append("            {\n");
                    json.append("              \"name\": \"").append(escapeJson(param.getName())).append("\",\n");
                    json.append("              \"type\": \"").append(escapeJson(param.getType())).append("\",\n");
                    json.append("              \"optional\": ").append(param.isOptional()).append("\n");
                    json.append("            }").append(k < params.size() - 1 ? "," : "").append("\n");
                }
                
                json.append("          ]\n");
                json.append("        }").append(j < methods.size() - 1 ? "," : "").append("\n");
            }
            
            json.append("      ]\n");
            json.append("    }").append(i < peripherals.size() - 1 ? "," : "").append("\n");
        }
        
        json.append("  ]\n");
        json.append("}\n");
        return json.toString();
    }
    
    /**
     * Saves to a JSON file.
     */
    public void saveToFile(Path path) throws IOException {
        Files.createDirectories(path.getParent());
        Files.writeString(path, toJson(), StandardCharsets.UTF_8);
    }
    
    /**
     * Loads from a JSON file.
     * Simple JSON parser - sufficient for our well-formed output.
     */
    public static PeripheralDefinitions loadFromFile(Path path) throws IOException {
        String json = Files.readString(path, StandardCharsets.UTF_8);
        return parseJson(json);
    }
    
    /**
     * Simple JSON parser for our peripheral definitions format.
     */
    public static PeripheralDefinitions parseJson(String json) {
        PeripheralDefinitions defs = new PeripheralDefinitions();
        
        // Extract peripherals array
        int peripheralsStart = json.indexOf("\"peripherals\":");
        if (peripheralsStart < 0) return defs;
        
        int arrayStart = json.indexOf('[', peripheralsStart);
        int arrayEnd = findMatchingBracket(json, arrayStart, '[', ']');
        if (arrayStart < 0 || arrayEnd < 0) return defs;
        
        String peripheralsJson = json.substring(arrayStart + 1, arrayEnd);
        
        // Parse each peripheral object
        int pos = 0;
        while (pos < peripheralsJson.length()) {
            int objStart = peripheralsJson.indexOf('{', pos);
            if (objStart < 0) break;
            int objEnd = findMatchingBracket(peripheralsJson, objStart, '{', '}');
            if (objEnd < 0) break;
            
            String objJson = peripheralsJson.substring(objStart, objEnd + 1);
            PeripheralDefinition peripheral = parsePeripheral(objJson);
            if (peripheral != null) {
                defs.addPeripheral(peripheral);
            }
            
            pos = objEnd + 1;
        }
        
        return defs;
    }
    
    private static PeripheralDefinition parsePeripheral(String json) {
        PeripheralDefinition p = new PeripheralDefinition();
        
        p.setType(extractStringValue(json, "type"));
        p.setClassName(extractStringValue(json, "className"));
        p.setDescription(extractStringValue(json, "description"));
        
        // Parse methods array
        int methodsStart = json.indexOf("\"methods\":");
        if (methodsStart >= 0) {
            int arrayStart = json.indexOf('[', methodsStart);
            int arrayEnd = findMatchingBracket(json, arrayStart, '[', ']');
            if (arrayStart >= 0 && arrayEnd >= 0) {
                String methodsJson = json.substring(arrayStart + 1, arrayEnd);
                parseMethodsInto(methodsJson, p);
            }
        }
        
        return p;
    }
    
    private static void parseMethodsInto(String methodsJson, PeripheralDefinition p) {
        int pos = 0;
        while (pos < methodsJson.length()) {
            int objStart = methodsJson.indexOf('{', pos);
            if (objStart < 0) break;
            int objEnd = findMatchingBracket(methodsJson, objStart, '{', '}');
            if (objEnd < 0) break;
            
            String objJson = methodsJson.substring(objStart, objEnd + 1);
            MethodDefinition method = parseMethod(objJson);
            if (method != null) {
                p.addMethod(method);
            }
            
            pos = objEnd + 1;
        }
    }
    
    private static MethodDefinition parseMethod(String json) {
        MethodDefinition m = new MethodDefinition();
        
        m.setJavaName(extractStringValue(json, "javaName"));
        m.setLuaName(extractStringValue(json, "luaName"));
        m.setReturnType(extractStringValue(json, "returnType"));
        m.setDescription(extractStringValue(json, "description"));
        m.setMainThread(extractBooleanValue(json, "mainThread"));
        
        // Parse parameters array
        int paramsStart = json.indexOf("\"parameters\":");
        if (paramsStart >= 0) {
            int arrayStart = json.indexOf('[', paramsStart);
            int arrayEnd = findMatchingBracket(json, arrayStart, '[', ']');
            if (arrayStart >= 0 && arrayEnd >= 0) {
                String paramsJson = json.substring(arrayStart + 1, arrayEnd);
                parseParametersInto(paramsJson, m);
            }
        }
        
        return m;
    }
    
    private static void parseParametersInto(String paramsJson, MethodDefinition m) {
        int pos = 0;
        while (pos < paramsJson.length()) {
            int objStart = paramsJson.indexOf('{', pos);
            if (objStart < 0) break;
            int objEnd = findMatchingBracket(paramsJson, objStart, '{', '}');
            if (objEnd < 0) break;
            
            String objJson = paramsJson.substring(objStart, objEnd + 1);
            ParameterDefinition param = new ParameterDefinition();
            param.setName(extractStringValue(objJson, "name"));
            param.setType(extractStringValue(objJson, "type"));
            param.setOptional(extractBooleanValue(objJson, "optional"));
            m.addParameter(param);
            
            pos = objEnd + 1;
        }
    }
    
    private static String extractStringValue(String json, String key) {
        String pattern = "\"" + key + "\":";
        int keyStart = json.indexOf(pattern);
        if (keyStart < 0) return null;
        
        int valueStart = json.indexOf('"', keyStart + pattern.length());
        if (valueStart < 0) return null;
        
        int valueEnd = valueStart + 1;
        while (valueEnd < json.length() && json.charAt(valueEnd) != '"') {
            if (json.charAt(valueEnd) == '\\') valueEnd++; // Skip escaped chars
            valueEnd++;
        }
        
        return unescapeJson(json.substring(valueStart + 1, valueEnd));
    }
    
    private static boolean extractBooleanValue(String json, String key) {
        String pattern = "\"" + key + "\":";
        int keyStart = json.indexOf(pattern);
        if (keyStart < 0) return false;
        
        int valueStart = keyStart + pattern.length();
        while (valueStart < json.length() && Character.isWhitespace(json.charAt(valueStart))) {
            valueStart++;
        }
        
        return json.regionMatches(valueStart, "true", 0, 4);
    }
    
    private static int findMatchingBracket(String str, int start, char open, char close) {
        if (start < 0 || start >= str.length()) return -1;
        int depth = 0;
        boolean inString = false;
        for (int i = start; i < str.length(); i++) {
            char c = str.charAt(i);
            if (c == '"' && (i == 0 || str.charAt(i - 1) != '\\')) {
                inString = !inString;
            } else if (!inString) {
                if (c == open) depth++;
                else if (c == close) {
                    depth--;
                    if (depth == 0) return i;
                }
            }
        }
        return -1;
    }
    
    private static String escapeJson(String s) {
        if (s == null) return "";
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", "\\n")
                .replace("\r", "\\r")
                .replace("\t", "\\t");
    }
    
    private static String unescapeJson(String s) {
        if (s == null) return null;
        return s.replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\");
    }
}
