package com.example.evanscomputermod.api.peripheral;

import java.util.UUID;

/**
 * A peripheral's handle on one computer it is attached to. One peripheral can
 * be attached to several computers at once (a block between two terminals),
 * each through its own {@code IComputerAccess}.
 */
public interface IComputerAccess {

    /** The name this computer knows the peripheral by, e.g. {@code "left"} or {@code "right_bay_2"}. */
    String getAttachmentName();

    /** The computer's persistent id. */
    UUID getComputerId();

    /**
     * Queue an event for programs on this computer waiting in
     * {@code peripheral.pull_event()}. Programs receive the tuple
     * {@code (event, attachment_name, *arguments)}. Safe to call from any
     * thread; dropped if the computer is not running. Arguments use the same
     * value mapping as {@link IPeripheral#callMethod} results.
     */
    void queueEvent(String event, Object... arguments);
}
