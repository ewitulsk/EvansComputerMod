package com.example.evanscomputermod.client;

import com.example.evanscomputermod.client.VisualBlockRegistry.BlockDef;
import com.example.evanscomputermod.client.VisualBlockRegistry.PortDef;

import java.util.*;

/**
 * Generates Python code from a visual programming graph.
 * Traverses the flow chain starting from a "Start" block and produces
 * Python code that calls the terminal/peripheral APIs.
 */
public class VisualCodeGenerator {

    /**
     * A placed block on the canvas with its id, definition, and input values.
     */
    public record BlockInstance(int id, BlockDef definition, Map<String, String> inputValues) {}

    /**
     * A connection between two ports.
     */
    public record Connection(int fromBlockId, String fromPort, int toBlockId, String toPort) {}

    /**
     * Generates Python code from the visual graph.
     *
     * @param blocks      All placed blocks
     * @param connections All connections between ports
     * @return The generated Python code, or an error message starting with "ERROR:"
     */
    public static String generate(List<BlockInstance> blocks, List<Connection> connections) {
        // Find the start block
        BlockInstance startBlock = null;
        for (BlockInstance block : blocks) {
            if ("start".equals(block.definition().name())) {
                startBlock = block;
                break;
            }
        }

        if (startBlock == null) {
            return "ERROR: No Start block found. Add a Start block to begin.";
        }

        // Build lookup maps
        Map<Integer, BlockInstance> blockById = new HashMap<>();
        for (BlockInstance block : blocks) {
            blockById.put(block.id(), block);
        }

        // Build flow chain: for each block, find which block its flow output connects to
        Map<Integer, Integer> flowNext = new HashMap<>();
        // Build data connections: for each (toBlockId, toPort), find (fromBlockId, fromPort)
        Map<String, String[]> dataConnections = new HashMap<>(); // key: "toBlockId:toPort" -> [fromBlockId, fromPort]

        for (Connection conn : connections) {
            BlockInstance fromBlock = blockById.get(conn.fromBlockId());
            if (fromBlock == null) continue;

            // Check if this is a flow connection
            boolean isFlowConn = false;
            for (PortDef port : fromBlock.definition().outputs()) {
                if (port.name().equals(conn.fromPort()) && port.isFlow()) {
                    isFlowConn = true;
                    break;
                }
            }

            if (isFlowConn) {
                flowNext.put(conn.fromBlockId(), conn.toBlockId());
            } else {
                String key = conn.toBlockId() + ":" + conn.toPort();
                dataConnections.put(key, new String[]{String.valueOf(conn.fromBlockId()), conn.fromPort()});
            }
        }

        // Traverse flow chain
        StringBuilder code = new StringBuilder();
        boolean needsTerminal = false;
        boolean needsPeripheral = false;

        // First pass: check which imports are needed
        List<BlockInstance> flowChain = new ArrayList<>();
        int currentId = startBlock.id();
        Set<Integer> visited = new HashSet<>();

        while (flowNext.containsKey(currentId)) {
            int nextId = flowNext.get(currentId);
            if (visited.contains(nextId)) break; // Prevent infinite loops
            visited.add(nextId);

            BlockInstance block = blockById.get(nextId);
            if (block == null) break;

            flowChain.add(block);
            String template = block.definition().codeTemplate();
            if (template.contains("terminal.")) needsTerminal = true;
            if (template.contains("peripheral.")) needsPeripheral = true;
            currentId = nextId;
        }

        // Imports
        if (needsTerminal) code.append("import terminal\n");
        if (needsPeripheral) code.append("import peripheral\n");
        if (needsTerminal || needsPeripheral) code.append("\n");

        // Generate code for each block in the flow chain
        for (BlockInstance block : flowChain) {
            String template = block.definition().codeTemplate();
            if (template == null || template.isEmpty()) continue;

            // Substitute each input placeholder
            for (PortDef input : block.definition().dataInputs()) {
                String placeholder = "{" + input.name() + "}";
                String value;

                // Check if this input has a data connection
                String connKey = block.id() + ":" + input.name();
                if (dataConnections.containsKey(connKey)) {
                    String[] source = dataConnections.get(connKey);
                    int fromBlockId = Integer.parseInt(source[0]);
                    String fromPort = source[1];
                    value = "_block" + fromBlockId + "_" + fromPort;
                } else {
                    // Use the literal value from the block's input values
                    String literal = block.inputValues().getOrDefault(input.name(), input.defaultValue());
                    if (literal == null) literal = "";
                    if ("string".equals(input.type())) {
                        value = "\"" + literal.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
                    } else {
                        value = literal;
                    }
                }

                template = template.replace(placeholder, value);
            }

            // Handle output variable assignment
            List<PortDef> dataOutputs = block.definition().dataOutputs();
            if (!dataOutputs.isEmpty()) {
                // The template contains {outputName} = ... pattern
                for (PortDef output : dataOutputs) {
                    String outputPlaceholder = "{" + output.name() + "}";
                    String varName = "_block" + block.id() + "_" + output.name();
                    template = template.replace(outputPlaceholder, varName);
                }
            }

            code.append(template).append("\n");
        }

        if (flowChain.isEmpty()) {
            return "ERROR: Start block has no connections. Connect blocks with flow wires.";
        }

        return code.toString();
    }
}
