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
                            inputs.add(new PortDef(
                                    portObj.get("name").getAsString(),
                                    portObj.get("type").getAsString(),
                                    portObj.has("default") ? portObj.get("default").getAsString() : null
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

    public record PortDef(String name, String type, String defaultValue) {
        public boolean isFlow() {
            return "flow".equals(type);
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
}
