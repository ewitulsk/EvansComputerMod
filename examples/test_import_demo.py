# Test script to verify the custom import system works
# This script imports test_lib.py from the virtual filesystem

import terminal

# Test importing the custom module
try:
    from test_lib import greet, add, VERSION
    
    terminal.println("=== Import System Test ===")
    terminal.println("")
    terminal.println("Successfully imported test_lib module!")
    terminal.println("")
    
    # Test the greet function
    message = greet("Player")
    terminal.println(f"greet('Player') = {message}")
    
    # Test the add function
    result = add(5, 3)
    terminal.println(f"add(5, 3) = {result}")
    
    # Test the module variable
    terminal.println(f"VERSION = {VERSION}")
    terminal.println("")
    terminal.println("✓ All imports working correctly!")
    
except ImportError as e:
    terminal.println(f"Import failed: {e}")
except Exception as e:
    terminal.println(f"Error: {e}")
