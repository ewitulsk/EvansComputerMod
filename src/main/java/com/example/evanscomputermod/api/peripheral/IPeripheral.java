package com.example.evanscomputermod.api.peripheral;

import org.jetbrains.annotations.Nullable;

import java.util.Set;

/**
 * Something a computer can talk to: a block next to it (exposed through
 * {@link PeripheralCapability#PERIPHERAL}) or a module installed in one of its
 * bays (see {@link com.example.evanscomputermod.api.module.IComputerModule}).
 *
 * <p>Programs reach a peripheral by its attachment name ({@code "left"},
 * {@code "back"}, {@code "left_bay_1"}, ...). From Python:
 * <pre>
 * import peripheral
 * link = peripheral.find("redstone_link")
 * link.set_output(0, 15)
 * </pre>
 *
 * <p>Most implementations extend {@link AnnotatedPeripheral} and mark their
 * callable methods with {@link PeripheralMethod} instead of implementing
 * {@link #getMethodNames()} / {@link #callMethod} by hand.
 *
 * <p>Threading: {@link #callMethod} runs on the server thread unless
 * {@link #runsOnMainThread} returns false for that method, in which case it
 * runs on the calling program's thread and must be thread-safe.
 * {@link #attach} and {@link #detach} always run on the server thread.
 */
public interface IPeripheral {

    /** Type name programs search for with {@code peripheral.find(type)}, e.g. {@code "redstone_link"}. */
    String getType();

    /** Names of the methods programs may call. */
    Set<String> getMethodNames();

    /**
     * Invoke a method. Arguments arrive decoded from the program: {@code null},
     * {@link Boolean}, {@link Long} / {@link Integer}, {@link Double},
     * {@link String}, {@code byte[]}, {@link java.util.List} and
     * {@link java.util.Map}. The return value is encoded back the same way
     * (see {@code PeripheralValues} for the full mapping).
     *
     * @throws PeripheralException to raise a {@code PeripheralError} in the program
     */
    @Nullable
    Object callMethod(IComputerAccess computer, String method, Object[] arguments) throws PeripheralException;

    /** Whether {@code method} must run on the server thread (the default). */
    default boolean runsOnMainThread(String method) {
        return true;
    }

    /** A computer can now see this peripheral under {@link IComputerAccess#getAttachmentName()}. */
    default void attach(IComputerAccess computer) {
    }

    /** The computer can no longer see this peripheral (removed, moved away, computer gone). */
    default void detach(IComputerAccess computer) {
    }

    /**
     * Whether {@code other} is the same peripheral. When a computer rescans
     * its neighbours, a peripheral that is still the same is kept attached
     * (no detach / attach events). Block capabilities are usually looked up
     * afresh, so implementations that create a new wrapper per lookup should
     * compare the underlying block entity.
     */
    default boolean isSame(IPeripheral other) {
        return this == other;
    }
}
