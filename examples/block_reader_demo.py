# Block Reader Demo
# Demonstrates reading information about blocks in front of the terminal

import terminal
import json

def main():
    terminal.clear()
    terminal.println("=== Block Reader Demo ===")
    terminal.println("")
    
    if not terminal.block_reader_available():
        terminal.println("Block reader not available!")
        terminal.println("Make sure AP integration is installed.")
        return
    
    terminal.println("Block reader is available!")
    terminal.println("Reading block in front of terminal...")
    terminal.println("")
    
    # Get block name
    block_name = terminal.get_block_name()
    if block_name:
        terminal.println(f"Block: {block_name}")
    else:
        terminal.println("Could not read block name")
        return
    
    # Check if it's a tile entity
    is_te = terminal.is_tile_entity()
    if is_te is not None:
        terminal.println(f"Is Tile Entity: {is_te}")
    terminal.println("")
    
    # Get block states
    terminal.println("Block States:")
    states_json = terminal.get_block_states()
    if states_json and states_json != "null":
        try:
            states = json.loads(states_json)
            if states:
                for key, value in states.items():
                    terminal.println(f"  {key}: {value}")
            else:
                terminal.println("  (no states)")
        except:
            terminal.println(f"  Raw: {states_json}")
    else:
        terminal.println("  (no states or not available)")
    
    terminal.println("")
    
    # Get block data (NBT) if it's a tile entity
    if is_te:
        terminal.println("Block Data (NBT):")
        data_json = terminal.get_block_data()
        if data_json and data_json != "null":
            try:
                data = json.loads(data_json)
                # Print first few keys
                keys = list(data.keys())[:5]
                for key in keys:
                    value = str(data[key])[:50]  # Truncate long values
                    terminal.println(f"  {key}: {value}")
                if len(data) > 5:
                    terminal.println(f"  ... and {len(data) - 5} more")
            except:
                # Just show raw if parsing fails
                if len(data_json) > 100:
                    terminal.println(f"  {data_json[:100]}...")
                else:
                    terminal.println(f"  {data_json}")
        else:
            terminal.println("  (no data)")
    
    terminal.println("")
    terminal.println("Demo complete!")
    terminal.println("")
    terminal.println("Tip: Place different blocks in front of")
    terminal.println("the terminal and run this script again!")

main()
