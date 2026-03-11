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
                    blocks.add(new BlockDef(
                            blockObj.get("name").getAsString(),
                            blockObj.get("label").getAsString()
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

    public record BlockDef(String name, String label) {}

    public record Category(String name, int color, List<BlockDef> blocks) {}
}
