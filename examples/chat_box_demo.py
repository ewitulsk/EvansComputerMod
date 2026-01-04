# Chat Box Demo
# Demonstrates sending messages to players

import terminal

def main():
    terminal.clear()
    terminal.println("=== Chat Box Demo ===")
    terminal.println("")
    
    if not terminal.chat_available():
        terminal.println("Chat box not available!")
        terminal.println("Make sure AP integration is installed.")
        return
    
    terminal.println("Chat box is available!")
    terminal.println("")
    
    # Get list of online players
    players = terminal.get_online_players()
    if not players:
        terminal.println("No players online to message.")
        return
    
    terminal.println(f"Online players: {', '.join(players)}")
    terminal.println("")
    
    # Send a broadcast message to all players in the same dimension
    terminal.println("Sending broadcast message...")
    count = terminal.send_chat("Hello from the Terminal!")
    terminal.println(f"Message sent to {count} players.")
    terminal.println("")
    
    # Send a private message to the first player
    if players:
        player = players[0]
        terminal.println(f"Sending private message to {player}...")
        if terminal.send_chat_to_player(player, "This is a private terminal message!"):
            terminal.println("Private message sent!")
        else:
            terminal.println("Failed to send private message.")
        terminal.println("")
        
        # Send a toast notification
        terminal.println(f"Sending toast to {player}...")
        if terminal.send_toast(player, "Terminal Alert", "Important notification!"):
            terminal.println("Toast sent!")
        else:
            terminal.println("Failed to send toast.")
    
    terminal.println("")
    terminal.println("Demo complete!")

main()
