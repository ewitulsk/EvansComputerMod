package com.example.customworld.stubgen;

import com.example.customworld.stubgen.mock.MockChatBox;
import com.example.customworld.stubgen.mock.MockEnvironmentDetector;
import com.example.customworld.stubgen.mock.MockLuaFunction;
import com.example.customworld.stubgen.mock.MockPlayerDetector;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Tests for PeripheralScanner.
 * Verifies that the scanner correctly extracts method information from
 * classes annotated with @LuaFunction (or our mock equivalent).
 */
class PeripheralScannerTest {
    
    private PeripheralScanner scanner;
    
    @BeforeEach
    void setUp() {
        // Create scanner with our mock annotation
        scanner = new PeripheralScanner(MockLuaFunction.class);
    }
    
    @Test
    void testScanPlayerDetector() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        assertNotNull(def);
        assertEquals("playerDetector", def.getType());
        assertEquals(MockPlayerDetector.class.getName(), def.getClassName());
        
        // Should find all @MockLuaFunction methods
        List<MethodDefinition> methods = def.getMethods();
        assertFalse(methods.isEmpty(), "Should find annotated methods");
        
        // Verify specific methods exist
        assertTrue(hasMethod(methods, "getOnlinePlayers"), "Should find getOnlinePlayers");
        assertTrue(hasMethod(methods, "getPlayersInRange"), "Should find getPlayersInRange");
        assertTrue(hasMethod(methods, "getPlayerPos"), "Should find getPlayerPos");
        assertTrue(hasMethod(methods, "isPlayerInRange"), "Should find isPlayerInRange");
    }
    
    @Test
    void testScanPlayerDetector_MethodParameters() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Find getPlayersInRange method
        MethodDefinition getPlayersInRange = findMethod(def.getMethods(), "getPlayersInRange");
        assertNotNull(getPlayersInRange, "Should find getPlayersInRange");
        
        // Should have one parameter
        List<ParameterDefinition> params = getPlayersInRange.getParameters();
        assertEquals(1, params.size(), "Should have one parameter");
        assertEquals("int", params.get(0).getType(), "Parameter should be int");
    }
    
    @Test
    void testScanPlayerDetector_MultipleParameters() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Find getPlayersInCoords method (4 parameters)
        MethodDefinition method = findMethod(def.getMethods(), "getPlayersInCoords");
        assertNotNull(method, "Should find getPlayersInCoords");
        
        List<ParameterDefinition> params = method.getParameters();
        assertEquals(4, params.size(), "Should have 4 parameters");
    }
    
    @Test
    void testScanPlayerDetector_ReturnTypes() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // getOnlinePlayers returns String[]
        MethodDefinition getOnline = findMethod(def.getMethods(), "getOnlinePlayers");
        assertNotNull(getOnline);
        assertEquals("String[]", getOnline.getReturnType());
        
        // isPlayerInRange returns boolean
        MethodDefinition isInRange = findMethod(def.getMethods(), "isPlayerInRange");
        assertNotNull(isInRange);
        assertEquals("boolean", isInRange.getReturnType());
    }
    
    @Test
    void testScanPlayerDetector_MainThreadAnnotation() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // getPlayerPos should have mainThread=true
        MethodDefinition getPos = findMethod(def.getMethods(), "getPlayerPos");
        assertNotNull(getPos);
        assertTrue(getPos.isMainThread(), "getPlayerPos should be mainThread=true");
        
        // getOnlinePlayers should have mainThread=false (default)
        MethodDefinition getOnline = findMethod(def.getMethods(), "getOnlinePlayers");
        assertNotNull(getOnline);
        assertFalse(getOnline.isMainThread(), "getOnlinePlayers should be mainThread=false");
    }
    
    @Test
    void testScanChatBox() {
        PeripheralDefinition def = scanner.scanClass(MockChatBox.class, "chatBox");
        
        assertNotNull(def);
        assertEquals("chatBox", def.getType());
        
        // Verify methods
        assertTrue(hasMethod(def.getMethods(), "sendMessage"));
        assertTrue(hasMethod(def.getMethods(), "getChatHistory"));
        assertTrue(hasMethod(def.getMethods(), "isReady"));
    }
    
    @Test
    void testScanChatBox_VoidReturnType() {
        PeripheralDefinition def = scanner.scanClass(MockChatBox.class, "chatBox");
        
        MethodDefinition sendMessage = findMethod(def.getMethods(), "sendMessage");
        assertNotNull(sendMessage);
        assertEquals("void", sendMessage.getReturnType());
    }
    
    @Test
    void testScanEnvironmentDetector() {
        PeripheralDefinition def = scanner.scanClass(MockEnvironmentDetector.class, "environmentDetector");
        
        assertNotNull(def);
        assertEquals("environmentDetector", def.getType());
        
        // Should have many methods
        assertTrue(def.getMethods().size() >= 10, "Should find at least 10 methods");
        
        // Verify specific methods
        assertTrue(hasMethod(def.getMethods(), "getBiome"));
        assertTrue(hasMethod(def.getMethods(), "getLightLevel"));
        assertTrue(hasMethod(def.getMethods(), "getWeather"));
        assertTrue(hasMethod(def.getMethods(), "getTemperature"));
    }
    
    @Test
    void testScanEnvironmentDetector_NumericReturnTypes() {
        PeripheralDefinition def = scanner.scanClass(MockEnvironmentDetector.class, "environmentDetector");
        
        // getLightLevel returns int
        MethodDefinition getLight = findMethod(def.getMethods(), "getLightLevel");
        assertNotNull(getLight);
        assertEquals("int", getLight.getReturnType());
        
        // getDayTime returns long
        MethodDefinition getDay = findMethod(def.getMethods(), "getDayTime");
        assertNotNull(getDay);
        assertEquals("long", getDay.getReturnType());
        
        // getTemperature returns double
        MethodDefinition getTemp = findMethod(def.getMethods(), "getTemperature");
        assertNotNull(getTemp);
        assertEquals("double", getTemp.getReturnType());
    }
    
    @Test
    void testScanMultipleClasses() {
        Map<String, Class<?>> classes = new HashMap<>();
        classes.put("playerDetector", MockPlayerDetector.class);
        classes.put("chatBox", MockChatBox.class);
        classes.put("environmentDetector", MockEnvironmentDetector.class);
        
        PeripheralDefinitions definitions = scanner.scanClasses(classes);
        
        assertNotNull(definitions);
        assertEquals(3, definitions.getPeripherals().size(), "Should have 3 peripherals");
    }
    
    @Test
    void testPythonNamingConversion() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Test Python class name conversion
        assertEquals("PlayerDetector", def.toPythonClassName());
        assertEquals("player_detector", def.toPythonModuleName());
        
        // Test method name conversion
        MethodDefinition getOnline = findMethod(def.getMethods(), "getOnlinePlayers");
        assertNotNull(getOnline);
        assertEquals("get_online_players", getOnline.toPythonName());
    }
    
    @Test
    void testRustNamingConversion() {
        PeripheralDefinition def = scanner.scanClass(MockPlayerDetector.class, "playerDetector");
        
        // Test Rust struct name conversion
        assertEquals("PlayerDetector", def.toRustStructName());
        assertEquals("player_detector", def.toRustModuleName());
        
        // Test method name conversion
        MethodDefinition getOnline = findMethod(def.getMethods(), "getOnlinePlayers");
        assertNotNull(getOnline);
        assertEquals("get_online_players", getOnline.toRustName());
    }
    
    // Helper methods
    
    private boolean hasMethod(List<MethodDefinition> methods, String name) {
        return methods.stream().anyMatch(m -> m.getLuaName().equals(name));
    }
    
    private MethodDefinition findMethod(List<MethodDefinition> methods, String name) {
        return methods.stream()
                .filter(m -> m.getLuaName().equals(name))
                .findFirst()
                .orElse(null);
    }
}
