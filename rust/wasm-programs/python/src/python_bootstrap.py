# Python bootstrap code for virtual filesystem import support
# This file is executed when the Python interpreter starts to set up
# custom import hooks that allow importing .py files from the virtual filesystem.

import sys
import builtins

# Get ModuleType from an existing module (avoids needing 'types' module)
ModuleType = type(sys)

# Save original __import__ BEFORE we do anything
_original_import = builtins.__import__

# Import shell module using the original import (before we replace it)
import shell as _terminal_module


def _virtual_fs_import(name, globals=None, locals=None, fromlist=(), level=0):
    """Custom __import__ that checks virtual filesystem first."""

    # For relative imports or already-loaded modules, use original import
    if level > 0 or name in sys.modules:
        return _original_import(name, globals, locals, fromlist, level)

    # Check if module exists in virtual filesystem
    module_file = name.replace('.', '/') + '.py'
    if _terminal_module.file_exists(module_file):
        # Load from virtual filesystem
        return _load_virtual_module(name, module_file, False)

    # Check for package
    package_init = name.replace('.', '/') + '/__init__.py'
    if _terminal_module.file_exists(package_init):
        return _load_virtual_module(name, package_init, True)

    # Fall back to original import
    return _original_import(name, globals, locals, fromlist, level)


def _load_virtual_module(fullname, filepath, is_package):
    """Load a module from the virtual filesystem."""

    # Return cached module if already loaded
    if fullname in sys.modules:
        return sys.modules[fullname]

    # Read source code
    source = _terminal_module.read_file(filepath)
    if source is None:
        raise ImportError("Cannot read " + filepath)

    # Create module
    module = ModuleType(fullname)
    module.__file__ = filepath
    module.__name__ = fullname

    if is_package:
        idx = filepath.rfind('/')
        module.__path__ = [filepath[:idx] if idx >= 0 else '']
        module.__package__ = fullname
    else:
        idx = fullname.rfind('.')
        module.__package__ = fullname[:idx] if idx >= 0 else ''

    # Register before executing (handles circular imports)
    sys.modules[fullname] = module

    # Compile and execute
    try:
        code = compile(source, filepath, 'exec')
        exec(code, module.__dict__)
    except:
        del sys.modules[fullname]
        raise

    return module


# The `peripheral` module is Python source shipped inside the native
# `_peripheral` module; build it once so `import peripheral` finds it in
# sys.modules.
import _peripheral as _peripheral_native
_peripheral_module = ModuleType('peripheral')
_peripheral_module.__file__ = '<peripheral>'
exec(compile(_peripheral_native._source, '<peripheral>', 'exec'), _peripheral_module.__dict__)
sys.modules['peripheral'] = _peripheral_module

# `sensors` (Wired Sensor Module / lidar helpers) ships the same way.
_sensors_module = ModuleType('sensors')
_sensors_module.__file__ = '<sensors>'
exec(compile(_peripheral_native._sensors_source, '<sensors>', 'exec'), _sensors_module.__dict__)
sys.modules['sensors'] = _sensors_module

# `controller` (Wireless Xbox Controller) too.
_controller_module = ModuleType('controller')
_controller_module.__file__ = '<controller>'
exec(compile(_peripheral_native._controller_source, '<controller>', 'exec'), _controller_module.__dict__)
sys.modules['controller'] = _controller_module

# `audio` (Speaker / /dev/audio).
_audio_module = ModuleType('audio')
_audio_module.__file__ = '<audio>'
exec(compile(_peripheral_native._audio_source, '<audio>', 'exec'), _audio_module.__dict__)
sys.modules['audio'] = _audio_module

# `radio` (SDR flowgraphs) ships inside the native `_radio` module.
import _radio as _radio_native
_radio_module = ModuleType('radio')
_radio_module.__file__ = '<radio>'
exec(compile(_radio_native._source, '<radio>', 'exec'), _radio_module.__dict__)
sys.modules['radio'] = _radio_module

# Replace __import__ with our custom version
builtins.__import__ = _virtual_fs_import


# === Redirect sys.stdout/sys.stderr to terminal ===
# This makes Python's built-in print() and expression display work.

class _TerminalWriter:
    """File-like object that writes to the terminal via the terminal module."""
    def __init__(self):
        self.encoding = 'utf-8'
        self.errors = 'strict'
    def write(self, s):
        if s:
            _terminal_module.write(str(s))
        return len(s) if s else 0
    def flush(self):
        pass
    def fileno(self):
        raise OSError("no file descriptor in WASM environment")
    def isatty(self):
        return True
    def readable(self):
        return False
    def writable(self):
        return True

sys.stdout = _TerminalWriter()
sys.stderr = _TerminalWriter()


# === Replace builtin input() with the echoing version from the shell module ===
# The default Rust stdin read_line() doesn't echo characters as the user types,
# so we route input() through shell.input() which reads byte-by-byte and echoes.
def _shell_input(prompt=""):
    return _terminal_module.input(prompt)
builtins.input = _shell_input
