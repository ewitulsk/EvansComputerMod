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
 * Tests for PythonStubGenerator.
 * Verifies that generated Python code is syntactically correct and contains
 * expected content.
 */
class PythonStubGeneratorTest {
    
    private PeripheralScanner scanner;
    private PythonStubGenerator generator;
    private PeripheralDefinitions definitions;
    
    @TempDir
    Path tempDir;
    
    @BeforeEach
    void setUp() {
        scanner = new PeripheralScanner(MockLuaFunction.class);
        generator = new PythonStubGenerator();
        
        // Scan all mock peripherals
        Map<String, Class<?>> classes = new HashMap<>();
        classes.put("playerDetector", MockPlayerDetector.class);
        classes.put("chatBox", MockChatBox.class);
        classes.put("environmentDetector", MockEnvironmentDetector.class);
        
        definitions = scanner.scanClasses(classes);
    }
    
    @Test
    void testGenerateCreatesFiles() throws IOException {
        generator.generate(definitions, tempDir);
        
        // Should create __init__.py
        assertTrue(Files.exists(tempDir.resolve("__init__.py")), "Should create __init__.py");
        
        // Should create a file for each peripheral
        assertTrue(Files.exists(tempDir.resolve("player_detector.py")), "Should create player_detector.py");
        assertTrue(Files.exists(tempDir.resolve("chat_box.py")), "Should create chat_box.py");
        assertTrue(Files.exists(tempDir.resolve("environment_detector.py")), "Should create environment_detector.py");
    }
    
    @Test
    void testGeneratedInitFile() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("__init__.py"));
        
        // Should import all peripherals
        assertTrue(content.contains("from player_detector import PlayerDetector"));
        assertTrue(content.contains("from chat_box import ChatBox"));
        assertTrue(content.contains("from environment_detector import EnvironmentDetector"));
        
        // Should have __all__
        assertTrue(content.contains("__all__"));
    }
    
    @Test
    void testGeneratedPlayerDetector() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("player_detector.py"));
        
        // Should have class definition
        assertTrue(content.contains("class PlayerDetector:"), "Should have class PlayerDetector");
        
        // Should import peripheral
        assertTrue(content.contains("import peripheral"), "Should import peripheral");
        
        // Should have __init__ method
        assertTrue(content.contains("def __init__(self, name):"), "Should have __init__");
        assertTrue(content.contains("self.name = name"), "Should set self.name");
        
        // Should have find() static method
        assertTrue(content.contains("@staticmethod"), "Should have @staticmethod");
        assertTrue(content.contains("def find():"), "Should have find()");
        assertTrue(content.contains("peripheral.find('playerDetector')"), "Should call peripheral.find");
        
        // Should have method wrappers - check for snake_case versions
        assertTrue(content.contains("def get_online_players(self)"), "Should have get_online_players");
        assertTrue(content.contains("def get_players_in_range(self"), "Should have get_players_in_range");
        assertTrue(content.contains("def get_player_pos(self"), "Should have get_player_pos");
        
        // Should use peripheral.call
        assertTrue(content.contains("peripheral.call(self.name,"), "Should use peripheral.call");
    }
    
    @Test
    void testGeneratedPythonSyntax() throws IOException {
        generator.generate(definitions, tempDir);
        
        // Check that all generated files have valid Python syntax by checking structure
        for (PeripheralDefinition peripheral : definitions.getPeripherals()) {
            Path file = tempDir.resolve(peripheral.toPythonModuleName() + ".py");
            String content = Files.readString(file);
            
            // Basic syntax checks
            assertBalancedParentheses(content, '(', ')');
            assertBalancedParentheses(content, '[', ']');
            assertBalancedParentheses(content, '{', '}');
            
            // Should have matching def/class indentation
            assertTrue(content.contains("class "), "Should have class definition");
            assertTrue(content.contains("def "), "Should have method definitions");
            
            // No obvious syntax errors
            assertFalse(content.contains("def def"), "No double def");
            assertFalse(content.contains("class class"), "No double class");
        }
    }
    
    @Test
    void testGeneratedMethodSignatures() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("player_detector.py"));
        
        // Methods with no parameters
        assertTrue(content.contains("def get_online_players(self)"), "Should have get_online_players");
        
        // Methods with parameters - parameter names come from Java reflection
        // Java compiled without -parameters flag uses arg0, arg1, etc.
        assertTrue(content.contains("def get_players_in_range(self"), "Should have get_players_in_range");
        
        // Methods with multiple parameters
        assertTrue(content.contains("def get_players_in_coords(self"), "Should have get_players_in_coords");
        assertTrue(content.contains("def is_player_in_range(self"), "Should have is_player_in_range");
    }
    
    @Test
    void testGeneratedDocstrings() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("player_detector.py"));
        
        // Should have docstrings
        assertTrue(content.contains("\"\"\""), "Should have docstrings");
        assertTrue(content.contains("Args:") || content.contains("Returns:"), 
                "Should have Args or Returns in docstrings");
    }
    
    @Test
    void testGeneratedHelperFunctions() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("player_detector.py"));
        
        // Should have helper functions for parsing
        assertTrue(content.contains("def _parse_json_result"));
        assertTrue(content.contains("def _parse_string_array"));
    }
    
    @Test
    void testGenerateSingleFile() {
        String content = generator.generateSingleFile(definitions);
        
        // Should contain all peripheral classes
        assertTrue(content.contains("class PlayerDetector:"));
        assertTrue(content.contains("class ChatBox:"));
        assertTrue(content.contains("class EnvironmentDetector:"));
        
        // Should be valid Python structure
        assertBalancedParentheses(content, '(', ')');
        assertBalancedParentheses(content, '[', ']');
    }
    
    @Test
    void testGeneratedChatBox() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("chat_box.py"));
        
        // Should have class
        assertTrue(content.contains("class ChatBox:"), "Should have ChatBox class");
        
        // Should have void-returning methods
        assertTrue(content.contains("def send_message(self"), "Should have send_message");
        
        // Should have methods with multiple string params
        assertTrue(content.contains("def send_to_player(self"), "Should have send_to_player");
    }
    
    @Test
    void testGeneratedEnvironmentDetector() throws IOException {
        generator.generate(definitions, tempDir);
        
        String content = Files.readString(tempDir.resolve("environment_detector.py"));
        
        // Should have all methods
        assertTrue(content.contains("def get_biome(self):"));
        assertTrue(content.contains("def get_light_level(self):"));
        assertTrue(content.contains("def get_weather(self):"));
        assertTrue(content.contains("def is_raining(self):"));
        assertTrue(content.contains("def get_temperature(self):"));
    }
    
    // Helper methods
    
    private void assertBalancedParentheses(String content, char open, char close) {
        int depth = 0;
        boolean inString = false;
        char stringChar = 0;
        
        for (int i = 0; i < content.length(); i++) {
            char c = content.charAt(i);
            
            // Track string state (simplified - doesn't handle all edge cases)
            if ((c == '"' || c == '\'') && (i == 0 || content.charAt(i-1) != '\\')) {
                if (!inString) {
                    inString = true;
                    stringChar = c;
                } else if (c == stringChar) {
                    inString = false;
                }
            }
            
            if (!inString) {
                if (c == open) depth++;
                else if (c == close) depth--;
            }
        }
        
        assertEquals(0, depth, "Unbalanced " + open + close + " in generated code");
    }
}
