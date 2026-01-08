package com.example.customworld.stubgen.mock;

/**
 * Mock peripheral mimicking Advanced Peripherals' EnvironmentDetector.
 * Used for testing the stub generation without requiring the actual mod.
 */
public class MockEnvironmentDetector {
    
    /**
     * Get the current biome.
     * @return The biome name
     */
    @MockLuaFunction
    public String getBiome() {
        return "minecraft:plains";
    }
    
    /**
     * Get the current light level.
     * @return Light level 0-15
     */
    @MockLuaFunction
    public int getLightLevel() {
        return 15;
    }
    
    /**
     * Get the sky light level.
     * @return Sky light level 0-15
     */
    @MockLuaFunction
    public int getSkyLightLevel() {
        return 15;
    }
    
    /**
     * Get the block light level.
     * @return Block light level 0-15
     */
    @MockLuaFunction
    public int getBlockLightLevel() {
        return 0;
    }
    
    /**
     * Get the current day time.
     * @return The day time in ticks
     */
    @MockLuaFunction
    public long getDayTime() {
        return 6000L;
    }
    
    /**
     * Get the current weather.
     * @return Weather type (clear, rain, thunder)
     */
    @MockLuaFunction
    public String getWeather() {
        return "clear";
    }
    
    /**
     * Check if it's currently raining.
     * @return true if raining
     */
    @MockLuaFunction
    public boolean isRaining() {
        return false;
    }
    
    /**
     * Check if it's currently thundering.
     * @return true if thundering
     */
    @MockLuaFunction
    public boolean isThundering() {
        return false;
    }
    
    /**
     * Get the moon phase.
     * @return Moon phase 0-7
     */
    @MockLuaFunction
    public int getMoonPhase() {
        return 0;
    }
    
    /**
     * Get the temperature at the detector's location.
     * @return Temperature value
     */
    @MockLuaFunction
    public double getTemperature() {
        return 0.8;
    }
    
    /**
     * Get the humidity at the detector's location.
     * @return Humidity value
     */
    @MockLuaFunction
    public double getHumidity() {
        return 0.4;
    }
}
