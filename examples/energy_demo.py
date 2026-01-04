# Energy Detector Demo
# Demonstrates energy transfer monitoring

import terminal

def main():
    terminal.clear()
    terminal.println("=== Energy Detector Demo ===")
    terminal.println("")
    
    if not terminal.energy_detector_available():
        terminal.println("Energy detector not available!")
        terminal.println("Make sure AP integration is installed.")
        return
    
    terminal.println("Energy detector is available!")
    terminal.println("")
    
    # Get current transfer rate
    rate = terminal.get_energy_transfer_rate()
    if rate is not None:
        terminal.println(f"Current Transfer Rate: {rate} FE/t")
    else:
        terminal.println("Could not read transfer rate")
    
    # Get current limit
    limit = terminal.get_energy_transfer_limit()
    if limit is not None:
        if limit == -1:
            terminal.println("Transfer Rate Limit: Unlimited")
        else:
            terminal.println(f"Transfer Rate Limit: {limit} FE/t")
    
    terminal.println("")
    
    # Set a new limit
    terminal.println("Setting transfer limit to 1000 FE/t...")
    if terminal.set_energy_transfer_limit(1000):
        terminal.println("Limit set successfully!")
        
        # Verify
        new_limit = terminal.get_energy_transfer_limit()
        terminal.println(f"New limit: {new_limit} FE/t")
    else:
        terminal.println("Failed to set limit")
    
    terminal.println("")
    
    # Monitor for a few seconds
    terminal.println("Monitoring energy for 5 seconds...")
    for i in range(5):
        terminal.sleep(1.0)
        rate = terminal.get_energy_transfer_rate()
        if rate is not None:
            terminal.println(f"  [{i+1}s] Rate: {rate} FE/t")
    
    terminal.println("")
    terminal.println("Demo complete!")
    terminal.println("")
    terminal.println("Note: Place an Energy Detector block")
    terminal.println("adjacent to the terminal for real readings.")

main()
