package com.example.evanscomputermod.storage.api;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.StorageException;
import com.example.evanscomputermod.storage.device.DecoderBlockEntity;
import com.example.evanscomputermod.storage.device.IItemPort;
import com.example.evanscomputermod.storage.device.IStorageDevice;
import com.example.evanscomputermod.storage.device.MountedCell;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.server.MinecraftServer;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * Everything one computer can reach for storage, built per call from what is
 * attached to it: drives and decoders next to it, Storage Modules in its bays,
 * and devices on its Wired Bus Module. Cells and ports in the same net can
 * exchange items; the computer is the router. Server thread.
 */
public final class StorageNet {

    /** A peripheral that contributes storage devices or ports to its computer's net. */
    public interface Node {
        void contribute(Builder net, String attachment);
    }

    public final class Builder {
        private Builder() {
        }

        public void device(IStorageDevice device) {
            if (device.isLive() && devices.put(device, Boolean.TRUE) == null) order.add(device);
        }

        public void port(String name, IItemPort port) {
            if (!port.isLive()) return;
            for (IItemPort p : ports.values()) if (p == port) return;
            String unique = name;
            for (int i = 2; ports.containsKey(unique); i++) unique = name + "_" + i;
            ports.put(unique, port);
        }
    }

    public final StorageLedger ledger;
    private final Map<IStorageDevice, Boolean> devices = new IdentityHashMap<>();
    private final List<IStorageDevice> order = new ArrayList<>();
    private final LinkedHashMap<String, IItemPort> ports = new LinkedHashMap<>();

    private StorageNet(StorageLedger ledger) {
        this.ledger = ledger;
    }

    public static StorageNet of(IComputerAccess computer, MinecraftServer server) {
        StorageNet net = new StorageNet(StorageLedger.get(server));
        Builder b = net.new Builder();
        for (Map.Entry<String, IPeripheral> e : computer.getAttachedPeripherals().entrySet()) {
            if (e.getValue() instanceof Node node) node.contribute(b, e.getKey());
        }
        return net;
    }

    /** A net of just the given devices (encoders, tests). */
    public static StorageNet ofDevices(StorageLedger ledger, List<? extends IStorageDevice> devices) {
        StorageNet net = new StorageNet(ledger);
        Builder b = net.new Builder();
        for (IStorageDevice d : devices) b.device(d);
        return net;
    }

    public List<IStorageDevice> devices() {
        return Collections.unmodifiableList(order);
    }

    public Map<String, IItemPort> ports() {
        return Collections.unmodifiableMap(ports);
    }

    /** Mounted cells, highest device priority first. */
    public List<MountedCell> cells() {
        List<MountedCell> out = new ArrayList<>();
        List<IStorageDevice> sorted = new ArrayList<>(order);
        sorted.sort(Comparator.comparingInt(IStorageDevice::priority).reversed());
        for (IStorageDevice d : sorted) out.addAll(d.cells());
        return out;
    }

    /** A cell by full id or a unique prefix of at least 8 characters. */
    public MountedCell cell(@Nullable String id) throws StorageException {
        if (id == null || id.isEmpty()) throw new StorageException("no cell given");
        String want = id.toLowerCase(java.util.Locale.ROOT);
        MountedCell found = null;
        for (MountedCell c : cells()) {
            String s = c.id().toString();
            if (s.equals(want)) return c;
            if (want.length() >= 8 && s.startsWith(want)) {
                if (found != null) throw new StorageException("cell id '" + id + "' is ambiguous");
                found = c;
            }
        }
        if (found == null) throw new StorageException("no reachable cell '" + id + "'");
        return found;
    }

    public CellContents contents(MountedCell cell) throws StorageException {
        CellContents c = ledger.get(cell.id());
        if (c == null) throw new StorageException("cell " + cell.shortId() + " has no record");
        return c;
    }

    public boolean hasDevice(UUID deviceId) {
        for (IStorageDevice d : order) if (d.deviceId().equals(deviceId)) return true;
        return false;
    }

    /** The named decoder, or the first one when {@code name} is null. */
    public DecoderBlockEntity decoder(@Nullable String name) throws StorageException {
        if (name != null) {
            IItemPort p = ports.get(name);
            if (p instanceof DecoderBlockEntity d) return d;
            for (IItemPort q : ports.values()) {
                if (q instanceof DecoderBlockEntity d && d.deviceId().toString().startsWith(name)) return d;
            }
            throw new StorageException("no reachable decoder '" + name + "'");
        }
        for (IItemPort p : ports.values()) if (p instanceof DecoderBlockEntity d) return d;
        throw new StorageException("no decoder reachable (place an Item Decoder next to the computer or on its wired bus)");
    }

    /** Name of a port in this net. */
    @Nullable
    public String nameOf(IItemPort port) {
        for (var e : ports.entrySet()) if (e.getValue() == port) return e.getKey();
        return null;
    }
}
//?}
