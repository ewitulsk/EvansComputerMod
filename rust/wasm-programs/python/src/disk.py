"""Disks in Disk Drives, by the drive's attachment name ("left", "top" ...).
The Python counterpart of Lua's `disk` module (same functions). A floppy in a
drive next to the computer is the folder disk/ (disk2/ ... for more drives).

    import disk
    if disk.is_present("left"):
        print(disk.get_mount_path("left"))     # "disk"
        print(disk.get_label("left"), disk.get_id("left"))
        disk.set_label("left", "backups")
        disk.eject("left")

Events (events.pull, peripheral.pull_event): ("disk", side) when a floppy goes
in, ("disk_eject", side) when it comes out.
"""

import peripheral


def _drive(name):
    if not isinstance(name, str):
        raise TypeError("name must be a str")
    return peripheral.get_type(name) == "drive"


def _call(name, method, *args):
    if not _drive(name):
        return None
    return peripheral.call(name, method, *args)


def is_present(name):
    return bool(_drive(name) and _call(name, "is_disk_present"))


def has_data(name):
    return bool(_drive(name) and _call(name, "has_data"))


def get_mount_path(name):
    return _call(name, "get_mount_path")


def get_label(name):
    return _call(name, "get_disk_label")


def set_label(name, label=None):
    if _drive(name):
        _call(name, "set_disk_label", label)


def get_id(name):
    return _call(name, "get_disk_id")


def eject(name):
    if _drive(name):
        _call(name, "eject_disk")


def has_audio(name):
    """Music discs need a jukebox drive: none here."""
    _drive(name)
    return False


def get_audio_title(name):
    _drive(name)
    return None


def play_audio(name):
    _drive(name)


def stop_audio(name):
    _drive(name)
