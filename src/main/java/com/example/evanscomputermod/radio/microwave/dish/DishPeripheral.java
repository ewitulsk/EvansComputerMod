package com.example.evanscomputermod.radio.microwave.dish;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.radio.microwave.DishAim;
import com.example.evanscomputermod.radio.microwave.DishPattern;
import com.example.evanscomputermod.radio.microwave.MicrowaveLink;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * A dish as a peripheral (type {@code dish}): aim it by yaw/pitch in its
 * block's frame, at a world point, or let it {@code align()} on the strongest
 * signal on its radio's channel. Yaw follows Minecraft (0 = south, 90 = west,
 * 180 = north, -90 = east); pitch is elevation, positive up. All methods run
 * on the server thread.
 */
public final class DishPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "dish";

    private final DishBlockEntity dish;

    DishPeripheral(DishBlockEntity dish) {
        this.dish = dish;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @PeripheralMethod(description = "Aim the dish: yaw (0 south, 90 west, 180 north, -90 east) and pitch (up positive), degrees, in the block's frame")
    public void set_aim(double yaw, double pitch) throws PeripheralException {
        try {
            dish.setAim(yaw, pitch);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
    }

    @PeripheralMethod(description = "Turn the dish by relative yaw and pitch, degrees")
    public void nudge(double dyaw, double dpitch) {
        dish.nudge(dyaw, dpitch);
    }

    @PeripheralMethod(description = "Current aim: {yaw, pitch} in the block's frame and {world_yaw, world_pitch} after a ship's rotation")
    public Map<String, Object> get_aim() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("yaw", dish.yaw());
        m.put("pitch", dish.elevation());
        double[] world = DishAim.aimOf(dish.worldBoresight());
        m.put("world_yaw", world[0]);
        m.put("world_pitch", world[1]);
        return m;
    }

    @PeripheralMethod(description = "Aim at a world position (x, y, z); works on a moving ship")
    public Map<String, Object> aim_at(double x, double y, double z) throws PeripheralException {
        try {
            dish.aimAt(x, y, z);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        return get_aim();
    }

    @PeripheralMethod(description = "Scan around the current aim (span degrees, default 10) for the strongest signal on the radio's channel and point there: {found, rssi_dbm, yaw, pitch}")
    public Map<String, Object> align(Double spanDeg) throws PeripheralException {
        try {
            return dish.align(spanDeg == null ? 10 : spanDeg);
        } catch (IllegalArgumentException | IllegalStateException e) {
            throw new PeripheralException(e.getMessage());
        }
    }

    @PeripheralMethod(description = "Dish facts: {size, diameter_m, gain_dbi, beamwidth_deg, frequency_mhz, world position, radio}")
    public Map<String, Object> info() {
        Map<String, Object> m = new LinkedHashMap<>();
        DishSize s = dish.size();
        m.put("size", s.n + "x" + s.n);
        m.put("diameter_m", s.diameterM);
        MicrowaveLink r = dish.radio();
        DishPattern p = r == null ? null : r.dish();
        m.put("radio", r != null);
        if (p != null) {
            m.put("frequency_mhz", p.freqHz() / 1e6);
            m.put("gain_dbi", p.peakGainDbi());
            m.put("beamwidth_deg", p.beamwidthDeg());
        }
        var pose = dish.worldPose();
        if (pose != null) {
            m.put("x", pose.x());
            m.put("y", pose.y());
            m.put("z", pose.z());
        }
        return m;
    }
}
//?}
