# Geo Scanner Demo
# Demonstrates block and ore scanning

import terminal
import json

def main():
    terminal.clear()
    terminal.println("=== Geo Scanner Demo ===")
    terminal.println("")
    
    if not terminal.geo_scanner_available():
        terminal.println("Geo scanner not available!")
        terminal.println("Make sure AP integration is installed.")
        return
    
    terminal.println("Geo scanner is available!")
    terminal.println("")
    
    # Check scan cost
    radius = 8
    cost = terminal.geo_scan_cost(radius)
    if cost is not None:
        terminal.println(f"Scan cost for radius {radius}: {cost} fuel")
    terminal.println("")
    
    # Scan blocks in small radius
    terminal.println(f"Scanning blocks (radius {radius})...")
    result = terminal.geo_scan(radius)
    if result:
        try:
            blocks = json.loads(result)
            terminal.println(f"Found {len(blocks)} non-air blocks")
            
            # Count block types
            block_counts = {}
            for block in blocks:
                name = block.get('name', 'unknown')
                # Get short name
                short_name = name.split(':')[-1] if ':' in name else name
                block_counts[short_name] = block_counts.get(short_name, 0) + 1
            
            # Show top 10 most common
            terminal.println("")
            terminal.println("Most common blocks:")
            sorted_blocks = sorted(block_counts.items(), key=lambda x: -x[1])
            for name, count in sorted_blocks[:10]:
                terminal.println(f"  {name}: {count}")
        except:
            terminal.println("Error parsing scan results")
    else:
        terminal.println("Scan failed or returned no results")
    
    terminal.println("")
    
    # Chunk ore analysis
    terminal.println("Analyzing chunk for ores...")
    ore_result = terminal.geo_chunk_analyze()
    if ore_result:
        try:
            ores = json.loads(ore_result)
            if ores:
                terminal.println("Ores found in this chunk:")
                for ore, count in ores.items():
                    short_name = ore.split(':')[-1] if ':' in ore else ore
                    terminal.println(f"  {short_name}: {count}")
            else:
                terminal.println("No ores found in this chunk.")
        except:
            terminal.println("Error parsing ore analysis")
    else:
        terminal.println("Ore analysis failed")
    
    terminal.println("")
    terminal.println("Demo complete!")

main()
