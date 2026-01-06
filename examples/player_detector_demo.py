# Player Detector Demo
# This script demonstrates the player detector functions using the peripheral module
# Requires CC:Tweaked and Advanced Peripherals to be installed
# Place a Player Detector block adjacent to the terminal

import terminal
import peripheral

def parse_result(json_str):
    """Parse a JSON result string, handling the ok/error wrapper."""
    try:
        # Simple JSON parsing for common cases
        if json_str == "null":
            return None
        if json_str.startswith('"') and json_str.endswith('"'):
            return json_str[1:-1]
        if json_str == "true":
            return True
        if json_str == "false":
            return False
        # Try to parse as number
        try:
            if '.' in json_str:
                return float(json_str)
            return int(json_str)
        except:
            pass
        # For arrays/objects, we'd need more complex parsing
        # For now, return the raw string
        return json_str
    except:
        return json_str

def main():
    terminal.clear()
    terminal.println("=== Player Detector Demo ===")
    terminal.println("")

    peripherals = peripheral.list()
    terminal.println(f"Available Peripherals: {peripherals}")
    
    # Find a player detector peripheral
    detector_name = peripheral.find("player_detector")
    
    if not detector_name:
        terminal.println("Player detector not available!")
        terminal.println("Make sure a Player Detector block is adjacent")
        terminal.println("to this terminal.")
        terminal.println("")
        
        # Show what peripherals ARE available
        peripherals = peripheral.list()
        if peripherals:
            terminal.println("Available peripherals:")
            for p in peripherals:
                terminal.println(f"  - {p['name']} ({p['type']})")
        else:
            terminal.println("No peripherals connected.")
        return
    
    terminal.println(f"Found player detector: {detector_name}")
    terminal.println("")
    
    # Show available methods
    methods = peripheral.get_methods(detector_name)
    if methods:
        terminal.println("Available methods:")
        for m in methods[:10]:  # Show first 10 methods
            terminal.println(f"  - {m}")
        if len(methods) > 10:
            terminal.println(f"  ... and {len(methods) - 10} more")
        terminal.println("")
    
    # Get all online players
    terminal.println("Fetching online players...")
    result = peripheral.call(detector_name, "getOnlinePlayers", "[]")
    parsed = parse_result(result)
    terminal.println(f"Online players: {parsed}")
    terminal.println("")
    
    # Get players within range (50 blocks)
    terminal.println("Fetching players within 50 blocks...")
    result = peripheral.call(detector_name, "getPlayersInRange", "[50]")
    parsed = parse_result(result)
    terminal.println(f"Nearby players: {parsed}")
    terminal.println("")
    
    # Try to get player position (if a player name is known)
    # This requires knowing a player name - we'll try to get it from the online list
    terminal.println("Attempting to get player position...")
    # The getPlayerPos method takes a player name as argument
    # Example: peripheral.call(detector_name, "getPlayerPos", '["PlayerName"]')
    result = peripheral.call(detector_name, "getPlayerPos", '[""]')  # Empty string for "any player"
    parsed = parse_result(result)
    terminal.println(f"Player position: {parsed}")
    terminal.println("")
    
    terminal.println("Demo complete!")
    terminal.println("")
    terminal.println("Tip: Use these functions in your own scripts:")
    terminal.println("  import peripheral")
    terminal.println("  name = peripheral.find('playerDetector')")
    terminal.println("  peripheral.call(name, 'getOnlinePlayers', '[]')")

# Run the demo
main()
