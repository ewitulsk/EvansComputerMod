package com.example.evanscomputermod.computer.peripheral;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.Level;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

/**
 * The peripherals one computer can see: blocks on its six sides (through
 * {@link PeripheralCapability#PERIPHERAL}) and modules in its bays. Owned by
 * the computer's block entity; all attach/detach work runs on the server
 * thread, while programs query and call through a published immutable
 * snapshot from their own threads.
 *
 * <p>Attaching or detaching posts {@code peripheral} / {@code peripheral_detach}
 * events to the computer.
 */
public final class PeripheralHub {

    /** Side names, relative to the computer's facing (same convention as redstone sides). */
    public static final String[] SIDE_NAMES = {"bottom", "top", "front", "back", "left", "right"};

    /** How long a program waits for the server thread to run a main-thread call. */
    private static final long MAIN_THREAD_TIMEOUT_MS = 10_000;

    /** What the hub needs from its owner. */
    public interface Owner {
        @Nullable Level level();

        BlockPos pos();

        Direction facing();

        UUID computerId();

        /** The running computer's event bus, or null while it's off. */
        @Nullable PeripheralEventBus events();
    }

    private final class Attachment implements IComputerAccess {
        final String name;
        final IPeripheral peripheral;

        Attachment(String name, IPeripheral peripheral) {
            this.name = name;
            this.peripheral = peripheral;
        }

        @Override
        public String getAttachmentName() {
            return name;
        }

        @Override
        public UUID getComputerId() {
            return owner.computerId();
        }

        @Override
        public void queueEvent(String event, Object... arguments) {
            if (attached.get(name) != this) return; // detached: stale access
            post(event, name, arguments);
        }

        @Override
        public Map<String, IPeripheral> getAttachedPeripherals() {
            Map<String, IPeripheral> out = new LinkedHashMap<>();
            for (Attachment a : attached.values()) out.put(a.name, a.peripheral);
            return out;
        }
    }

    private final Owner owner;
    private final Map<String, IPeripheral> sides = new LinkedHashMap<>();
    private final Map<String, IPeripheral> modules = new LinkedHashMap<>();
    /** Published snapshot (immutable), read by program threads. */
    private volatile Map<String, Attachment> attached = Map.of();

    public PeripheralHub(Owner owner) {
        this.owner = owner;
    }

    // ------------------------------------------------------------ server thread

    /** Relative side name for an absolute direction, given the computer's facing. */
    public static String sideName(Direction facing, Direction dir) {
        if (dir == Direction.DOWN) return "bottom";
        if (dir == Direction.UP) return "top";
        if (dir == facing) return "front";
        if (dir == facing.getOpposite()) return "back";
        if (dir == facing.getCounterClockWise()) return "left";
        return "right";
    }

    /** Look up block peripherals on all six sides. Server thread. */
    public void rescanSides() {
        Level level = owner.level();
        if (level == null || level.isClientSide()) return;
        sides.clear();
        BlockPos pos = owner.pos();
        Direction facing = owner.facing();
        for (Direction dir : Direction.values()) {
            BlockPos neighbor = pos.relative(dir);
            if (!level.isLoaded(neighbor)) continue;
            IPeripheral p = level.getCapability(PeripheralCapability.PERIPHERAL, neighbor, dir.getOpposite());
            if (p != null) sides.put(sideName(facing, dir), p);
        }
        apply();
    }

    /** Set or clear the module peripheral in a bay slot. Server thread. */
    public void setModule(String slotName, @Nullable IPeripheral module) {
        if (module == null) modules.remove(slotName);
        else modules.put(slotName, module);
        apply();
    }

    /** Detach everything (block removed / chunk unloaded / moving). Server thread. */
    public void detachAll() {
        sides.clear();
        modules.clear();
        apply();
    }

    private void apply() {
        Map<String, IPeripheral> desired = new LinkedHashMap<>(sides);
        desired.putAll(modules);

        Map<String, Attachment> old = attached;
        Map<String, Attachment> next = new LinkedHashMap<>();
        List<Attachment> detached = new ArrayList<>();
        List<Attachment> added = new ArrayList<>();
        for (Map.Entry<String, IPeripheral> e : desired.entrySet()) {
            Attachment prev = old.get(e.getKey());
            if (prev != null && (prev.peripheral == e.getValue() || prev.peripheral.isSame(e.getValue()))) {
                next.put(e.getKey(), prev);
            } else {
                if (prev != null) detached.add(prev);
                Attachment a = new Attachment(e.getKey(), e.getValue());
                next.put(e.getKey(), a);
                added.add(a);
            }
        }
        for (Map.Entry<String, Attachment> e : old.entrySet()) {
            if (!desired.containsKey(e.getKey())) detached.add(e.getValue());
        }
        if (detached.isEmpty() && added.isEmpty()) return;

        attached = Collections.unmodifiableMap(next);
        for (Attachment a : detached) {
            try {
                a.peripheral.detach(a);
            } catch (RuntimeException ex) {
                EvansComputerMod.LOGGER.warn("Peripheral {} ({}) failed in detach", a.name, a.peripheral.getType(), ex);
            }
            post("peripheral_detach", a.name);
        }
        for (Attachment a : added) {
            try {
                a.peripheral.attach(a);
            } catch (RuntimeException ex) {
                EvansComputerMod.LOGGER.warn("Peripheral {} ({}) failed in attach", a.name, a.peripheral.getType(), ex);
            }
            post("peripheral", a.name, a.peripheral.getType());
        }
    }

    private void post(String event, String attachment, Object... args) {
        PeripheralEventBus bus = owner.events();
        if (bus != null) bus.post(event, attachment, args);
    }

    // ------------------------------------------------------------ program threads

    /** Encoded result: LIST of {@code [name, type]} pairs. */
    public byte[] list() {
        List<List<String>> out = new ArrayList<>();
        for (Attachment a : attached.values()) {
            out.add(List.of(a.name, a.peripheral.getType()));
        }
        return PeripheralValues.ok(out);
    }

    /** Encoded result: {@code [type, [method names...]]}, or an error if nothing is attached as {@code name}. */
    public byte[] methods(String name) {
        Attachment a = attached.get(name);
        if (a == null) return PeripheralValues.error("no peripheral named '" + name + "'");
        return PeripheralValues.ok(List.of(a.peripheral.getType(), new ArrayList<>(a.peripheral.getMethodNames())));
    }

    /**
     * Call {@code method} on the peripheral attached as {@code name}.
     * Blocks the calling program thread until the server thread has run the
     * call (for main-thread methods).
     */
    public byte[] call(String name, String method, byte[] encodedArgs, @Nullable MinecraftServer server) {
        Attachment a = attached.get(name);
        if (a == null) return PeripheralValues.error("no peripheral named '" + name + "'");
        if (!a.peripheral.getMethodNames().contains(method)) {
            return PeripheralValues.error(a.peripheral.getType() + " has no method '" + method + "'");
        }
        Object[] args;
        try {
            args = PeripheralValues.decodeArgs(encodedArgs);
        } catch (PeripheralValues.DecodeException e) {
            return PeripheralValues.error("malformed arguments: " + e.getMessage());
        }

        if (!a.peripheral.runsOnMainThread(method) || server == null || server.isSameThread()) {
            return invoke(a, method, args);
        }
        CompletableFuture<byte[]> future = server.submit(() -> {
            if (attached.get(name) != a) {
                return PeripheralValues.error("peripheral '" + name + "' was detached");
            }
            return invoke(a, method, args);
        });
        try {
            return future.get(MAIN_THREAD_TIMEOUT_MS, TimeUnit.MILLISECONDS);
        } catch (InterruptedException e) {
            future.cancel(false);
            Thread.currentThread().interrupt();
            throw new RuntimeException("peripheral call interrupted");
        } catch (TimeoutException e) {
            future.cancel(false);
            return PeripheralValues.error("peripheral call timed out (server busy)");
        } catch (ExecutionException e) {
            return PeripheralValues.error("internal error: " + e.getCause());
        }
    }

    private static byte[] invoke(Attachment a, String method, Object[] args) {
        try {
            return PeripheralValues.ok(a.peripheral.callMethod(a, method, args));
        } catch (PeripheralException e) {
            return PeripheralValues.error(e.getMessage());
        } catch (RuntimeException e) {
            EvansComputerMod.LOGGER.error("Peripheral {} ({}) method {} threw", a.name, a.peripheral.getType(), method, e);
            return PeripheralValues.error("internal error in " + method + ": " + e);
        }
    }

    /** Current attachment names (for tests and debugging). */
    public List<String> names() {
        return new ArrayList<>(attached.keySet());
    }

    @Nullable
    public IPeripheral get(String name) {
        Attachment a = attached.get(name);
        return a == null ? null : a.peripheral;
    }
}
