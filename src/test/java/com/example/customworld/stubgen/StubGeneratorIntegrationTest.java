package com.example.customworld.stubgen;

import com.example.customworld.stubgen.mock.MockChatBox;
import com.example.customworld.stubgen.mock.MockEnvironmentDetector;
import com.example.customworld.stubgen.mock.MockLuaFunction;
import com.example.customworld.stubgen.mock.MockPlayerDetector;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Integration tests for the stub generation pipeline.
 * Tests the full flow from scanning mock peripherals to generating Python stubs.
 * 
 * NOTE: Rust stub generation tests have been removed since RustStubGenerator is disabled.
 */
class StubGeneratorIntegrationTest {
    
    @TempDir
    Path tempDir;
    
    private Path pythonDir;
    private Path definitionsFile;
    
    @BeforeEach
    void setUp() {
        pythonDir = tempDir.resolve("python");
        definitionsFile = tempDir.resolve("peripheral_definitions.json");
    }
    
    @Test
    void testFullPipeline() throws IOException {
        // Step 1: Scan mock peripherals
        PeripheralScanner scanner = new PeripheralScanner(MockLuaFunction.class);
        
        Map<String, Class<?>> classes = new HashMap<>();
        classes.put("playerDetector", MockPlayerDetector.class);
        classes.put("chatBox", MockChatBox.class);
        classes.put("environmentDetector", MockEnvironmentDetector.class);
        
        PeripheralDefinitions definitions = scanner.scanClasses(classes);
        
        // Should find all peripherals
        assertEquals(3, definitions.getPeripherals().size());
        
        // Step 2: Save to JSON
        definitions.saveToFile(definitionsFile);
        assertTrue(Files.exists(definitionsFile));
        
        // Step 3: Load from JSON
        PeripheralDefinitions loaded = PeripheralDefinitions.loadFromFile(definitionsFile);
        assertEquals(3, loaded.getPeripherals().size());
        
        // Verify loaded data matches original
        for (PeripheralDefinition original : definitions.getPeripherals()) {
            PeripheralDefinition loadedDef = loaded.getPeripherals().stream()
                    .filter(p -> p.getType().equals(original.getType()))
                    .findFirst()
                    .orElse(null);
            
            assertNotNull(loadedDef, "Should find " + original.getType());
            assertEquals(original.getMethods().size(), loadedDef.getMethods().size(),
                    "Method count should match for " + original.getType());
        }
        
        // Step 4: Generate Python stubs
        PythonStubGenerator pythonGen = new PythonStubGenerator();
        pythonGen.generate(loaded, pythonDir);
        
        // Verify Python output
        assertTrue(Files.exists(pythonDir.resolve("__init__.py")));
        assertTrue(Files.exists(pythonDir.resolve("player_detector.py")));
        assertTrue(Files.exists(pythonDir.resolve("chat_box.py")));
        assertTrue(Files.exists(pythonDir.resolve("environment_detector.py")));
    }
    
    @Test
    void testJsonRoundTrip() throws IOException {
        // Create definitions manually
        PeripheralDefinitions original = new PeripheralDefinitions();
        
        PeripheralDefinition player = new PeripheralDefinition("playerDetector", "mock.PlayerDetector");
        MethodDefinition getPlayers = new MethodDefinition("getOnlinePlayers", "getOnlinePlayers");
        getPlayers.setReturnType("String[]");
        player.addMethod(getPlayers);
        
        MethodDefinition getRange = new MethodDefinition("getPlayersInRange", "getPlayersInRange");
        getRange.setReturnType("String[]");
        getRange.addParameter(new ParameterDefinition("range", "int", false));
        player.addMethod(getRange);
        
        original.addPeripheral(player);
        
        // Save to JSON
        original.saveToFile(definitionsFile);
        
        // Read JSON content and verify structure
        String json = Files.readString(definitionsFile);
        assertTrue(json.contains("\"type\": \"playerDetector\""));
        assertTrue(json.contains("\"luaName\": \"getOnlinePlayers\""));
        assertTrue(json.contains("\"returnType\": \"String[]\""));
        assertTrue(json.contains("\"name\": \"range\""));
        
        // Load and verify
        PeripheralDefinitions loaded = PeripheralDefinitions.loadFromFile(definitionsFile);
        assertEquals(1, loaded.getPeripherals().size());
        
        PeripheralDefinition loadedPlayer = loaded.getPeripherals().get(0);
        assertEquals("playerDetector", loadedPlayer.getType());
        assertEquals(2, loadedPlayer.getMethods().size());
        
        MethodDefinition loadedGetPlayers = loadedPlayer.getMethods().get(0);
        assertEquals("getOnlinePlayers", loadedGetPlayers.getLuaName());
        assertEquals("String[]", loadedGetPlayers.getReturnType());
    }
    
    @Test
    void testGeneratedPythonIsConsistent() throws IOException {
        // Generate Python twice and verify identical output
        PeripheralScanner scanner = new PeripheralScanner(MockLuaFunction.class);
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        PeripheralDefinitions definitions = new PeripheralDefinitions();
        definitions.addPeripheral(def);
        
        PythonStubGenerator generator = new PythonStubGenerator();
        
        Path output1 = tempDir.resolve("python1");
        Path output2 = tempDir.resolve("python2");
        
        generator.generate(definitions, output1);
        generator.generate(definitions, output2);
        
        // Content should be identical (except possibly timestamp in comments)
        String content1 = Files.readString(output1.resolve("player_detector.py"));
        String content2 = Files.readString(output2.resolve("player_detector.py"));
        
        assertEquals(content1, content2, "Generated Python should be deterministic");
    }
    
    @Test
    void testEmptyPeripherals() throws IOException {
        // Test with no peripherals
        PeripheralDefinitions empty = new PeripheralDefinitions();
        
        PythonStubGenerator pythonGen = new PythonStubGenerator();
        pythonGen.generate(empty, pythonDir);
        
        // Should still create __init__.py
        assertTrue(Files.exists(pythonDir.resolve("__init__.py")));
        String initContent = Files.readString(pythonDir.resolve("__init__.py"));
        assertTrue(initContent.contains("__all__"));
    }
    
    @Test
    void testMethodCountPreserved() throws IOException {
        // Verify all methods are preserved through the pipeline
        PeripheralScanner scanner = new PeripheralScanner(MockLuaFunction.class);
        
        Map<String, Class<?>> classes = new HashMap<>();
        classes.put("playerDetector", MockPlayerDetector.class);
        classes.put("chatBox", MockChatBox.class);
        classes.put("environmentDetector", MockEnvironmentDetector.class);
        
        PeripheralDefinitions definitions = scanner.scanClasses(classes);
        
        // Count methods per peripheral
        int playerMethods = definitions.getPeripherals().stream()
                .filter(p -> p.getType().equals("playerDetector"))
                .mapToInt(p -> p.getMethods().size())
                .sum();
        
        // Save and reload
        definitions.saveToFile(definitionsFile);
        PeripheralDefinitions loaded = PeripheralDefinitions.loadFromFile(definitionsFile);
        
        // Verify counts match
        int loadedPlayerMethods = loaded.getPeripherals().stream()
                .filter(p -> p.getType().equals("playerDetector"))
                .mapToInt(p -> p.getMethods().size())
                .sum();
        
        assertEquals(playerMethods, loadedPlayerMethods, "Player detector method count should match");
        
        // Generate and verify method presence in output
        PythonStubGenerator pythonGen = new PythonStubGenerator();
        pythonGen.generate(loaded, pythonDir);
        
        String playerPython = Files.readString(pythonDir.resolve("player_detector.py"));
        // Count "def " occurrences - subtract helpers (_parse_json_result, _parse_string_array)
        // and class methods (__init__, find)
        int defCount = countOccurrences(playerPython, "def ");
        // We have 2 helper functions, 1 __init__, 1 find(), plus the actual methods
        assertTrue(defCount >= playerMethods, 
                "Python should have at least " + playerMethods + " method defs, got " + defCount);
    }
    
    private int countOccurrences(String str, String sub) {
        int count = 0;
        int idx = 0;
        while ((idx = str.indexOf(sub, idx)) != -1) {
            count++;
            idx += sub.length();
        }
        return count;
    }
}
