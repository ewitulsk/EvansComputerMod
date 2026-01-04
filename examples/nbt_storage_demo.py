# NBT Storage Demo
# Demonstrates persistent data storage

import terminal
import json

def main():
    terminal.clear()
    terminal.println("=== NBT Storage Demo ===")
    terminal.println("")
    
    if not terminal.nbt_storage_available():
        terminal.println("NBT storage not available!")
        terminal.println("Make sure AP integration is installed.")
        return
    
    terminal.println("NBT storage is available!")
    terminal.println("")
    
    # Read existing data
    terminal.println("Reading stored data...")
    existing = terminal.nbt_read()
    if existing:
        try:
            data = json.loads(existing)
            terminal.println(f"Found {len(data)} stored entries:")
            for key, value in list(data.items())[:5]:
                terminal.println(f"  {key}: {value}")
            if len(data) > 5:
                terminal.println(f"  ... and {len(data) - 5} more")
        except:
            terminal.println(f"Raw data: {existing[:50]}...")
    else:
        data = {}
        terminal.println("No existing data found.")
    
    terminal.println("")
    
    # Write some new data
    terminal.println("Writing new data...")
    
    # Merge with existing data
    if not isinstance(data, dict):
        data = {}
    
    # Add some entries
    data["last_run"] = str(terminal.get_time())
    data["counter"] = data.get("counter", 0) + 1
    data["biome"] = terminal.get_biome() or "unknown"
    data["dimension"] = terminal.get_dimension() or "unknown"
    
    # Save
    json_str = json.dumps(data)
    if terminal.nbt_write(json_str):
        terminal.println("Data saved successfully!")
    else:
        terminal.println("Failed to save data!")
    
    terminal.println("")
    
    # Verify
    terminal.println("Verifying saved data...")
    verify = terminal.nbt_read()
    if verify:
        try:
            saved = json.loads(verify)
            terminal.println(f"Verified {len(saved)} entries:")
            for key, value in saved.items():
                terminal.println(f"  {key}: {value}")
        except:
            terminal.println("Verification failed to parse")
    
    terminal.println("")
    terminal.println("Demo complete!")
    terminal.println("")
    terminal.println("Run this script again to see")
    terminal.println("the counter increment!")

main()
