package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.core.CellTier;

import java.util.UUID;

/** A cell that is live in a device slot. */
public record MountedCell(UUID id, CellTier tier, IStorageDevice device, int slot) {

    /** Short id for display: the first 8 hex digits. */
    public String shortId() {
        return id.toString().substring(0, 8);
    }
}
//?}
