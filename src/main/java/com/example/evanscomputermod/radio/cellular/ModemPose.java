package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.radio.api.Pose;
import net.minecraft.world.phys.Vec3;

/**
 * The one place a modem module works out where its antenna is. Today a module
 * host is a block (the computer's position, projected through a Sable ship's
 * transform when it rides one); once {@code IModuleHost} exposes a radio
 * {@link Pose} (held ePhones, P0's "modules without a block"), this becomes
 * {@code return host.getPose();}. Called every tick: mobile hosts move.
 */
final class ModemPose {
    private ModemPose() {}

    static Pose of(IModuleHost host) {
        var level = host.getLevel();
        if (level == null || host.getPos() == null) return null;
        Vec3 p = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, Vec3.atCenterOf(host.getPos()).add(0, 0.4, 0));
        return Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }
}
//?}
