# Environment Detector Demo
# Demonstrates weather, time, biome, and light level detection

import terminal

def main():
    terminal.clear()
    terminal.println("=== Environment Detector Demo ===")
    terminal.println("")
    
    if not terminal.environment_available():
        terminal.println("Environment detector not available!")
        terminal.println("Make sure AP integration is installed.")
        return
    
    # Current dimension and biome
    terminal.println(f"Dimension: {terminal.get_dimension()}")
    terminal.println(f"Biome: {terminal.get_biome()}")
    terminal.println("")
    
    # Time info
    time = terminal.get_time()
    if time is not None:
        day = time // 24000
        time_of_day = time % 24000
        terminal.println(f"World Time: Day {day}, {time_of_day} ticks")
    
    # Moon phase
    moon_id = terminal.get_moon_id()
    moon_name = terminal.get_moon_name()
    if moon_id is not None:
        terminal.println(f"Moon Phase: {moon_name} (ID: {moon_id})")
    terminal.println("")
    
    # Weather
    terminal.println("Weather Status:")
    if terminal.is_sunny():
        terminal.println("  - Sunny")
    if terminal.is_raining():
        terminal.println("  - Raining")
    if terminal.is_thunder():
        terminal.println("  - Thundering")
    terminal.println("")
    
    # Light levels
    terminal.println("Light Levels:")
    terminal.println(f"  Sky Light: {terminal.get_sky_light_level()}")
    terminal.println(f"  Block Light: {terminal.get_block_light_level()}")
    terminal.println(f"  Day Light: {terminal.get_day_light_level()}")
    terminal.println("")
    
    # Slime chunk check
    if terminal.is_slime_chunk():
        terminal.println("This IS a slime chunk!")
    else:
        terminal.println("This is NOT a slime chunk.")
    terminal.println("")
    
    # Available dimensions
    dims = terminal.list_dimensions()
    if dims:
        terminal.println(f"Available Dimensions ({len(dims)}):")
        for dim in dims:
            terminal.println(f"  - {dim}")
    
    terminal.println("")
    terminal.println("Demo complete!")

main()
