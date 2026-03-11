package com.example.evanscomputermod.client;

import com.example.evanscomputermod.client.VisualBlockRegistry.BlockDef;
import com.example.evanscomputermod.client.VisualBlockRegistry.PortDef;

import java.util.*;

/**
 * Generates Python code from a visual programming graph.
 * Supports linear flow, if/else if/else branching, and for loops
 * with proper indentation via recursive traversal.
 */
public class VisualCodeGenerator {

    public record BlockInstance(int id, BlockDef definition, Map<String, String> inputValues) {}

    public record Connection(int fromBlockId, String fromPort, int toBlockId, String toPort) {}

    // Control flow block names
    private static final Set<String> CONTROL_FLOW_BLOCKS = Set.of("if", "else_if", "else", "for_loop");
    // Inputs on control flow blocks that should NOT be string-quoted
    private static final Set<String> RAW_INPUTS = Set.of("condition", "variable", "range");

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

        // Build flow map: keyed on "blockId:portName" -> targetBlockId
        Map<String, Integer> flowNext = new HashMap<>();
        // Build data connections: "toBlockId:toPort" -> [fromBlockId, fromPort]
        Map<String, String[]> dataConnections = new HashMap<>();

        for (Connection conn : connections) {
            BlockInstance fromBlock = blockById.get(conn.fromBlockId());
            if (fromBlock == null) continue;

            boolean isFlowConn = false;
            for (PortDef port : fromBlock.definition().outputs()) {
                if (port.name().equals(conn.fromPort()) && port.isFlow()) {
                    isFlowConn = true;
                    break;
                }
            }

            if (isFlowConn) {
                String key = conn.fromBlockId() + ":" + conn.fromPort();
                flowNext.put(key, conn.toBlockId());
            } else {
                String key = conn.toBlockId() + ":" + conn.toPort();
                dataConnections.put(key, new String[]{String.valueOf(conn.fromBlockId()), conn.fromPort()});
            }
        }

        // First pass: detect imports by walking the entire reachable graph
        boolean needsTerminal = false;
        boolean needsPeripheral = false;
        Set<Integer> reachable = new HashSet<>();
        collectReachable(startBlock.id(), blockById, flowNext, reachable);

        for (int id : reachable) {
            BlockInstance block = blockById.get(id);
            if (block == null) continue;
            String template = block.definition().codeTemplate();
            if (template != null) {
                if (template.contains("terminal.")) needsTerminal = true;
                if (template.contains("peripheral.")) needsPeripheral = true;
            }
        }

        // Generate code
        StringBuilder code = new StringBuilder();
        if (needsTerminal) code.append("import terminal\n");
        if (needsPeripheral) code.append("import peripheral\n");
        if (needsTerminal || needsPeripheral) code.append("\n");

        // Follow the start block's flow output
        int firstId = getFlowTarget(flowNext, startBlock.id(), "flow");
        if (firstId == -1) {
            return "ERROR: Start block has no connections. Connect blocks with flow wires.";
        }

        generateChain(firstId, blockById, flowNext, dataConnections, code, 0, new HashSet<>());

        return code.toString();
    }

    /**
     * Recursively generates code following the flow chain from blockId.
     */
    private static void generateChain(
            int blockId,
            Map<Integer, BlockInstance> blockById,
            Map<String, Integer> flowNext,
            Map<String, String[]> dataConnections,
            StringBuilder code,
            int indentLevel,
            Set<Integer> visited
    ) {
        if (blockId == -1 || visited.contains(blockId)) return;
        visited.add(blockId);

        BlockInstance block = blockById.get(blockId);
        if (block == null) return;

        String indent = "    ".repeat(indentLevel);
        String blockName = block.definition().name();

        switch (blockName) {
            case "if" -> {
                String condition = resolveInputRaw(block, "condition", dataConnections, blockById);
                code.append(indent).append("if ").append(condition).append(":\n");

                int trueTarget = getFlowTarget(flowNext, blockId, "true");
                generateChain(trueTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited));

                int falseTarget = getFlowTarget(flowNext, blockId, "false");
                handleFalseBranch(falseTarget, blockById, flowNext, dataConnections, code, indentLevel, visited);
            }
            case "else_if" -> {
                String condition = resolveInputRaw(block, "condition", dataConnections, blockById);
                code.append(indent).append("elif ").append(condition).append(":\n");

                int trueTarget = getFlowTarget(flowNext, blockId, "true");
                generateChain(trueTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited));

                int falseTarget = getFlowTarget(flowNext, blockId, "false");
                handleFalseBranch(falseTarget, blockById, flowNext, dataConnections, code, indentLevel, visited);
            }
            case "else" -> {
                code.append(indent).append("else:\n");
                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited));
            }
            case "for_loop" -> {
                String variable = resolveInputRaw(block, "variable", dataConnections, blockById);
                String range = resolveInputRaw(block, "range", dataConnections, blockById);
                code.append(indent).append("for ").append(variable).append(" in range(").append(range).append("):\n");

                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited));

                // Continue after the loop
                int doneTarget = getFlowTarget(flowNext, blockId, "done");
                generateChain(doneTarget, blockById, flowNext, dataConnections, code, indentLevel, visited);
            }
            default -> {
                // Normal block: substitute template
                String template = block.definition().codeTemplate();
                if (template != null && !template.isEmpty()) {
                    template = substituteTemplate(block, template, dataConnections, blockById);
                    code.append(indent).append(template).append("\n");
                }

                // Follow the single "flow" output
                int nextId = getFlowTarget(flowNext, blockId, "flow");
                generateChain(nextId, blockById, flowNext, dataConnections, code, indentLevel, visited);
            }
        }
    }

    /**
     * Handles the false branch of an if/else_if block.
     * If the target is else_if or else, chains naturally. Otherwise wraps in else:.
     */
    private static void handleFalseBranch(
            int falseTarget,
            Map<Integer, BlockInstance> blockById,
            Map<String, Integer> flowNext,
            Map<String, String[]> dataConnections,
            StringBuilder code,
            int indentLevel,
            Set<Integer> visited
    ) {
        if (falseTarget == -1) return;

        BlockInstance falseBlock = blockById.get(falseTarget);
        if (falseBlock == null) return;

        String falseName = falseBlock.definition().name();
        if ("else_if".equals(falseName) || "else".equals(falseName)) {
            // Chain directly — these emit their own elif/else keywords
            generateChain(falseTarget, blockById, flowNext, dataConnections, code, indentLevel, visited);
        } else {
            // Wrap in an implicit else block
            String indent = "    ".repeat(indentLevel);
            code.append(indent).append("else:\n");
            generateChain(falseTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited));
        }
    }

    /**
     * Substitutes template placeholders for a normal (non-control-flow) block.
     */
    private static String substituteTemplate(
            BlockInstance block,
            String template,
            Map<String, String[]> dataConnections,
            Map<Integer, BlockInstance> blockById
    ) {
        // Substitute data inputs
        for (PortDef input : block.definition().dataInputs()) {
            String placeholder = "{" + input.name() + "}";
            String value = resolveInput(block, input, dataConnections, blockById);
            template = template.replace(placeholder, value);
        }

        // Substitute data outputs (variable assignment)
        for (PortDef output : block.definition().dataOutputs()) {
            String placeholder = "{" + output.name() + "}";
            String varName = "_block" + block.id() + "_" + output.name();
            template = template.replace(placeholder, varName);
        }

        return template;
    }

    /**
     * Resolves a data input value: uses connected variable name or literal (with string quoting).
     */
    private static String resolveInput(
            BlockInstance block,
            PortDef input,
            Map<String, String[]> dataConnections,
            Map<Integer, BlockInstance> blockById
    ) {
        String connKey = block.id() + ":" + input.name();
        if (dataConnections.containsKey(connKey)) {
            String[] source = dataConnections.get(connKey);
            int fromBlockId = Integer.parseInt(source[0]);
            String fromPort = source[1];
            return "_block" + fromBlockId + "_" + fromPort;
        }

        String literal = block.inputValues().getOrDefault(input.name(), input.defaultValue());
        if (literal == null) literal = "";
        if ("string".equals(input.type())) {
            return "\"" + literal.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
        }
        return literal;
    }

    /**
     * Resolves an input as a raw Python expression (no string quoting).
     * Used for control flow inputs like condition, variable, range.
     */
    private static String resolveInputRaw(
            BlockInstance block,
            String inputName,
            Map<String, String[]> dataConnections,
            Map<Integer, BlockInstance> blockById
    ) {
        String connKey = block.id() + ":" + inputName;
        if (dataConnections.containsKey(connKey)) {
            String[] source = dataConnections.get(connKey);
            int fromBlockId = Integer.parseInt(source[0]);
            String fromPort = source[1];
            return "_block" + fromBlockId + "_" + fromPort;
        }

        // Find the port definition to get the default
        for (PortDef input : block.definition().dataInputs()) {
            if (input.name().equals(inputName)) {
                String literal = block.inputValues().getOrDefault(inputName, input.defaultValue());
                return literal != null ? literal : "";
            }
        }
        return block.inputValues().getOrDefault(inputName, "");
    }

    /**
     * Gets the flow target for a specific named flow output port.
     */
    private static int getFlowTarget(Map<String, Integer> flowNext, int blockId, String portName) {
        return flowNext.getOrDefault(blockId + ":" + portName, -1);
    }

    /**
     * Collects all block IDs reachable from the given block via flow connections.
     */
    private static void collectReachable(
            int blockId,
            Map<Integer, BlockInstance> blockById,
            Map<String, Integer> flowNext,
            Set<Integer> reachable
    ) {
        if (blockId == -1 || reachable.contains(blockId)) return;
        reachable.add(blockId);

        BlockInstance block = blockById.get(blockId);
        if (block == null) return;

        // Follow all flow outputs
        for (PortDef port : block.definition().flowOutputs()) {
            int target = getFlowTarget(flowNext, blockId, port.name());
            collectReachable(target, blockById, flowNext, reachable);
        }
        // Also check the generic "flow" output for start block
        int flowTarget = getFlowTarget(flowNext, blockId, "flow");
        collectReachable(flowTarget, blockById, flowNext, reachable);
    }
}
