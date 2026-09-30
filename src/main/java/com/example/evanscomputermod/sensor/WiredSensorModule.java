package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.sensor.wire.BaseWireEntity;
import com.example.evanscomputermod.sensor.wire.BlockWireEndpoint;
import com.example.evanscomputermod.sensor.wire.IWireEndpoint;
import com.example.evanscomputermod.sensor.wire.JunctionWireEndpoint;
import com.example.evanscomputermod.sensor.wire.WireConnections;
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.StringTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * Wired Sensor Module: finds the sensors wired to its bay connector (through
 * any number of junctions, up to {@link #MAX_SENSORS}) and lets programs
 * configure and read them. Sensors are named {@code lidar_1}, {@code lidar_2},
 * ... in the order they were first seen (or by an anvil name on the sensor),
 * and keep their names, settings and running state across moves, reloads and
 * taking the module out.
 */
public final class WiredSensorModule extends AnnotatedPeripheral implements IComputerModule {
    public static final String TYPE = "wired_sensors";
    public static final int MAX_SENSORS = 8;
    private static final int REFRESH_TICKS = 20;
    private static final int MAX_WIRES_WALKED = 256;
    private static final int NAME_MAX = 32;

    private final IModuleHost host;
    private final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();

    /** Mount key -> name, for sensors without an anvil name. */
    private final Map<String, String> names = new TreeMap<>();
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

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
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
    }

    // ------------------------------------------------------------ wires -> sensors

    /** Lidar sensors reachable over wires from this module's bay connector, nearest first. */
    List<BlockPos> findSensors(ServerLevel level) {
        List<BlockPos> found = new ArrayList<>();
        Set<IWireEndpoint> seen = new HashSet<>();
        Set<BaseWireEntity> walked = new HashSet<>();
        ArrayDeque<IWireEndpoint> queue = new ArrayDeque<>();
        IWireEndpoint start = new BlockWireEndpoint(host.getPos(), host.getSlot());
        queue.add(start);
        seen.add(start);
        while(!queue.isEmpty() && walked.size() < MAX_WIRES_WALKED) {
            IWireEndpoint at = queue.poll();
            for(BaseWireEntity wire : WireConnections.get(level, at)) {
                if(!walked.add(wire))
                    continue;
                IWireEndpoint other = at.equals(wire.getEndpoint1()) ? wire.getEndpoint2() : wire.getEndpoint1();
                if(other == null || !seen.add(other))
                    continue;
                if(other instanceof JunctionWireEndpoint) {
                    queue.add(other);
                } else if(other instanceof BlockWireEndpoint b && !b.getPos().equals(host.getPos())
                        && level.getBlockState(b.getPos()).getBlock() instanceof LidarSensorBlock) {
                    found.add(b.getPos());
                }
            }
        }
        return found;
    }

    private void refresh(ServerLevel level) {
        List<BlockPos> found = findSensors(level);
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
