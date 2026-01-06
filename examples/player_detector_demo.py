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

def parse_string_array(json_str):
    """Parse a JSON array of strings like ["Player1", "Player2"] into a Python list."""
    try:
        json_str = json_str.strip()
        if not json_str.startswith('[') or not json_str.endswith(']'):
            return []
        
        # Remove brackets
        inner = json_str[1:-1].strip()
        if not inner:
            return []
        
        # Split by comma and extract quoted strings
        result = []
        for item in inner.split(','):
            item = item.strip()
            if item.startswith('"') and item.endswith('"'):
                result.append(item[1:-1])
            elif item:
                result.append(item)
        return result
    except:
        return []

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
    online_players = parse_string_array(result)
    terminal.println(f"Online players: {online_players}")
    terminal.println("")
    
    # Get players within range (50 blocks)
    terminal.println("Fetching players within 50 blocks...")
    result = peripheral.call(detector_name, "getPlayersInRange", "[50]")
    nearby_players = parse_string_array(result)
    terminal.println(f"Nearby players: {nearby_players}")
    terminal.println("")
    
    # Try to get player position using a real player name
    terminal.println("Attempting to get player position...")
    if online_players:
        # Use the first online player's name
        player_name = online_players[0]
        terminal.println(f"Looking up position for: {player_name}")
        # The getPlayerPos method takes a player name as argument
        result = peripheral.call(detector_name, "getPlayerPos", f'["{player_name}"]')
        terminal.println(f"Result: {result}")
        parsed = parse_result(result)
        terminal.println(f"Player position: {parsed}")
    else:
        terminal.println("No players online to look up!")
    terminal.println("")
    
    terminal.println("Demo complete!")
    terminal.println("")
    terminal.println("Tip: Use these functions in your own scripts:")
    terminal.println("  import peripheral")
    terminal.println("  name = peripheral.find('playerDetector')")
    terminal.println("  peripheral.call(name, 'getOnlinePlayers', '[]')")

# Run the demo
main()
