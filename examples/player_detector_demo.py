# Player Detector Demo
# This script demonstrates the player detector functions using the peripheral module
# Requires CC:Tweaked and Advanced Peripherals to be installed
# Place a Player Detector block adjacent to the terminal

import terminal
import peripheral
from player_detector import PlayerDetector

def main():
    terminal.clear()
    terminal.println("=== Player Detector Demo ===")
    terminal.println("")

    peripherals = peripheral.list()
    terminal.println(f"Available Peripherals: {peripherals}")
    
    # Find a player detector peripheral using the wrapper
    detector = PlayerDetector.find()
    
    if not detector:
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
    
    terminal.println(f"Found player detector: {detector.name}")
    terminal.println("")
    
    # Show available methods
    methods = peripheral.get_methods(detector.name)
    if methods:
        terminal.println("Available methods:")
        for m in methods[:10]:  # Show first 10 methods
            terminal.println(f"  - {m}")
        if len(methods) > 10:
            terminal.println(f"  ... and {len(methods) - 10} more")
        terminal.println("")
    
    # Get all online players
    terminal.println("Fetching online players...")
    online_players = detector.get_online_players()
    terminal.println(f"Online players: {online_players}")
    terminal.println("")
    
    # Get players within range (50 blocks)
    terminal.println("Fetching players within 50 blocks...")
    nearby_players = detector.get_players_in_range(50)
    terminal.println(f"Nearby players: {nearby_players}")
    terminal.println("")
    
    # Try to get player position
    terminal.println("Attempting to get player position...")
    if online_players:
        # The getPlayerPos method returns position data
        player_pos = detector.get_player_pos()
        terminal.println(f"Player position: {player_pos}")
    else:
        terminal.println("No players online to look up!")
    terminal.println("")
    
    terminal.println("Demo complete!")
    terminal.println("")
    terminal.println("Tip: Use the PlayerDetector wrapper in your own scripts:")
    terminal.println("  from player_detector import PlayerDetector")
    terminal.println("  detector = PlayerDetector.find()")
    terminal.println("  players = detector.get_online_players()")

# Run the demo
main()
