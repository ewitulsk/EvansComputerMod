package com.example.evanscomputermod.disk;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import org.jetbrains.annotations.Nullable;

import java.nio.file.Path;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The {@code drive} peripheral of a Disk Drive, as ComputerCraft's: while a
 * floppy is in, every computer it is attached to sees the disk's files as
 * {@code disk/} (or {@code disk2/} ... when that name is taken).
 *
 * <p>Events: {@code "disk"} when a floppy goes in, {@code "disk_eject"} when
 * it comes out (both with the drive's attachment name).
 */
public final class DiskDrivePeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "drive";

    private final DiskDriveBlockEntity drive;
    /** Each attached computer, and the name the disk is mounted as there (null: not mounted). */
    private final Map<IComputerAccess, String> computers = new ConcurrentHashMap<>();

    DiskDrivePeripheral(DiskDriveBlockEntity drive) {
        this.drive = drive;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public void attach(IComputerAccess computer) {
        computers.put(computer, "");
        mount(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        unmount(computer);
        computers.remove(computer);
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other instanceof DiskDrivePeripheral p && p.drive == drive;
    }

    private void mount(IComputerAccess computer) {
        Path folder = drive.diskFolder();
        if (folder == null) return;
        String name = computer.mount("disk", folder, false);
        computers.put(computer, name == null ? "" : name);
    }

    private void unmount(IComputerAccess computer) {
        String name = computers.get(computer);
        if (name != null && !name.isEmpty()) computer.unmount(name);
        computers.put(computer, "");
    }

    /** A floppy went in: mount it everywhere, then tell the programs. */
    void diskInserted() {
        for (IComputerAccess c : computers.keySet()) {
            mount(c);
            c.queueEvent("disk");
        }
    }

    /** The floppy came out: unmount it, then tell the programs. */
    void diskRemoved() {
        for (IComputerAccess c : computers.keySet()) {
            unmount(c);
            c.queueEvent("disk_eject");
        }
    }

    @PeripheralMethod(description = "Whether a disk is in the drive")
    public boolean isDiskPresent() {
        return drive.hasDisk();
    }

    @PeripheralMethod(description = "Whether the disk in the drive holds files (a floppy does): false when empty")
    public boolean hasData() {
        return drive.hasDisk();
    }

    @PeripheralMethod(description = "Where this computer sees the disk's files (\"disk\", \"disk2\" ...), or nil")
    @Nullable
    public String getMountPath(IComputerAccess computer) {
        String name = computers.get(computer);
        return name == null || name.isEmpty() ? null : name;
    }

    @PeripheralMethod(description = "The disk's label (the item's name), or nil")
    @Nullable
    public String getDiskLabel() {
        return drive.label();
    }

    @PeripheralMethod(description = "Set the disk's label (nil clears it)")
    public void setDiskLabel(@Nullable String label) throws PeripheralException {
        if (!drive.hasDisk()) throw new PeripheralException("No disk in the drive");
        if (label != null && label.length() > 32) label = label.substring(0, 32);
        drive.setLabel(label);
    }

    @PeripheralMethod(description = "The disk's number (its folder in the world), or nil")
    @Nullable
    public Integer getDiskId() {
        return drive.hasDisk() ? drive.diskId() : null;
    }

    @PeripheralMethod(description = "Push the disk out of the front of the drive")
    public void ejectDisk() {
        drive.eject();
    }
}
//?}
