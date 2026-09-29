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


    public static String generate(List<BlockInstance> blocks, List<Connection> connections) {
        // Find the start block
        BlockInstance startBlock = null;
        for (BlockInstance block : blocks) {
            if ("start".equals(block.definition().name())) {
                startBlock = block;
                break;
            }
        }

        // Collect top-level event blocks (these can exist without a Start block)
        List<BlockInstance> eventBlocks = new ArrayList<>();
        for (BlockInstance block : blocks) {
            String name = block.definition().name();
            if ("on_key_press".equals(name) || "on_redstone".equals(name)) {
                eventBlocks.add(block);
            }
        }

        if (startBlock == null && eventBlocks.isEmpty()) {
            return "ERROR: No Start block or event blocks found. Add a Start block or event handler to begin.";
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

        // Detect imports by checking ALL blocks (data-only blocks aren't flow-reachable)
        boolean needsTerminal = false;
        boolean needsPeripheral = false;
        for (BlockInstance block : blocks) {
            String template = block.definition().codeTemplate();
            if (template != null) {
                if (template.contains("terminal.")) needsTerminal = true;
                if (template.contains("peripheral.")) needsPeripheral = true;
            }
            // Event blocks use terminal.on_interrupt but have empty templates
            String name = block.definition().name();
            if ("on_key_press".equals(name) || "on_redstone".equals(name)) needsTerminal = true;
        }

        // Generate code
        StringBuilder code = new StringBuilder();
        if (needsTerminal) code.append("import terminal\n");
        if (needsPeripheral) code.append("import peripheral\n");
        if (needsTerminal || needsPeripheral) code.append("\n");

        Set<Integer> emittedDataBlocks = new HashSet<>();
        Set<Integer> visitedBlocks = new HashSet<>();

        // Generate event handler blocks first (they define functions and register interrupts)
        for (BlockInstance eventBlock : eventBlocks) {
            generateChain(eventBlock.id(), blockById, flowNext, dataConnections, code, 0, visitedBlocks, emittedDataBlocks);
        }

        // Generate the Start block's flow chain (if Start exists)
        if (startBlock != null) {
            int firstId = getFlowTarget(flowNext, startBlock.id(), "flow");
            if (firstId != -1) {
                generateChain(firstId, blockById, flowNext, dataConnections, code, 0, visitedBlocks, emittedDataBlocks);
            }
        }

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
            Set<Integer> visited,
            Set<Integer> emittedDataBlocks
    ) {
        if (blockId == -1 || visited.contains(blockId)) return;
        visited.add(blockId);

        BlockInstance block = blockById.get(blockId);
        if (block == null) return;

        String indent = "    ".repeat(indentLevel);
        String generatorName = block.definition().generatorOrName();

        // Emit any data dependency blocks before this block's code
        emitDataDependencies(block, blockById, dataConnections, code, indentLevel, emittedDataBlocks);

        switch (generatorName) {
            case "if" -> {
                String condition = resolveInputRaw(block, "condition", dataConnections, blockById);
                code.append(indent).append("if ").append(condition).append(":\n");

                int trueTarget = getFlowTarget(flowNext, blockId, "true");
                generateChain(trueTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);

                int falseTarget = getFlowTarget(flowNext, blockId, "false");
                handleFalseBranch(falseTarget, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
            }
            case "else_if" -> {
                String condition = resolveInputRaw(block, "condition", dataConnections, blockById);
                code.append(indent).append("elif ").append(condition).append(":\n");

                int trueTarget = getFlowTarget(flowNext, blockId, "true");
                generateChain(trueTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);

                int falseTarget = getFlowTarget(flowNext, blockId, "false");
                handleFalseBranch(falseTarget, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
            }
            case "else" -> {
                code.append(indent).append("else:\n");
                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);
            }
            case "loop" -> {
                String times = resolveInputRaw(block, "times", dataConnections, blockById);
                code.append(indent).append("for _loop").append(block.id()).append(" in range(int(").append(times).append(")):\n");

                int loopBodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(loopBodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);

                int loopDoneTarget = getFlowTarget(flowNext, blockId, "done");
                generateChain(loopDoneTarget, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
            }
            case "loop_forever" -> {
                code.append(indent).append("while True:\n");

                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);
            }
            case "on_key_press" -> {
                String funcName = "_on_key_" + block.id();
                String keyVar = "_block" + block.id() + "_key";
                code.append(indent).append("def ").append(funcName).append("(data):\n");
                code.append(indent).append("    ").append(keyVar).append(" = data['key']\n");

                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);

                code.append(indent).append("terminal.on_interrupt(terminal.IRQ_KEYBOARD, ").append(funcName).append(")\n");

                // Follow flow output for blocks after this event registration
                int nextId = getFlowTarget(flowNext, blockId, "flow");
                generateChain(nextId, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
            }
            case "on_redstone" -> {
                String funcName = "_on_redstone_" + block.id();
                String sidesVar = "_block" + block.id() + "_sides";
                code.append(indent).append("def ").append(funcName).append("(data):\n");
                code.append(indent).append("    ").append(sidesVar).append(" = data['sides']\n");

                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);

                code.append(indent).append("terminal.on_interrupt(terminal.IRQ_REDSTONE, ").append(funcName).append(")\n");

                int nextId = getFlowTarget(flowNext, blockId, "flow");
                generateChain(nextId, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
            }
            case "for_loop" -> {
                String variable = resolveInputRaw(block, "variable", dataConnections, blockById);
                String range = resolveInputRaw(block, "range", dataConnections, blockById);
                code.append(indent).append("for ").append(variable).append(" in range(").append(range).append("):\n");

                int bodyTarget = getFlowTarget(flowNext, blockId, "body");
                generateChain(bodyTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);

                // Continue after the loop
                int doneTarget = getFlowTarget(flowNext, blockId, "done");
                generateChain(doneTarget, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
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
                generateChain(nextId, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
            }
        }
    }

    /**
     * Recursively emits code for data-only blocks that are connected as inputs
     * to the given block. Ensures dependency ordering (deepest first).
     */
    private static void emitDataDependencies(
            BlockInstance block,
            Map<Integer, BlockInstance> blockById,
            Map<String, String[]> dataConnections,
            StringBuilder code,
            int indentLevel,
            Set<Integer> emittedDataBlocks
    ) {
        for (PortDef input : block.definition().dataInputs()) {
            String connKey = block.id() + ":" + input.name();
            if (!dataConnections.containsKey(connKey)) continue;

            String[] source = dataConnections.get(connKey);
            int fromBlockId = Integer.parseInt(source[0]);
            if (emittedDataBlocks.contains(fromBlockId)) continue;

            BlockInstance sourceBlock = blockById.get(fromBlockId);
            if (sourceBlock == null) continue;

            // Skip flow blocks — they are emitted by the flow chain, not as data dependencies.
            // This prevents duplicate code for blocks like Input that have both flow and data outputs.
            if (!sourceBlock.definition().flowInputs().isEmpty() || !sourceBlock.definition().flowOutputs().isEmpty()) {
                continue;
            }

            String srcTemplate = sourceBlock.definition().codeTemplate();
            if (srcTemplate == null || srcTemplate.isEmpty()) {
                // Passthrough block (constants) — may have its own upstream dependencies
                emitDataDependencies(sourceBlock, blockById, dataConnections, code, indentLevel, emittedDataBlocks);
                continue;
            }

            // Recursively emit this block's dependencies first
            emitDataDependencies(sourceBlock, blockById, dataConnections, code, indentLevel, emittedDataBlocks);

            // Emit this data block's code
            String template = substituteTemplate(sourceBlock, srcTemplate, dataConnections, blockById);
            code.append("    ".repeat(indentLevel)).append(template).append("\n");
            emittedDataBlocks.add(fromBlockId);
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
            Set<Integer> visited,
            Set<Integer> emittedDataBlocks
    ) {
        if (falseTarget == -1) return;

        BlockInstance falseBlock = blockById.get(falseTarget);
        if (falseBlock == null) return;

        String falseGen = falseBlock.definition().generatorOrName();
        if ("else_if".equals(falseGen) || "else".equals(falseGen)) {
            generateChain(falseTarget, blockById, flowNext, dataConnections, code, indentLevel, visited, emittedDataBlocks);
        } else {
            String indent = "    ".repeat(indentLevel);
            code.append(indent).append("else:\n");
            generateChain(falseTarget, blockById, flowNext, dataConnections, code, indentLevel + 1, new HashSet<>(visited), emittedDataBlocks);
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
     * For passthrough blocks (empty code template, like constants), follows through to their input.
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

            // Check if source block is a passthrough (empty code template)
            BlockInstance sourceBlock = blockById.get(fromBlockId);
            if (sourceBlock != null) {
                String srcTemplate = sourceBlock.definition().codeTemplate();
                if (srcTemplate == null || srcTemplate.isEmpty()) {
                    // Follow through to the source block's input with matching name
                    for (PortDef srcInput : sourceBlock.definition().dataInputs()) {
                        if (srcInput.name().equals(fromPort)) {
                            return resolveInput(sourceBlock, srcInput, dataConnections, blockById);
                        }
                    }
                }
            }
            return "_block" + fromBlockId + "_" + fromPort;
        }

        String literal = block.inputValues().getOrDefault(input.name(), input.defaultValue());
        if (literal == null) literal = "";
        if ("string".equals(input.type())) {
            return "\"" + literal.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
        }
        if ("any".equals(input.type())) {
            String typeMode = block.inputValues().getOrDefault(input.name() + "_type", "string");
            if ("string".equals(typeMode)) {
                return "\"" + literal.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
            }
            return literal; // int or float — pass raw
        }
        return literal;
    }

    /**
     * Resolves an input as a raw Python expression (no string quoting).
     * Used for control flow inputs like condition, variable, range.
     * For passthrough blocks, follows through to their input.
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

            // Check if source block is a passthrough (empty code template)
            BlockInstance sourceBlock = blockById.get(fromBlockId);
            if (sourceBlock != null) {
                String srcTemplate = sourceBlock.definition().codeTemplate();
                if (srcTemplate == null || srcTemplate.isEmpty()) {
                    for (PortDef srcInput : sourceBlock.definition().dataInputs()) {
                        if (srcInput.name().equals(fromPort)) {
                            // Resolve raw — use value without quoting
                            String literal = sourceBlock.inputValues().getOrDefault(srcInput.name(), srcInput.defaultValue());
                            return literal != null ? literal : "";
                        }
                    }
                }
            }
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

}
