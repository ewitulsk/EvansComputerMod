package com.example.customworld.stubgen;

import com.example.customworld.stubgen.mock.MockChatBox;
import com.example.customworld.stubgen.mock.MockEnvironmentDetector;
import com.example.customworld.stubgen.mock.MockLuaFunction;
import com.example.customworld.stubgen.mock.MockPlayerDetector;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Tests for the runtime stub generation logic.
 * 
 * Note: These tests verify the underlying scanning and generation logic
 * using mock peripherals with @MockLuaFunction annotation.
 * 
 * RuntimeStubGenerator itself cannot be directly tested because it depends on
 * NeoForge's ModList which is not available in unit tests. Instead, we test
 * the PeripheralScanner and PythonStubGenerator classes that it uses internally.
 */
class RuntimeStubGeneratorTest {
    
    private PeripheralScanner scanner;
    
    @BeforeEach
    void setUp() {
        // Use mock annotation for testing
        scanner = new PeripheralScanner(MockLuaFunction.class);
    }
    
    @Test
    void testRegisterPeripheralWithClass() {
        // Test the underlying logic that RuntimeStubGenerator uses
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        assertNotNull(def);
        assertEquals("playerDetector", def.getType());
        assertFalse(def.getMethods().isEmpty(), "Should have methods");
    }
    
    @Test
    void testScannerFindsAllMethods() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Check specific methods exist
        boolean hasGetOnlinePlayers = def.getMethods().stream()
                .anyMatch(m -> m.getLuaName().equals("getOnlinePlayers"));
        boolean hasGetPlayersInRange = def.getMethods().stream()
                .anyMatch(m -> m.getLuaName().equals("getPlayersInRange"));
        boolean hasGetPlayerPos = def.getMethods().stream()
                .anyMatch(m -> m.getLuaName().equals("getPlayerPos"));
        
        assertTrue(hasGetOnlinePlayers, "Should find getOnlinePlayers");
        assertTrue(hasGetPlayersInRange, "Should find getPlayersInRange");
        assertTrue(hasGetPlayerPos, "Should find getPlayerPos");
    }
    
    @Test
    void testPythonGeneratorFromScannedClass() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        PeripheralDefinitions defs = new PeripheralDefinitions();
        defs.addPeripheral(def);
        
        PythonStubGenerator pythonGen = new PythonStubGenerator();
        String pythonCode = pythonGen.generateSingleFile(defs);
        
        assertNotNull(pythonCode);
        assertTrue(pythonCode.contains("class PlayerDetector:"), "Should have class definition");
        assertTrue(pythonCode.contains("def get_online_players(self)"), "Should have method");
        assertTrue(pythonCode.contains("peripheral.call"), "Should call peripheral.call");
    }
    
    @Test
    void testGenerateIndividualStubsSimulation() {
        // Simulate what RuntimeStubGenerator.generateIndividualStubs() does
        // by manually scanning and generating
        
        PeripheralDefinition playerDef = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        PeripheralDefinition chatDef = scanner.scanClass(MockChatBox.class, "chatBox");
        
        PeripheralDefinitions defs = new PeripheralDefinitions();
        defs.addPeripheral(playerDef);
        defs.addPeripheral(chatDef);
        
        PythonStubGenerator pythonGen = new PythonStubGenerator();
        
        // Generate single file with all peripherals
        String combined = pythonGen.generateSingleFile(defs);
        
        assertNotNull(combined);
        assertTrue(combined.contains("class PlayerDetector:"));
        assertTrue(combined.contains("class ChatBox:"));
    }
    
    @Test
    void testNamingConventions() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Test Python naming
        assertEquals("PlayerDetector", def.toPythonClassName());
        assertEquals("player_detector", def.toPythonModuleName());
        
        // Test method naming
        MethodDefinition method = def.getMethods().stream()
                .filter(m -> m.getLuaName().equals("getOnlinePlayers"))
                .findFirst()
                .orElse(null);
        
        assertNotNull(method);
        assertEquals("get_online_players", method.toPythonName());
    }
    
    @Test
    void testParameterExtraction() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Find method with parameters
        MethodDefinition method = def.getMethods().stream()
                .filter(m -> m.getLuaName().equals("getPlayersInRange"))
                .findFirst()
                .orElse(null);
        
        assertNotNull(method, "Should find getPlayersInRange");
        assertFalse(method.getParameters().isEmpty(), "Should have parameters");
        assertEquals("int", method.getParameters().get(0).getType());
    }
    
    @Test
    void testReturnTypeExtraction() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // getOnlinePlayers returns String[]
        MethodDefinition getPlayers = def.getMethods().stream()
                .filter(m -> m.getLuaName().equals("getOnlinePlayers"))
                .findFirst()
                .orElse(null);
        
        assertNotNull(getPlayers);
        assertEquals("String[]", getPlayers.getReturnType());
        
        // isPlayerInRange returns boolean
        MethodDefinition isInRange = def.getMethods().stream()
                .filter(m -> m.getLuaName().equals("isPlayerInRange"))
                .findFirst()
                .orElse(null);
        
        assertNotNull(isInRange);
        assertEquals("boolean", isInRange.getReturnType());
    }
    
    @Test
    void testMainThreadAnnotation() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // getPlayerPos has mainThread=true
        MethodDefinition getPos = def.getMethods().stream()
                .filter(m -> m.getLuaName().equals("getPlayerPos"))
                .findFirst()
                .orElse(null);
        
        assertNotNull(getPos);
        assertTrue(getPos.isMainThread(), "getPlayerPos should be mainThread=true");
    }
    
    @Test
    void testEnvironmentDetectorMethods() {
        PeripheralDefinition def = scanner.scanClass(MockEnvironmentDetector.class, "environmentDetector");
        
        // Should have many methods
        assertTrue(def.getMethods().size() >= 10, "Should have at least 10 methods");
        
        // Check various return types
        boolean hasIntMethod = def.getMethods().stream()
                .anyMatch(m -> m.getReturnType().equals("int"));
        boolean hasLongMethod = def.getMethods().stream()
                .anyMatch(m -> m.getReturnType().equals("long"));
        boolean hasDoubleMethod = def.getMethods().stream()
                .anyMatch(m -> m.getReturnType().equals("double"));
        boolean hasBoolMethod = def.getMethods().stream()
                .anyMatch(m -> m.getReturnType().equals("boolean"));
        
        assertTrue(hasIntMethod, "Should have int-returning method");
        assertTrue(hasLongMethod, "Should have long-returning method");
        assertTrue(hasDoubleMethod, "Should have double-returning method");
        assertTrue(hasBoolMethod, "Should have boolean-returning method");
    }
    
    @Test
    void testChatBoxVoidMethods() {
        PeripheralDefinition def = scanner.scanClass(MockChatBox.class, "chatBox");
        
        // sendMessage returns void
        MethodDefinition sendMsg = def.getMethods().stream()
                .filter(m -> m.getLuaName().equals("sendMessage"))
                .findFirst()
                .orElse(null);
        
        assertNotNull(sendMsg);
        assertEquals("void", sendMsg.getReturnType());
    }
}
