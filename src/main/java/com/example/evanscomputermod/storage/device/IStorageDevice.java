package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.IComputerAccess;

import java.util.Collection;
import java.util.List;
import java.util.UUID;

/**
 * Something that holds mounted Storage Cells: a Drive, or a Storage Module in
 * a computer's bay. Its id is persistent; it is the issuer of tokens taken
 * from its cells.
 */
public interface IStorageDevice {

    UUID deviceId();

    /** {@code "drive"} or {@code "module"}. */
    String kind();

    /** Cells mounted right now (duplicates of a cell mounted elsewhere are left out). */
    List<MountedCell> cells();

    /** Higher first when choosing where items go (AE2-style). */
    default int priority() {
        return 0;
    }

    boolean isLive();

    /** For messages. */
    String describe();

    /** Computers to tell about changes (attached directly or through a wired bus). */
    Collection<IComputerAccess> watchers();
}
//?}
