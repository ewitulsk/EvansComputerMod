package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.sensor.wire.BlockWireEndpoint;
import com.example.evanscomputermod.sensor.wire.WireBus;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;

import java.util.ArrayList;
import java.util.List;

/**
 * Storage devices a block can reach on its own, without a computer: drives
 * and computers' bay Storage Modules touching it, plus those on its Sensor
 * Wire bus. Used by the Encoder so hopper intake works with the computer off.
 */
public final class LocalReach {

    private LocalReach() {
    }

    public static List<IStorageDevice> find(ServerLevel level, BlockPos pos) {
        List<IStorageDevice> out = new ArrayList<>();
        for (Direction d : Direction.values()) addAt(level, pos.relative(d), out);
        for (BlockWireEndpoint e : WireBus.walk(level, new BlockWireEndpoint(pos, 0))) {
            if (!e.getPos().equals(pos)) addAt(level, e.getPos(), out);
        }
        return out;
    }

    private static void addAt(ServerLevel level, BlockPos p, List<IStorageDevice> out) {
        if (!level.isLoaded(p)) return;
        var be = level.getBlockEntity(p);
        if (be instanceof DriveBlockEntity drive) {
            if (!out.contains(drive)) out.add(drive);
        } else if (be instanceof TerminalBlockEntity te) {
            ModuleBays bays = te.getModuleBays();
            for (int i = 0; i < ModuleBays.SLOTS; i++) {
                if (bays.getModule(i) instanceof StorageModule m && !out.contains(m)) out.add(m);
            }
        }
    }
}
//?}
