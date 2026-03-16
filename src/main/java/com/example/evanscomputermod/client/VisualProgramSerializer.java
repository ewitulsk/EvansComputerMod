package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.VisualBlockRegistry.BlockDef;
import com.example.evanscomputermod.client.VisualBlockRegistry.PortDef;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class VisualProgramSerializer {

    public record SerializedBlock(int id, BlockDef definition, int color, float x, float y, Map<String, String> inputValues) {}
    public record SerializedConnection(int fromBlockId, String fromPort, int toBlockId, String toPort) {}

    public record DeserializedProgram(
            List<SerializedBlock> blocks,
            List<SerializedConnection> connections,
            float canvasOffsetX, float canvasOffsetY, float zoom,
            String name,
            int newNextBlockId
    ) {}

    public interface BlockInfo {
        int id();
        BlockDef definition();
        float x();
        float y();
        Map<String, String> inputValues();
    }

    public interface ConnectionInfo {
        int fromBlockId();
        String fromPort();
        int toBlockId();
        String toPort();
    }

    public static String serialize(List<? extends BlockInfo> blocks, List<? extends ConnectionInfo> connections,
                                   float canvasOffsetX, float canvasOffsetY, float zoom, String name) {
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("name", name);

        JsonObject canvas = new JsonObject();
        canvas.addProperty("offsetX", canvasOffsetX);
        canvas.addProperty("offsetY", canvasOffsetY);
        canvas.addProperty("zoom", zoom);
        root.add("canvas", canvas);

        JsonArray blocksArr = new JsonArray();
        for (BlockInfo block : blocks) {
            JsonObject blockObj = new JsonObject();
            blockObj.addProperty("id", block.id());
            blockObj.addProperty("type", block.definition().name());
            blockObj.addProperty("x", block.x());
            blockObj.addProperty("y", block.y());

            JsonObject inputs = new JsonObject();
            for (Map.Entry<String, String> entry : block.inputValues().entrySet()) {
                // Only save non-default values
                PortDef portDef = null;
                for (PortDef p : block.definition().dataInputs()) {
                    if (p.name().equals(entry.getKey())) {
                        portDef = p;
                        break;
                    }
                }
                if (portDef == null || !entry.getValue().equals(portDef.defaultValue())) {
                    inputs.addProperty(entry.getKey(), entry.getValue());
                }
            }
            blockObj.add("inputs", inputs);
            blocksArr.add(blockObj);
        }
        root.add("blocks", blocksArr);

        JsonArray connsArr = new JsonArray();
        for (ConnectionInfo conn : connections) {
            JsonObject connObj = new JsonObject();
            connObj.addProperty("fromBlock", conn.fromBlockId());
            connObj.addProperty("fromPort", conn.fromPort());
            connObj.addProperty("toBlock", conn.toBlockId());
            connObj.addProperty("toPort", conn.toPort());
            connsArr.add(connObj);
        }
        root.add("connections", connsArr);

        return root.toString();
    }

    public static DeserializedProgram deserialize(String json, int startingNextBlockId) {
        JsonObject root = JsonParser.parseString(json).getAsJsonObject();

        String name = root.has("name") ? root.get("name").getAsString() : "untitled";

        float offsetX = 0, offsetY = 0, zoom = 1.0f;
        if (root.has("canvas")) {
            JsonObject canvas = root.getAsJsonObject("canvas");
            offsetX = canvas.has("offsetX") ? canvas.get("offsetX").getAsFloat() : 0;
            offsetY = canvas.has("offsetY") ? canvas.get("offsetY").getAsFloat() : 0;
            zoom = canvas.has("zoom") ? canvas.get("zoom").getAsFloat() : 1.0f;
        }

        Map<Integer, Integer> idRemap = new HashMap<>();
        List<SerializedBlock> blocks = new ArrayList<>();
        int nextId = startingNextBlockId;

        if (root.has("blocks")) {
            for (JsonElement el : root.getAsJsonArray("blocks")) {
                JsonObject blockObj = el.getAsJsonObject();
                int oldId = blockObj.get("id").getAsInt();
                String type = blockObj.get("type").getAsString();

                BlockDef def = VisualBlockRegistry.findBlockByName(type);
                if (def == null) {
                    EvansComputerMod.LOGGER.warn("Unknown block type '{}' in visual program, skipping", type);
                    continue;
                }

                int newId = nextId++;
                idRemap.put(oldId, newId);

                int color = VisualBlockRegistry.getColorForBlock(type);
                float x = blockObj.get("x").getAsFloat();
                float y = blockObj.get("y").getAsFloat();

                Map<String, String> inputValues = new HashMap<>();
                // Initialize defaults
                for (PortDef p : def.dataInputs()) {
                    if (p.defaultValue() != null) {
                        inputValues.put(p.name(), p.defaultValue());
                    }
                }
                // Override with saved values
                if (blockObj.has("inputs")) {
                    JsonObject inputs = blockObj.getAsJsonObject("inputs");
                    for (Map.Entry<String, JsonElement> entry : inputs.entrySet()) {
                        inputValues.put(entry.getKey(), entry.getValue().getAsString());
                    }
                }

                blocks.add(new SerializedBlock(newId, def, color, x, y, inputValues));
            }
        }

        List<SerializedConnection> connections = new ArrayList<>();
        if (root.has("connections")) {
            for (JsonElement el : root.getAsJsonArray("connections")) {
                JsonObject connObj = el.getAsJsonObject();
                int oldFrom = connObj.get("fromBlock").getAsInt();
                int oldTo = connObj.get("toBlock").getAsInt();

                Integer newFrom = idRemap.get(oldFrom);
                Integer newTo = idRemap.get(oldTo);
                if (newFrom == null || newTo == null) {
                    continue; // Skip connections to unknown blocks
                }

                connections.add(new SerializedConnection(
                        newFrom,
                        connObj.get("fromPort").getAsString(),
                        newTo,
                        connObj.get("toPort").getAsString()
                ));
            }
        }

        return new DeserializedProgram(blocks, connections, offsetX, offsetY, zoom, name, nextId);
    }
}
