package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.radio.api.Pose;

/**
 * The one place a modem module works out where its antenna is: the module
 * host's pose (a block computer, possibly on a Sable ship, or a held ePhone),
 * raised a little to the antenna. Called every tick: mobile hosts move.
 */
final class ModemPose {
    private ModemPose() {}

    static Pose of(IModuleHost host) {
        if (host.getLevel() == null) return null;
        Pose p = host.getPose();
        if (p == null) return null;
        return new Pose(p.dimension(), p.x(), p.y() + 0.4, p.z(), p.qx(), p.qy(), p.qz(), p.qw());
    }
}
//?}
