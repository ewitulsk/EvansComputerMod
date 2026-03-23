package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.ComputerModuleRegistry;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * Loads visual programming block definitions from the blocks.json resource file.
 */
public class VisualBlockRegistry {

    private static final List<Category> categories = new ArrayList<>();
    private static boolean loaded = false;

    public static List<Category> getCategories() {
        if (!loaded) {
            load();
            generateBlocksFromModules();
        }
        return categories;
    }

    /**
     * Adds a dynamically created category to the registry.
     */
    public static void registerDynamic(Category category) {
        getCategories(); // ensure loaded
        categories.add(category);
    }

    /**
     * Auto-generates visual programming blocks from {@link ComputerModuleRegistry}.
     * Creates one category per module with blocks for each registered function.
     */
    private static void generateBlocksFromModules() {
        if (!ComputerModuleRegistry.hasModules()) {
            return;
        }

        for (ComputerModuleRegistry.ModuleRegistration module : ComputerModuleRegistry.getAllModules()) {
            List<BlockDef> blocks = new ArrayList<>();

            for (ComputerModuleRegistry.MethodRegistration method : module.methods.values()) {
                // Build inputs: flow + one port per parameter
                List<PortDef> inputs = new ArrayList<>();
                inputs.add(new PortDef("flow", "flow", null));
                for (ComputerModuleRegistry.ParameterInfo param : method.params) {
                    String portType = javaTypeToPortType(param.type);
                    inputs.add(new PortDef(param.name, portType, getDefaultForType(param.type)));
                }

                // Build outputs: flow + result (if non-void)
                List<PortDef> outputs = new ArrayList<>();
                outputs.add(new PortDef("flow", "flow", null));
                if (method.returnType != void.class && method.returnType != Void.class) {
                    outputs.add(new PortDef("result", javaTypeToPortType(method.returnType), null));
                }

                // Build code template
                String code = buildCodeTemplate(module.moduleName, method);

                // Block name and label
                String blockName = module.moduleName + "_" + method.pythonName;
                String label = method.description.isEmpty()
                        ? capitalizeLabel(method.pythonName)
                        : method.description;

                blocks.add(new BlockDef(blockName, label, inputs, outputs, code, null));
            }

            if (!blocks.isEmpty()) {
                int color = generateColor(module.moduleName);
                String categoryName = capitalize(module.moduleName);
                categories.add(new Category(categoryName, color, blocks));
                EvansComputerMod.LOGGER.info("Generated {} visual blocks for module '{}'",
                        blocks.size(), module.moduleName);
            }
        }
    }

    private static String javaTypeToPortType(Class<?> type) {
        if (type == String.class) return "string";
        if (type == int.class || type == Integer.class) return "number";
        if (type == long.class || type == Long.class) return "number";
        if (type == float.class || type == Float.class) return "number";
        if (type == double.class || type == Double.class) return "number";
        if (type == boolean.class || type == Boolean.class) return "boolean";
        return "string";
    }

    private static String getDefaultForType(Class<?> type) {
        if (type == int.class || type == Integer.class) return "0";
        if (type == long.class || type == Long.class) return "0";
        if (type == float.class || type == Float.class) return "0.0";
        if (type == double.class || type == Double.class) return "0.0";
        if (type == boolean.class || type == Boolean.class) return "False";
        return "";
    }

    private static String buildCodeTemplate(String moduleName, ComputerModuleRegistry.MethodRegistration method) {
        StringBuilder sb = new StringBuilder();
        boolean hasReturn = method.returnType != void.class && method.returnType != Void.class;

        if (hasReturn) {
            sb.append("{result} = ");
        }
        sb.append(moduleName).append(".").append(method.pythonName).append("(");
        for (int i = 0; i < method.params.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append("{").append(method.params[i].name).append("}");
        }
        sb.append(")");
        return sb.toString();
    }

    private static String capitalizeLabel(String snakeCase) {
        StringBuilder sb = new StringBuilder();
        for (String part : snakeCase.split("_")) {
            if (!part.isEmpty()) {
                if (sb.length() > 0) sb.append(" ");
                sb.append(Character.toUpperCase(part.charAt(0)));
                sb.append(part.substring(1));
            }
        }
        return sb.toString();
    }

    private static String capitalize(String s) {
        if (s == null || s.isEmpty()) return s;
        return Character.toUpperCase(s.charAt(0)) + s.substring(1);
    }

    private static int generateColor(String moduleName) {
        // Generate a deterministic color from the module name hash
        float hue = (Math.abs(moduleName.hashCode()) % 360) / 360.0f;
        float saturation = 0.6f;
        float brightness = 0.8f;

        // Manual HSB to RGB conversion (avoids java.awt dependency)
        int hi = (int) (hue * 6) % 6;
        float f = hue * 6 - hi;
        float p = brightness * (1 - saturation);
        float q = brightness * (1 - f * saturation);
        float t = brightness * (1 - (1 - f) * saturation);
        float r, g, b;
        switch (hi) {
            case 0: r = brightness; g = t; b = p; break;
            case 1: r = q; g = brightness; b = p; break;
            case 2: r = p; g = brightness; b = t; break;
            case 3: r = p; g = q; b = brightness; break;
            case 4: r = t; g = p; b = brightness; break;
            default: r = brightness; g = p; b = q; break;
        }
        return 0xFF000000 | ((int)(r * 255) << 16) | ((int)(g * 255) << 8) | (int)(b * 255);
    }

    private static void load() {
        loaded = true;
        categories.clear();

        try (InputStream is = VisualBlockRegistry.class.getResourceAsStream(
                "/assets/evanscomputermod/visual/blocks.json")) {
            if (is == null) {
                EvansComputerMod.LOGGER.error("Could not find visual/blocks.json");
                return;
            }

            JsonObject root = JsonParser.parseReader(
                    new InputStreamReader(is, StandardCharsets.UTF_8)).getAsJsonObject();
            JsonArray cats = root.getAsJsonArray("categories");

            for (JsonElement catEl : cats) {
                JsonObject catObj = catEl.getAsJsonObject();
                String name = catObj.get("name").getAsString();
                int color = parseColor(catObj.get("color").getAsString());

                List<BlockDef> blocks = new ArrayList<>();
                for (JsonElement blockEl : catObj.getAsJsonArray("blocks")) {
                    JsonObject blockObj = blockEl.getAsJsonObject();

                    List<PortDef> inputs = new ArrayList<>();
                    if (blockObj.has("inputs")) {
                        for (JsonElement portEl : blockObj.getAsJsonArray("inputs")) {
                            JsonObject portObj = portEl.getAsJsonObject();
                            List<String> options = null;
                            if (portObj.has("options")) {
                                options = new ArrayList<>();
                                for (JsonElement optEl : portObj.getAsJsonArray("options")) {
                                    options.add(optEl.getAsString());
                                }
                            }
                            inputs.add(new PortDef(
                                    portObj.get("name").getAsString(),
                                    portObj.get("type").getAsString(),
                                    portObj.has("default") ? portObj.get("default").getAsString() : null,
                                    options
                            ));
                        }
                    }

                    List<PortDef> outputs = new ArrayList<>();
                    if (blockObj.has("outputs")) {
                        for (JsonElement portEl : blockObj.getAsJsonArray("outputs")) {
                            JsonObject portObj = portEl.getAsJsonObject();
                            outputs.add(new PortDef(
                                    portObj.get("name").getAsString(),
                                    portObj.get("type").getAsString(),
                                    null
                            ));
                        }
                    }

                    String codeTemplate = blockObj.has("code") ? blockObj.get("code").getAsString() : "";
                    String generator = blockObj.has("generator") ? blockObj.get("generator").getAsString() : null;

                    blocks.add(new BlockDef(
                            blockObj.get("name").getAsString(),
                            blockObj.get("label").getAsString(),
                            inputs,
                            outputs,
                            codeTemplate,
                            generator
                    ));
                }
                categories.add(new Category(name, color, blocks));
            }

            EvansComputerMod.LOGGER.info("Loaded {} visual block categories", categories.size());
        } catch (Exception e) {
            EvansComputerMod.LOGGER.error("Failed to load visual/blocks.json", e);
        }
    }

    private static int parseColor(String hex) {
        hex = hex.replace("#", "");
        return 0xFF000000 | Integer.parseInt(hex, 16);
    }

    public record PortDef(String name, String type, String defaultValue, List<String> options) {
        /** Convenience constructor without options. */
        public PortDef(String name, String type, String defaultValue) {
            this(name, type, defaultValue, null);
        }

        public boolean isFlow() {
            return "flow".equals(type);
        }

        /** Whether this port has predefined options (click-to-cycle dropdown). */
        public boolean hasOptions() {
            return options != null && !options.isEmpty();
        }

        /** Returns the display label for a stored value, or the value itself if not found. */
        public String labelForValue(String value) {
            if (options == null) return value;
            for (String opt : options) {
                int sep = opt.indexOf(':');
                if (sep >= 0 && opt.substring(sep + 1).equals(value)) {
                    return opt.substring(0, sep);
                }
            }
            return value;
        }

        /** Cycles to the next option value. Returns the first option if current value isn't found. */
        public String nextOptionValue(String currentValue) {
            if (options == null || options.isEmpty()) return currentValue;
            for (int i = 0; i < options.size(); i++) {
                int sep = options.get(i).indexOf(':');
                String val = sep >= 0 ? options.get(i).substring(sep + 1) : options.get(i);
                if (val.equals(currentValue)) {
                    String next = options.get((i + 1) % options.size());
                    int nextSep = next.indexOf(':');
                    return nextSep >= 0 ? next.substring(nextSep + 1) : next;
                }
            }
            // Current value not found — return first option's value
            String first = options.get(0);
            int sep = first.indexOf(':');
            return sep >= 0 ? first.substring(sep + 1) : first;
        }
    }

    public record BlockDef(String name, String label, List<PortDef> inputs, List<PortDef> outputs, String codeTemplate, String generator) {
        /** Returns the generator name for code generation dispatch. Falls back to block name. */
        public String generatorOrName() {
            return generator != null ? generator : name;
        }

        /** Returns data-only inputs (excludes flow ports). */
        public List<PortDef> dataInputs() {
            return inputs.stream().filter(p -> !p.isFlow()).toList();
        }

        /** Returns data-only outputs (excludes flow ports). */
        public List<PortDef> dataOutputs() {
            return outputs.stream().filter(p -> !p.isFlow()).toList();
        }

        public boolean hasFlowIn() {
            return inputs.stream().anyMatch(PortDef::isFlow);
        }

        public boolean hasFlowOut() {
            return outputs.stream().anyMatch(PortDef::isFlow);
        }

        /** Returns flow-type output ports. */
        public List<PortDef> flowOutputs() {
            return outputs.stream().filter(PortDef::isFlow).toList();
        }

        /** Returns flow-type input ports. */
        public List<PortDef> flowInputs() {
            return inputs.stream().filter(PortDef::isFlow).toList();
        }
    }

    public record Category(String name, int color, List<BlockDef> blocks) {}

    public static BlockDef findBlockByName(String name) {
        for (Category cat : getCategories()) {
            for (BlockDef b : cat.blocks()) {
                if (b.name().equals(name)) return b;
            }
        }
        return null;
    }

    public static int getColorForBlock(String blockName) {
        for (Category cat : getCategories()) {
            for (BlockDef b : cat.blocks()) {
                if (b.name().equals(blockName)) return cat.color();
            }
        }
        return 0xFF888888;
    }
}
