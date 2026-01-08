package com.example.customworld.stubgen.mock;

/**
 * Mock peripheral mimicking Advanced Peripherals' ChatBox.
 * Used for testing the stub generation without requiring the actual mod.
 */
public class MockChatBox {
    
    /**
     * Send a message to all players.
     * @param message The message to send
     */
    @MockLuaFunction
    public void sendMessage(String message) {
        // Mock implementation
    }
    
    /**
     * Send a message to all players with a prefix.
     * @param message The message to send
     * @param prefix The prefix for the message
     */
    @MockLuaFunction
    public void sendMessageToPlayer(String message, String prefix) {
        // Mock implementation
    }
    
    /**
     * Send a message to a specific player.
     * @param message The message content
     * @param playerName The target player
     * @param prefix Optional prefix
     */
    @MockLuaFunction(mainThread = true)
    public void sendToPlayer(String message, String playerName, String prefix) {
        // Mock implementation
    }
    
    /**
     * Get the chat history.
     * @return Array of recent chat messages
     */
    @MockLuaFunction
    public String[] getChatHistory() {
        return new String[] { "[Player] Hello!", "[System] Welcome!" };
    }
    
    /**
     * Check if the chat box is ready.
     * @return true if ready
     */
    @MockLuaFunction
    public boolean isReady() {
        return true;
    }
}
