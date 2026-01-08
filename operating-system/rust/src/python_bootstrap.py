# Python bootstrap code for virtual filesystem import support
# This file is executed when the Python interpreter starts to set up
# custom import hooks that allow importing .py files from the virtual filesystem.
#
# Note: We avoid using 'importlib' and 'types' modules as they may not be 
# available in RustPython's minimal WASM build.

import sys
import builtins

# Get ModuleType from an existing module (avoids needing 'types' module)
ModuleType = type(sys)

# Save original __import__ BEFORE we do anything
_original_import = builtins.__import__

# Import terminal using the original import (before we replace it)
# This avoids recursion when _virtual_fs_import tries to use terminal
import terminal as _terminal_module


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


# Replace __import__ with our custom version
builtins.__import__ = _virtual_fs_import
