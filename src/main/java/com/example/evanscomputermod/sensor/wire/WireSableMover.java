/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: the wire-moving part of PowerGrid's
 * mixin.sable.SubLevelAssemblerMixin, without the electrical network: wires
 * are found through WireConnections and junctions instead of transmission
 * lines.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
package com.example.evanscomputermod.sensor.wire;

//? if <=1.21.1 {
import dev.ryanhcode.sable.api.SubLevelAssemblyHelper;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Rotation;

import java.util.ArrayDeque;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.Set;

/**
 * Sable is about to move {@code blocks} (assembling a structure, splitting
 * one, or putting one back into the world). Wires plugged into those blocks
 * go with them: their ends are re-pointed to the new positions and the
 * entities are moved (and turned, if the move rotates). A wire that would end
 * up with one end moved and the other left behind is cut and dropped.
 */
public final class WireSableMover {
    private WireSableMover() {
    }

    public static void beforeMoveBlocks(ServerLevel level, SubLevelAssemblyHelper.AssemblyTransform transform, Iterable<BlockPos> blocks) {
        Set<BlockPos> moved = new HashSet<>();
        for(BlockPos p : blocks)
            moved.add(p.immutable());

        // Every wire reachable (through junctions) from a moved wire host.
        Set<BaseWireEntity> wires = new LinkedHashSet<>();
        Set<IWireEndpoint> seen = new HashSet<>();
        ArrayDeque<IWireEndpoint> queue = new ArrayDeque<>();
        for(BlockPos pos : moved) {
            var host = IWireHost.getAt(level, pos);
            if(host == null)
                continue;
            for(int i = 0; i < host.terminalCount(); i++) {
                var e = new BlockWireEndpoint(pos, i);
                if(seen.add(e))
                    queue.add(e);
            }
        }
        while(!queue.isEmpty()) {
            var at = queue.poll();
            for(var wire : WireConnections.get(level, at)) {
                if(!wires.add(wire))
                    continue;
                for(var end : new IWireEndpoint[] {wire.getEndpoint1(), wire.getEndpoint2()}) {
                    if(end instanceof JunctionWireEndpoint && seen.add(end))
                        queue.add(end);
                }
            }
        }
        if(wires.isEmpty())
            return;

        for(var wire : wires) {
            var a = wire.getEndpoint1();
            var b = wire.getEndpoint2();
            var ma = action(level, a, moved);
            var mb = action(level, b, moved);
            boolean anyMove = ma == IWireEndpoint.MoveAction.MOVE || mb == IWireEndpoint.MoveAction.MOVE;
            boolean anyStay = ma == IWireEndpoint.MoveAction.STAY || mb == IWireEndpoint.MoveAction.STAY;
            if(!anyMove)
                continue;
            if(anyStay) {
                // Stretched between the structure and the world: cut it.
                wire.kill();
                continue;
            }
            var na = offset(level, a, transform);
            var nb = offset(level, b, transform);
            wire.sublevelMove(na, nb);
            if(transform.getRotation() != Rotation.NONE)
                wire.sublevelRotate(transform.getRotation());
            wire.setPos(transform.apply(wire.position()));
            wire.setOldPosAndRot();
        }
    }

    /** Block ends move or stay with their block; junctions and free ends follow the wire. */
    private static IWireEndpoint.MoveAction action(ServerLevel level, IWireEndpoint e, Set<BlockPos> moved) {
        if(e instanceof BlockWireEndpoint b)
            return moved.contains(b.getPos()) ? IWireEndpoint.MoveAction.MOVE : IWireEndpoint.MoveAction.STAY;
        return null;
    }

    private static IWireEndpoint offset(ServerLevel level, IWireEndpoint endpoint, SubLevelAssemblyHelper.AssemblyTransform transform) {
        if(endpoint == null)
            return null;
        // A block end follows its block (its connection point may sit just outside the block).
        if(endpoint instanceof BlockWireEndpoint b)
            return new BlockWireEndpoint(transform.apply(b.getPos()), b.getTerminal());
        var pos = endpoint.getExactPosition(level);
        var blockPos = BlockPos.containing(pos);
        return endpoint.makeOffset(transform.apply(blockPos).subtract(blockPos), transform.apply(pos).subtract(pos));
    }
}
//?}
