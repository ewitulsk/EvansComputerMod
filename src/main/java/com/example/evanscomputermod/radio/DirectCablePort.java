package com.example.evanscomputermod.radio;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.InterfaceBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.Set;

/**
 * A cable-port device (Access Point, Microwave Radio) touching a computer directly: a
 * Terminal's network face or an Interface Block face against the device joins the device's
 * port and that NIC on one segment, as a cable between them would. Uses the cable manager's
 * logical links (named {@code direct-<device mac>}), so it carries carrier and frames like a
 * cable segment. Server thread.
 */
public final class DirectCablePort {
    private static final int MAX_INTERFACE_BLOCKS = 64;

    private final byte[] deviceMac;
    private final String name;
    private byte[] linkedNic;
    private CableNetworkManager linkedIn;

    public DirectCablePort(byte[] deviceMac) {
        this.deviceMac = deviceMac.clone();
        this.name = "direct-" + HexFormat.of().formatHex(deviceMac);
    }

    /** Re-check what touches the device at {@code pos} and (re)link or unlink. Cheap; call on neighbour changes. */
    public void update(ServerLevel level, BlockPos pos) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        byte[] nic = nicFacing(level, pos);
        if (mgr == linkedIn && Arrays.equals(nic, linkedNic)) return;
        remove();
        if (mgr != null && nic != null) {
            mgr.logicalLink(name, deviceMac, nic, true);
            linkedIn = mgr;
            linkedNic = nic;
        }
    }

    /** Drop the link (device removed). */
    public void remove() {
        if (linkedIn != null && linkedIn == CableNetworkManager.getInstance()) linkedIn.removeLogicalLink(name);
        linkedIn = null;
        linkedNic = null;
    }

    /** The MAC of the computer NIC directly linked to this device, or null. */
    public @Nullable byte[] linkedNic() {
        return linkedNic == null ? null : linkedNic.clone();
    }

    /**
     * The MAC of the computer network face whose exit is {@code pos}: a Terminal face touching
     * it, or a free face of an Interface Block (cluster) touching it. Null if none.
     */
    public static @Nullable byte[] nicFacing(ServerLevel level, BlockPos pos) {
        for (Direction d : Direction.values()) {
            BlockPos n = pos.relative(d);
            var block = level.getBlockState(n).getBlock();
            if (block instanceof TerminalBlock) {
                byte[] mac = nicOf(level, n, pos);
                if (mac != null) return mac;
            } else if (block instanceof InterfaceBlock) {
                // Walk the Interface Block cluster to the computer it hangs off.
                Set<BlockPos> seen = new HashSet<>();
                ArrayDeque<BlockPos> todo = new ArrayDeque<>();
                todo.add(n);
                seen.add(n);
                while (!todo.isEmpty() && seen.size() <= MAX_INTERFACE_BLOCKS) {
                    BlockPos c = todo.poll();
                    for (Direction e : Direction.values()) {
                        BlockPos q = c.relative(e);
                        if (q.equals(pos) || !seen.add(q)) continue;
                        var b = level.getBlockState(q).getBlock();
                        if (b instanceof InterfaceBlock) todo.add(q);
                        else if (b instanceof TerminalBlock) {
                            byte[] mac = nicOf(level, q, pos);
                            if (mac != null) return mac;
                        }
                    }
                }
            }
        }
        return null;
    }

    private static @Nullable byte[] nicOf(ServerLevel level, BlockPos terminal, BlockPos exit) {
        if (!(level.getBlockEntity(terminal) instanceof TerminalBlockEntity t)) return null;
        int i = t.findInterfaceIndexByExitPos(exit);
        return i < 0 || t.getComputerId() == null ? null : NetworkHub.deriveMac(t.getComputerId(), i);
    }
}
//?}
