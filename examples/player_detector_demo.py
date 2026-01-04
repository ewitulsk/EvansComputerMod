# Player Detector Demo
# This script demonstrates the player detector functions
# Requires the Advanced Peripherals integration mod to be installed

import terminal

def main():
    terminal.clear()
    terminal.println("=== Player Detector Demo ===")
    terminal.println("")
    
    # Check if player detector is available
    if not terminal.player_detector_available():
        terminal.println("Player detector not available!")
        terminal.println("Make sure the AP integration mod is installed.")
        return
    
    terminal.println("Player detector is available!")
    terminal.println("")
    
    # Get player count
    count = terminal.get_player_count()
    if count is not None:
        terminal.println(f"Online players: {count}")
    
    terminal.println("")
    
    # Get all online players
    players = terminal.get_online_players()
    if players:
        terminal.println("All online players:")
        for name in players:
            terminal.println(f"  - {name}")
    else:
        terminal.println("No players online (or error)")
    
    terminal.println("")
    
    # Get players within 50 blocks
    nearby = terminal.get_players_in_range(50)
    if nearby:
        terminal.println("Players within 50 blocks:")
        for name in nearby:
            terminal.println(f"  - {name}")
            
            # Get detailed info for each nearby player
            info = terminal.get_player_info(name)
            if info:
                terminal.println(f"    Position: ({info['x']:.1f}, {info['y']:.1f}, {info['z']:.1f})")
                terminal.println(f"    Dimension: {info['dimension']}")
                terminal.println(f"    Health: {info['health']:.1f}/{info['max_health']:.1f}")
    else:
        terminal.println("No players within 50 blocks")
    
    terminal.println("")
    terminal.println("Demo complete!")

# Run the demo
main()
