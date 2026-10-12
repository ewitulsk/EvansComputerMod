package com.example.evanscomputermod.client;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.computer.peripheral.PeripheralCatalog;
import com.google.gson.Gson;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * One-off export of the old Java Visual Programmer's code generator output
 * (ROADMAP P6): every {@code rust/wasm-programs/visual/tests/vpl/*.vpl} is
 * loaded the way the old screen loaded it (ids renumbered from 1) and run
 * through {@link VisualCodeGenerator}; the Python is written next to it as
 * {@code <name>.py}. The Rust port must reproduce these bytes.
 * Also writes the peripheral catalog of the test peripheral used by
 * {@code peripheral.vpl}.
 */
public class VisualGoldenExportTest {

    public static class TestLamp extends AnnotatedPeripheral {
        @Override
        public String getType() {
            return "test_lamp";
        }

        @PeripheralMethod(description = "Set the lamp color")
        public void setColor(int color) {
        }

        @PeripheralMethod
        public int getLevel() {
            return 0;
        }

        @PeripheralMethod(description = "Show a label")
        public void setLabel(String text, boolean bright) {
        }
    }

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) {
            p = p.getParent();
        }
        return p;
    }

    @Test
    void exportGoldens() throws Exception {
        PeripheralTypes.register("test_lamp", "Test lamp", TestLamp.class);
        Path dir = repoRoot().resolve("rust/wasm-programs/visual/tests/vpl");
        List<Path> files;
        try (Stream<Path> s = Files.list(dir)) {
            files = s.filter(p -> p.toString().endsWith(".vpl")).sorted().toList();
        }
        assertTrue(files.size() >= 5, "samples found in " + dir);
        for (Path f : files) {
            String json = Files.readString(f, StandardCharsets.UTF_8);
            VisualProgramSerializer.DeserializedProgram prog = VisualProgramSerializer.deserialize(json, 1);
            List<VisualCodeGenerator.BlockInstance> blocks = new ArrayList<>();
            for (VisualProgramSerializer.SerializedBlock b : prog.blocks()) {
                blocks.add(new VisualCodeGenerator.BlockInstance(b.id(), b.definition(), b.inputValues()));
            }
            List<VisualCodeGenerator.Connection> conns = new ArrayList<>();
            for (VisualProgramSerializer.SerializedConnection c : prog.connections()) {
                conns.add(new VisualCodeGenerator.Connection(c.fromBlockId(), c.fromPort(), c.toBlockId(), c.toPort()));
            }
            String code = VisualCodeGenerator.generate(blocks, conns);
            String name = f.getFileName().toString().replace(".vpl", ".py");
            Files.writeString(dir.resolve(name), code, StandardCharsets.UTF_8);
        }

        // The catalog the guest gets from periph_methods("@catalog"), as JSON.
        List<Object> catalog = new ArrayList<>();
        for (Object t : PeripheralCatalog.describe()) {
            if ("test_lamp".equals(((List<?>) t).get(0))) catalog.add(t);
        }
        Files.writeString(dir.resolve("peripheral_catalog.json"), new Gson().toJson(catalog) + "\n", StandardCharsets.UTF_8);
    }
}
