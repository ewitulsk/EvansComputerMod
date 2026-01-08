import terminal
# Save a module to the virtual filesystem
terminal.write_file("mymath.py", """
def add(a, b):
    return a + b

PI = 3.14159
""")

# Now you can import it!
import mymath
terminal.println(mymath.add(2, 3))  # 5
terminal.println(mymath.PI)         # 3.14159