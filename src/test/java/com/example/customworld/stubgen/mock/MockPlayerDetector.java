package com.example.customworld.stubgen.mock;

import java.util.List;
import java.util.Map;

/**
 * Mock peripheral mimicking Advanced Peripherals' PlayerDetector.
 * Used for testing the stub generation without requiring the actual mod.
 */
public class MockPlayerDetector {
    
    /**
     * Get all online players on the server.
     * @return List of player names
     */
    @MockLuaFunction
    public String[] getOnlinePlayers() {
        return new String[] { "Steve", "Alex" };
    }
    
    /**
     * Get players within the specified range of the detector.
     * @param range The range in blocks
     * @return List of player names within range
     */
    @MockLuaFunction
    public String[] getPlayersInRange(int range) {
        return new String[] { "Steve" };
    }
    
    /**
     * Get players within the specified coordinates.
     * @param x X coordinate
     * @param y Y coordinate  
     * @param z Z coordinate
     * @param range Range in blocks
     * @return List of player names
     */
    @MockLuaFunction
    public String[] getPlayersInCoords(int x, int y, int z, int range) {
        return new String[] {};
    }
    
    /**
     * Get the position of a specific player.
     * @param playerName The player's name
     * @return Map with x, y, z coordinates
     */
    @MockLuaFunction(mainThread = true)
    public Map<String, Double> getPlayerPos(String playerName) {
        return Map.of("x", 0.0, "y", 64.0, "z", 0.0);
    }
    
    /**
     * Check if a player is within range.
     * @param range The range to check
     * @param playerName The player's name
     * @return true if player is within range
     */
    @MockLuaFunction
    public boolean isPlayerInRange(int range, String playerName) {
        return true;
    }
    
    /**
     * Check if any player is within range.
     * @param range The range to check
     * @return true if any player is within range
     */
    @MockLuaFunction
    public boolean isPlayersInRange(int range) {
        return true;
    }
}
