package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.sensor.wire.BlockWireEndpoint;
import com.example.evanscomputermod.sensor.wire.WireBus;
import com.example.evanscomputermod.storage.api.StorageApiPeripheral;
import com.example.evanscomputermod.storage.api.StorageNet;
import com.example.evanscomputermod.storage.device.DecoderBlockEntity;
import com.example.evanscomputermod.storage.device.DriveBlockEntity;
import com.example.evanscomputermod.storage.device.EncoderBlockEntity;
import com.example.evanscomputermod.storage.device.StorageDeviceBlock;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.StringTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

/**
 * Wired Bus Module (item id {@code wired_sensor_module}, peripheral type
 * {@code wired_sensors} for compatibility): finds the sensors and storage
 * devices (Drives, Item Encoders and Decoders) wired to its bay connector.
 * Storage devices join the computer's storage net, so the storage API methods
 * inherited from {@link StorageApiPeripheral} see them.
 *
 * <p>Sensors are found through
 * any number of junctions, up to {@link #MAX_SENSORS}) and lets programs
 * configure and read them. Sensors are named {@code lidar_1}, {@code lidar_2},
 * ... in the order they were first seen (or by an anvil name on the sensor),
 * and keep their names, settings and running state across moves, reloads and
 * taking the module out.
 */
public final class WiredSensorModule extends StorageApiPeripheral implements IComputerModule {
    public static final String TYPE = "wired_sensors";
    public static final int MAX_SENSORS = 8;
    private static final int REFRESH_TICKS = 20;
    /** Most storage devices one module reaches. */
    public static final int MAX_STORAGE_DEVICES = 32;
    private static final int NAME_MAX = 32;

    private final IModuleHost host;

    /** Mount key -> name, for sensors without an anvil name. */
    private final Map<String, String> names = new TreeMap<>();
    /** Storage device offset key -> name ({@code drive_1}, {@code decoder_1}, ...). */
    private final Map<String, String> storageNames = new TreeMap<>();
    /** Wired storage devices by name. Server thread. */
    private final Map<String, BlockPos> storage = new LinkedHashMap<>();
    private final Map<String, LidarConfig> configs = new HashMap<>();
    private final Set<String> running = new HashSet<>();

    /** Server thread only. */
    private final Map<String, Sensor> sensors = new LinkedHashMap<>();
    /** Read by program threads (mainThread = false methods). */
    private volatile Map<String, View> views = Map.of();

    private boolean live;
    private int ticks;
    private boolean refreshNow;
    private int rotate;

    /** A sensor found on the wires. */
    private static final class Sensor {
        final String name;
        SensorMount mount;
        LidarScanner.Job job;
        /** Keep scanning until {@link #seq} reaches this (single scans asked for with scan()). */
        int wantSeq;
        int seq;
        @Nullable
        LidarScanner.Scan latest;

        Sensor(String name, SensorMount mount) {
            this.name = name;
            this.mount = mount;
        }
    }

    private record View(SensorMount mount, @Nullable LidarScanner.Scan latest, boolean running) {
    }

    public WiredSensorModule(IModuleHost host, CompoundTag savedState) {
        this.host = host;
        load(savedState);
    }

    @Override
    public String getType() {
        return TYPE;
    }

    // ------------------------------------------------------------ lifecycle (server thread)

    @Override
    public void onLoad() {
        live = true;
        sensors.clear();
        refreshNow = true;
        publish();
    }

    @Override
    public void onUnload() {
        live = false;
        sensors.clear();
        storage.clear();
        publish();
    }

    @Override
    public void tick() {
        if(!live || !host.isAlive())
            return;
        ServerLevel level = host.getLevel();
        if(refreshNow || ++ticks >= REFRESH_TICKS) {
            ticks = 0;
            refreshNow = false;
            refresh(level);
        }
        scan(level);
        StorageLedger ledger = StorageLedger.get(level.getServer());
        if(ledger.anyChangedLastTick() && !storage.isEmpty()) {
            List<String> changed = new ArrayList<>();
            for(BlockPos p : storage.values()) {
                if(level.getBlockEntity(p) instanceof DriveBlockEntity d)
                    changed.addAll(d.mounts().changedIds(ledger));
            }
            postChanged(changed);
        }
    }

    // ------------------------------------------------------------ wires -> sensors

    /** Lidar sensors reachable over wires from this module's bay connector, nearest first. */
    List<BlockPos> findSensors(ServerLevel level) {
        return find(level, WireBus.walk(level, new BlockWireEndpoint(host.getPos(), host.getSlot())), false);
    }

    private List<BlockPos> find(ServerLevel level, List<BlockWireEndpoint> reached, boolean storageDevices) {
        List<BlockPos> found = new ArrayList<>();
        for(BlockWireEndpoint b : reached) {
            if(b.getPos().equals(host.getPos()) || found.contains(b.getPos()))
                continue;
            var block = level.getBlockState(b.getPos()).getBlock();
            if(storageDevices ? block instanceof StorageDeviceBlock : block instanceof LidarSensorBlock)
                found.add(b.getPos());
        }
        return found;
    }

    private void refresh(ServerLevel level) {
        List<BlockWireEndpoint> reached = WireBus.walk(level, new BlockWireEndpoint(host.getPos(), host.getSlot()));
        refreshStorage(level, find(level, reached, true));
        List<BlockPos> found = find(level, reached, false);
        Map<String, SensorMount> now = new LinkedHashMap<>();
        List<SensorMount> mounts = new ArrayList<>();
        for(BlockPos pos : found) {
            BlockState state = level.getBlockState(pos);
            mounts.add(SensorMount.of(host.getPos(), host.getFacing(), pos, state));
        }
        mounts.sort((a, b) -> a.key().compareTo(b.key()));
        for(SensorMount mount : mounts) {
            if(now.size() >= MAX_SENSORS)
                break;
            String name = nameFor(level, mount, now.keySet());
            now.put(name, mount);
        }

        boolean changed = false;
        for(var it = sensors.entrySet().iterator(); it.hasNext(); ) {
            var e = it.next();
            if(!now.containsKey(e.getKey())) {
                it.remove();
                changed = true;
                event("sensor_detach", e.getKey());
            }
        }
        for(var e : now.entrySet()) {
            Sensor s = sensors.get(e.getKey());
            if(s == null) {
                sensors.put(e.getKey(), new Sensor(e.getKey(), e.getValue()));
                changed = true;
                event("sensor_attach", e.getKey(), "lidar");
            } else if(!s.mount.equals(e.getValue())) {
                s.mount = e.getValue();
                s.job = null;     // restart a scan that was taken from the old position
                changed = true;
            }
        }
        if(changed)
            publish();
    }

    private String nameFor(ServerLevel level, SensorMount mount, Set<String> taken) {
        String name = null;
        if(level.getBlockEntity(mount.sensorPos()) instanceof LidarSensorBlockEntity be)
            name = be.sensorName();
        if(name == null) {
            name = names.get(mount.key());
            if(name == null) {
                Set<String> used = new HashSet<>(names.values());
                int n = 1;
                while(used.contains("lidar_" + n))
                    n++;
                name = "lidar_" + n;
                names.put(mount.key(), name);
                host.markDirty();
            }
        }
        String unique = name;
        for(int i = 2; taken.contains(unique); i++)
            unique = name + "_" + i;
        return unique;
    }

    // ------------------------------------------------------------ wires -> storage devices

    private void refreshStorage(ServerLevel level, List<BlockPos> found) {
        Map<String, BlockPos> now = new LinkedHashMap<>();
        List<BlockPos> sorted = new ArrayList<>(found);
        sorted.sort(java.util.Comparator.comparing(this::offsetKey));
        for(BlockPos pos : sorted) {
            if(now.size() >= MAX_STORAGE_DEVICES)
                break;
            var be = level.getBlockEntity(pos);
            String kind = be instanceof DriveBlockEntity ? "drive" : be instanceof EncoderBlockEntity ? "encoder"
                    : be instanceof DecoderBlockEntity ? "decoder" : null;
            if(kind == null)
                continue;
            String key = offsetKey(pos) + "," + kind;
            String name = storageNames.get(key);
            if(name == null) {
                Set<String> used = new HashSet<>(storageNames.values());
                int n = 1;
                while(used.contains(kind + "_" + n))
                    n++;
                name = kind + "_" + n;
                storageNames.put(key, name);
                host.markDirty();
            }
            now.put(name, pos);
        }
        if(!now.equals(storage)) {
            for(String gone : storage.keySet())
                if(!now.containsKey(gone))
                    event("storage_detach", gone);
            for(String name : now.keySet())
                if(!storage.containsKey(name))
                    event("storage_attach", name);
            storage.clear();
            storage.putAll(now);
        }
    }

    /** A wired block's offset from the computer in the computer's frame (stable across Sable moves). */
    private String offsetKey(BlockPos pos) {
        BlockPos d = pos.subtract(host.getPos());
        var f = host.getFacing();
        var l = f.getCounterClockWise();
        int fwd = d.getX() * f.getStepX() + d.getZ() * f.getStepZ();
        int left = d.getX() * l.getStepX() + d.getZ() * l.getStepZ();
        return fwd + "," + left + "," + d.getY();
    }

    @Override
    @Nullable
    protected MinecraftServer server() {
        ServerLevel level = host.getLevel();
        return level == null ? null : level.getServer();
    }

    @Override
    public void contribute(StorageNet.Builder net, String attachment) {
        ServerLevel level = host.getLevel();
        if(level == null || !live)
            return;
        for(var e : storage.entrySet()) {
            if(!level.isLoaded(e.getValue()))
                continue;
            var be = level.getBlockEntity(e.getValue());
            if(be instanceof DriveBlockEntity d)
                net.device(d);
            else if(be instanceof EncoderBlockEntity enc)
                net.port(e.getKey(), enc);
            else if(be instanceof DecoderBlockEntity dec)
                net.port(e.getKey(), dec);
        }
    }

    @PeripheralMethod(description = "Storage devices on the wires: {name: kind}")
    public Map<String, Object> storageDevices() {
        Map<String, Object> out = new LinkedHashMap<>();
        ServerLevel level = host.getLevel();
        for(var e : storage.entrySet()) {
            var be = level == null ? null : level.getBlockEntity(e.getValue());
            out.put(e.getKey(), be instanceof DriveBlockEntity ? "drive" : be instanceof EncoderBlockEntity ? "encoder"
                    : be instanceof DecoderBlockEntity ? "decoder" : "unknown");
        }
        return out;
    }

    // ------------------------------------------------------------ scanning (server thread)

    private void scan(ServerLevel level) {
        if(sensors.isEmpty())
            return;
        List<Sensor> order = new ArrayList<>(sensors.values());
        rotate = (rotate + 1) % order.size();
        boolean published = false;
        for(int k = 0; k < order.size(); k++) {
            Sensor s = order.get((k + rotate) % order.size());
            boolean wants = s.seq < s.wantSeq || running.contains(s.name);
            if(!wants && s.job == null)
                continue;
            if(level.getBlockEntity(s.mount.sensorPos()) instanceof LidarSensorBlockEntity be)
                be.touch(level.getGameTime());
            if(s.job == null)
                s.job = new LidarScanner.Job(config(s.name), s.mount, level.getGameTime());
            int granted = LidarScanner.claim(level.getServer(), s.job.ranges.length - s.job.next);
            LidarScanner.advance(level, s.job, granted);
            if(s.job.done()) {
                s.seq++;
                s.latest = new LidarScanner.Scan(s.seq, s.job.config, s.job.mount, s.job.startTick, level.getGameTime(), s.job.ranges);
                s.job = null;
                published = true;
                event("lidar_scan", s.name, s.seq);
            }
        }
        if(published)
            publish();
    }

    private LidarConfig config(String name) {
        return configs.getOrDefault(name, LidarConfig.DEFAULT);
    }

    private void publish() {
        Map<String, View> v = new LinkedHashMap<>();
        // Programs see sensors in name order.
        for(Sensor s : sensors.values().stream().sorted(java.util.Comparator.comparing(x -> x.name)).toList()) {
            v.put(s.name, new View(s.mount, s.latest, running.contains(s.name)));
        }
        views = java.util.Collections.unmodifiableMap(v);
    }

    private void event(String name, Object... args) {
        for(IComputerAccess computer : computers)
            computer.queueEvent(name, args);
    }

    // ------------------------------------------------------------ persistence

    private void load(CompoundTag tag) {
        CompoundTag n = tag.getCompound("names");
        for(String key : n.getAllKeys())
            names.put(key, n.getString(key));
        CompoundTag c = tag.getCompound("configs");
        for(String key : c.getAllKeys())
            configs.put(key, LidarConfig.load(c.getCompound(key)));
        ListTag r = tag.getList("running", Tag.TAG_STRING);
        for(int i = 0; i < r.size(); i++)
            running.add(r.getString(i));
        CompoundTag sn = tag.getCompound("storage_names");
        for(String key : sn.getAllKeys())
            storageNames.put(key, sn.getString(key));
    }

    @Override
    public void saveState(CompoundTag tag) {
        CompoundTag n = new CompoundTag();
        names.forEach(n::putString);
        tag.put("names", n);
        CompoundTag c = new CompoundTag();
        configs.forEach((k, v) -> c.put(k, v.save()));
        tag.put("configs", c);
        ListTag r = new ListTag();
        running.stream().sorted().forEach(s -> r.add(StringTag.valueOf(s)));
        tag.put("running", r);
        if(!storageNames.isEmpty()) {
            CompoundTag sn = new CompoundTag();
            storageNames.forEach(sn::putString);
            tag.put("storage_names", sn);
        }
    }

    // ------------------------------------------------------------ program API

    private View view(String name) throws PeripheralException {
        View v = views.get(name);
        if(v == null)
            throw new PeripheralException("no sensor named '" + name + "' (see list())");
        return v;
    }

    private Sensor sensor(String name) throws PeripheralException {
        Sensor s = sensors.get(name);
        if(s == null)
            throw new PeripheralException("no sensor named '" + name + "' (see list())");
        return s;
    }

    @PeripheralMethod(description = "Sensors on the wires: [{name, type, mount, running}]", mainThread = false)
    public List<Object> list() {
        List<Object> out = new ArrayList<>();
        views.forEach((name, v) -> {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", name);
            m.put("type", "lidar");
            m.put("mount", v.mount.toMap());
            m.put("running", v.running);
            out.add(m);
        });
        return out;
    }

    @PeripheralMethod(description = "Names of the sensors on the wires", mainThread = false)
    public List<String> names() {
        return new ArrayList<>(views.keySet());
    }

    @PeripheralMethod(description = "Where a sensor sits relative to the computer: {x, y, z, yaw, mount, block, facing}", mainThread = false)
    public Map<String, Object> mount(String name) throws PeripheralException {
        return view(name).mount.toMap();
    }

    @PeripheralMethod(description = "Maximum number of sensors on one module", mainThread = false)
    public int maxSensors() {
        return MAX_SENSORS;
    }

    @PeripheralMethod(description = "Maximum rays in one scan (az_steps * rows)", mainThread = false)
    public int maxRays() {
        return LidarConfig.MAX_RAYS;
    }

    @PeripheralMethod(description = "Change a lidar's scan pattern: {az_min, az_max, az_steps, el_min, el_max, rows, range}; returns the full setting")
    public Map<String, Object> configure(String name, Map<?, ?> settings) throws PeripheralException {
        Sensor s = sensor(name);
        LidarConfig c = config(name).with(settings);
        configs.put(name, c);
        s.job = null;
        host.markDirty();
        return c.toMap();
    }

    @PeripheralMethod(description = "A lidar's scan pattern")
    public Map<String, Object> getConfig(String name) throws PeripheralException {
        sensor(name);
        return config(name).toMap();
    }

    @PeripheralMethod(description = "Take one scan; returns the seq it will have (a lidar_scan event follows)")
    public int scan(String name) throws PeripheralException {
        Sensor s = sensor(name);
        // A scan already under way started before this call, so the answer is the one after it.
        int target = s.seq + (s.job != null ? 2 : 1);
        s.wantSeq = Math.max(s.wantSeq, target);
        return target;
    }

    @PeripheralMethod(description = "Scan continuously (kept across reloads and moves)")
    public void start(String name) throws PeripheralException {
        sensor(name);
        if(running.add(name)) {
            host.markDirty();
            publish();
        }
    }

    @PeripheralMethod(description = "Stop scanning continuously")
    public void stop(String name) throws PeripheralException {
        sensor(name);
        if(running.remove(name)) {
            host.markDirty();
            publish();
        }
    }

    @PeripheralMethod(description = "Latest scan: {seq, start_tick, end_tick, rows, columns, config, hits, ranges} "
            + "with ranges as little-endian float32 bytes (row-major, +inf = no hit), or nil", mainThread = false)
    @Nullable
    public Map<String, Object> getScan(String name) throws PeripheralException {
        var latest = view(name).latest;
        return latest == null ? null : latest.toMap(true, false);
    }

    @PeripheralMethod(description = "Latest scan's hits as points in the computer frame (x forward, y left, z up): "
            + "{seq, ..., hits, points} with points as little-endian float32 x,y,z triples, or nil", mainThread = false)
    @Nullable
    public Map<String, Object> getPoints(String name) throws PeripheralException {
        var latest = view(name).latest;
        return latest == null ? null : latest.toMap(false, true);
    }

    @PeripheralMethod(description = "Sequence number of the latest finished scan (0 = none yet)", mainThread = false)
    public int getSeq(String name) throws PeripheralException {
        var latest = view(name).latest;
        return latest == null ? 0 : latest.seq();
    }

    @PeripheralMethod(description = "Rename a sensor (the new name sticks to its mounting spot)")
    public void rename(String name, String newName) throws PeripheralException {
        Sensor s = sensor(name);
        String n = newName == null ? "" : newName.trim();
        if(n.isEmpty() || n.length() > NAME_MAX)
            throw new PeripheralException("name must be 1-" + NAME_MAX + " characters");
        if(sensors.containsKey(n) || names.containsValue(n))
            throw new PeripheralException("name '" + n + "' is taken");
        names.put(s.mount.key(), n);
        LidarConfig c = configs.remove(name);
        if(c != null)
            configs.put(n, c);
        if(running.remove(name))
            running.add(n);
        host.markDirty();
        refreshNow = true;
    }
}
//?}
