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

    /**
     * Another peripheral attached to the same computer, by its attachment
     * name ({@code "right"}, {@code "left_bay_1"}), or null. Peripherals that
     * move things between each other (an inventory's {@code push_items}) find
     * their partner this way.
     */
    @org.jetbrains.annotations.Nullable
    default IPeripheral getAvailablePeripheral(String name) {
        return null;
    }

    /**
     * Mount a folder into the computer's filesystem as {@code base} (or
     * {@code base2}, {@code base3} ... if that is taken), the way a Disk
     * Drive mounts its floppy as {@code disk/}. Returns the name it got, or
     * null if this computer can't mount folders. Unmount before detaching.
     */
    @org.jetbrains.annotations.Nullable
    default String mount(String base, java.nio.file.Path root, boolean readOnly) {
        return null;
    }

    /** Take a folder mounted with {@link #mount} out again. */
    default void unmount(String name) {
    }
}
