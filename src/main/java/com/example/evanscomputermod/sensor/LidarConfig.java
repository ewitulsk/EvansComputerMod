package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import net.minecraft.nbt.CompoundTag;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * One lidar scan pattern: {@code azSteps} columns from {@code azMin} to
 * {@code azMax} (degrees around the sensor's up axis, 0 = forward, positive =
 * left / counter-clockwise from above) times {@code rows} rows from
 * {@code elMin} to {@code elMax} (degrees, positive = up), out to {@code range}
 * blocks. A full 360 degree sweep does not repeat its first column.
 */
public record LidarConfig(double azMin, double azMax, int azSteps, double elMin, double elMax, int rows, double range) {
    public static final double MAX_RANGE = 64;
    public static final int MAX_RAYS = 8192;
    public static final LidarConfig DEFAULT = new LidarConfig(-180, 180, 360, 0, 0, 1, 32);

    public int rays() {
        return azSteps * rows;
    }

    private boolean fullCircle() {
        return azMax - azMin >= 360 - 1e-9;
    }

    /** Azimuth of column {@code i}, degrees. */
    public double azimuth(int i) {
        if(fullCircle())
            return azMin + i * (azMax - azMin) / azSteps;
        return azSteps == 1 ? azMin : azMin + i * (azMax - azMin) / (azSteps - 1);
    }

    /** Elevation of row {@code r}, degrees. */
    public double elevation(int r) {
        return rows == 1 ? elMin : elMin + r * (elMax - elMin) / (rows - 1);
    }

    /**
     * Unit direction of ray {@code index} (row-major: {@code index = row * azSteps + column})
     * in the sensor frame, as (forward, left, up).
     */
    public double[] direction(int index) {
        double az = Math.toRadians(azimuth(index % azSteps));
        double el = Math.toRadians(elevation(index / azSteps));
        double c = Math.cos(el);
        return new double[] {c * Math.cos(az), c * Math.sin(az), Math.sin(el)};
    }

    public LidarConfig validate() throws PeripheralException {
        if(azSteps < 1 || rows < 1)
            throw new PeripheralException("az_steps and rows must be at least 1");
        if((long) azSteps * rows > MAX_RAYS)
            throw new PeripheralException("too many rays: az_steps * rows must be at most " + MAX_RAYS);
        if(!(range > 0) || range > MAX_RANGE)
            throw new PeripheralException("range must be in (0, " + (int) MAX_RANGE + "]");
        if(!(azMax > azMin) && azSteps > 1)
            throw new PeripheralException("az_max must be greater than az_min");
        if(azMax - azMin > 360 + 1e-9)
            throw new PeripheralException("azimuth span must be at most 360 degrees");
        if(elMin < -90 || elMax > 90 || elMax < elMin)
            throw new PeripheralException("elevations must satisfy -90 <= el_min <= el_max <= 90");
        return this;
    }

    /** {@code this} with any keys of {@code map} applied (az_min, az_max, az_steps, el_min, el_max, rows, range). */
    public LidarConfig with(Map<?, ?> map) throws PeripheralException {
        double azMin = num(map, "az_min", this.azMin);
        double azMax = num(map, "az_max", this.azMax);
        int azSteps = (int) num(map, "az_steps", this.azSteps);
        double elMin = num(map, "el_min", this.elMin);
        double elMax = num(map, "el_max", this.elMax);
        int rows = (int) num(map, "rows", this.rows);
        double range = num(map, "range", this.range);
        for(Object key : map.keySet()) {
            if(!java.util.List.of("az_min", "az_max", "az_steps", "el_min", "el_max", "rows", "range").contains(String.valueOf(key)))
                throw new PeripheralException("unknown lidar setting '" + key + "'");
        }
        return new LidarConfig(azMin, azMax, azSteps, elMin, elMax, rows, range).validate();
    }

    private static double num(Map<?, ?> map, String key, double fallback) throws PeripheralException {
        Object v = map.get(key);
        if(v == null)
            return fallback;
        if(v instanceof Number n)
            return n.doubleValue();
        throw new PeripheralException(key + " must be a number");
    }

    public Map<String, Object> toMap() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("az_min", azMin);
        m.put("az_max", azMax);
        m.put("az_steps", azSteps);
        m.put("el_min", elMin);
        m.put("el_max", elMax);
        m.put("rows", rows);
        m.put("range", range);
        return m;
    }

    public CompoundTag save() {
        CompoundTag t = new CompoundTag();
        t.putDouble("az_min", azMin);
        t.putDouble("az_max", azMax);
        t.putInt("az_steps", azSteps);
        t.putDouble("el_min", elMin);
        t.putDouble("el_max", elMax);
        t.putInt("rows", rows);
        t.putDouble("range", range);
        return t;
    }

    public static LidarConfig load(CompoundTag t) {
        try {
            return new LidarConfig(t.getDouble("az_min"), t.getDouble("az_max"), t.getInt("az_steps"),
                    t.getDouble("el_min"), t.getDouble("el_max"), t.getInt("rows"), t.getDouble("range")).validate();
        } catch(PeripheralException e) {
            return DEFAULT;
        }
    }
}
//?}
