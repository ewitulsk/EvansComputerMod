package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
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
        }
        return categories;
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
