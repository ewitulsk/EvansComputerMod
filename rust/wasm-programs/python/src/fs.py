"""The computer's files, by path from its root ("progs/a.lua" and
"/progs/a.lua" are the same file). The Python counterpart of Lua's `fs`
module (same functions; Lua also has ComputerCraft's camelCase names).

    import fs
    fs.list("/")                       # sorted names
    fs.make_dir("data")
    h = fs.open("data/log.txt", "a")
    h.write_line("hello"); h.close()
    for p in fs.find("progs/*.lua"): print(p, fs.get_size(p))
    fs.combine("a", "../b")            # "b"

fs.open(path, mode) gives a handle, or (None, message) if it can't:
  "r"  read_line(keep_newline), read_all(), read(n), close()
  "w"/"a"  write(text), write_line(text), flush(), close()
  "rb"/"wb"/"ab"  read() a byte as an int, read(n), write(int or bytes), seek(whence, offset)
Errors (OSError) are worded as CC's: "/x: No such file", "/x: File exists".
"""

import _console


def _bytes(text):
    """A str as bytes, one byte per character (past 255: "?"), as the term draws text."""
    return bytes(ord(c) if ord(c) < 256 else 63 for c in text)


def _text(data):
    return "".join(map(chr, data))

list = _console.fs_list
exists = _console.fs_exists
is_dir = _console.fs_is_dir
is_read_only = _console.fs_is_read_only
get_size = _console.fs_get_size
make_dir = _console.fs_make_dir
delete = _console.fs_delete
copy = _console.fs_copy
move = _console.fs_move
find = _console.fs_find
combine = _console.fs_combine
get_name = _console.fs_get_name
get_dir = _console.fs_get_dir
attributes = _console.fs_attributes


def get_free_space(path="/"):
    """Free bytes on the drive, or "unlimited"."""
    n = _console.fs_get_free_space(path)
    return "unlimited" if n is None else n


def get_drive(path):
    """The drive a path is on ("hdd"), or None if it doesn't exist."""
    return "hdd" if _console.fs_exists(path) else None


def get_capacity(path):
    """The drive's size in bytes, or None (the computer's storage has no fixed size here)."""
    if not _console.fs_exists(path):
        raise OSError("/" + combine(path) + ": No such file")
    return None


def is_drive_root(path):
    return combine(path) == ""


class Handle:
    """An open file, as fs.open gives it."""

    def __init__(self, f, mode):
        self._f = f
        self._binary = "b" in mode
        self._open = True

    def _check(self):
        if not self._open:
            raise ValueError("attempt to use a closed file")

    def read_line(self, keep_newline=False):
        self._check()
        line = self._f.readline()
        if not line:
            return None
        if isinstance(line, bytes):
            line = _text(line)
        return line if keep_newline else line.rstrip("\n")

    def read_all(self):
        self._check()
        data = self._f.read()
        if isinstance(data, bytes) and not self._binary:
            data = _text(data)
        return data

    def read(self, count=None):
        self._check()
        if self._binary and count is None:
            b = self._f.read(1)
            return b[0] if b else None
        data = self._f.read(1 if count is None else count)
        if isinstance(data, bytes) and not self._binary:
            data = _text(data)
        return data if data else None

    def write(self, value):
        self._check()
        if self._binary:
            if isinstance(value, int):
                value = bytes([value % 256])
            elif isinstance(value, str):
                value = _bytes(value)
            self._f.write(value)
        else:
            self._f.write(_bytes(str(value)))

    def write_line(self, value):
        self.write(str(value) + "\n")

    def flush(self):
        self._check()
        self._f.flush()

    def seek(self, whence="cur", offset=0):
        self._check()
        self._f.seek(offset, {"set": 0, "cur": 1, "end": 2}[whence])
        return self._f.tell()

    def close(self):
        if self._open:
            self._open = False
            self._f.close()


def open(path, mode="r"):
    """Open a file: a Handle, or (None, why not)."""
    if mode not in ("r", "w", "a", "rb", "wb", "ab"):
        raise ValueError("Unsupported mode")
    name = "/" + combine(path)
    if name.startswith("/.."):
        return None, name + ": Invalid Path"
    if is_dir(path):
        return None, name + ": Is a directory"
    if mode[0] == "r" and not exists(path):
        return None, name + ": No such file"
    if mode[0] != "r":
        d = get_dir(path)
        if d not in ("", ".."):
            make_dir(d)
    import builtins
    try:
        f = builtins.open(_console.fs_real_path(path), mode[0] + "b")
    except OSError as e:
        return None, name + ": " + str(e)
    return Handle(f, mode)
