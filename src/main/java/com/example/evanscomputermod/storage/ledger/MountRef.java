package com.example.evanscomputermod.storage.ledger;

//? if <=1.21.1 {

/**
 * Where a cell is mounted (a drive slot, a storage module). A cell id can be
 * live in one mount at a time; a copy of the same cell elsewhere is refused.
 */
public interface MountRef {

    /** Whether this mount is still in the world (a removed block entity is stale and can be taken over). */
    boolean isAlive();

    /** For messages: "drive at 10 64 -3, slot 2". */
    String describe();
}
//?}
