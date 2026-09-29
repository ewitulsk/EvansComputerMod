package com.example.evanscomputermod.api.peripheral;

/**
 * Thrown by a peripheral method to report a user-facing error. The message is
 * raised in the calling program as {@code peripheral.PeripheralError}.
 */
public class PeripheralException extends Exception {

    public PeripheralException(String message) {
        super(message);
    }
}
