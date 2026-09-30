package com.example.evanscomputermod.storage.core;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.PeripheralException;

/**
 * A storage operation that was refused (cell full, not enough items, unknown
 * token, ...). Raised in programs as {@code PeripheralError}.
 */
public class StorageException extends PeripheralException {

    public StorageException(String message) {
        super(message);
    }
}
//?}
