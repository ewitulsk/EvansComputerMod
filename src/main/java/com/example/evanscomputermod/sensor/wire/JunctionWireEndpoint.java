/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.JunctionWireEndpoint); the wires
 * meeting at a junction are tracked in WireConnections, and the electrical
 * node / state sync parts are removed.
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
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.util.Mth;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.level.Level;
import org.jetbrains.annotations.NotNull;

import java.util.ArrayList;
import java.util.ConcurrentModificationException;
import java.util.HashSet;
import java.util.Random;
import java.util.Set;
import java.util.UUID;

/** A T-junction where three or more block wires meet. */
public class JunctionWireEndpoint implements IWireEndpoint {
    private static final Random random = new Random();
    /** Junctions being merged away; further removals on them are ignored. */
    private static final Set<UUID> LOCKED = new HashSet<>();

    private UUID id;
    private Vec3 pos;

    public static UUID makeUuid(Vec3 pos, long id) {
        int x = Mth.floor(pos.x);
        short y = (short) Mth.floor(pos.y);
        int z = Mth.floor(pos.z);
        // Version 8 Custom UUID
        // xxxxxxxx-yyyy-8iii-8iii-iiiizzzzzzzz
        long msb = (((long) x << 32) & 0xFFFFFFFF00000000L)
                 | (((long) y << 16) & 0x00000000FFFF0000L)
                 |                                 0x8000L
                 | ((id >> 28)       & 0x0000000000000FFFL);
        long lsb = ((long) z   & 0x00000000FFFFFFFFL)
                 |               0x8000000000000000L
                 | ((id << 32) & 0x0FFFFFFF00000000L);
        return new UUID(msb, lsb);
    }

    public static UUID makeUuid(Vec3 pos) {
        return makeUuid(pos, random.nextLong());
    }

    public static long getId(UUID uuid) {
        long id = (uuid.getLeastSignificantBits() >> 32) & 0xFFF_FFFF;
        id |= (uuid.getMostSignificantBits() & 0xFFF) << 28;
        return id;
    }

    public JunctionWireEndpoint() {
        this(null, null);
    }

    public JunctionWireEndpoint(Vec3 pos) {
        this(pos, makeUuid(pos));
    }

    private JunctionWireEndpoint(Vec3 pos, UUID id) {
        this.pos = pos;
        this.id = id;
    }

    @Override
    public WireEndpointType type() {
        return WireEndpointType.JUNCTION;
    }

    @Override
    public void read(CompoundTag nbt) {
        pos = new Vec3(nbt.getDouble("X"), nbt.getDouble("Y"), nbt.getDouble("Z"));
        id = nbt.getUUID("Id");
    }

    @Override
    public void write(CompoundTag nbt) {
        nbt.putDouble("X", pos.x);
        nbt.putDouble("Y", pos.y);
        nbt.putDouble("Z", pos.z);
        nbt.putUUID("Id", id);
    }

    @Override
    @NotNull
    public Vec3 getExactPosition(Level world) {
        return pos;
    }

    @Override
    public <T extends BaseWireEntity> boolean canAcceptType(Class<T> clazz) {
        return BlockWireEntity.class.isAssignableFrom(clazz);
    }

    @Override
    public void assignWireEntity(BaseWireEntity entity) {
        if(!(entity instanceof BlockWireEntity))
            throw new IllegalArgumentException("Wire junction must receive block wire entities");
        WireConnections.add(entity.level(), this, entity);
    }

    @Override
    public void removeWireEntity(BaseWireEntity entity) {
        var level = entity.level();
        if(LOCKED.contains(id))
            return;
        WireConnections.remove(level, this, entity);
        var holders = WireConnections.get(level, this);
        if(holders.size() == 2) {
            // Two holders remaining, we can merge them.
            LOCKED.add(id);
            try {
                merge(level, holders);
            } finally {
                LOCKED.remove(id);
            }
        } else if(holders.size() == 1) {
            // One holder remaining, remove the junction from it.
            var holder = holders.get(0);
            if(this.equals(holder.getEndpoint1()))
                holder.setEndpoint1(null);
            if(this.equals(holder.getEndpoint2()))
                holder.setEndpoint2(null);
            holder.sendExtraData();
        }
    }

    private void merge(Level level, java.util.List<BaseWireEntity> holders) {
        var wire1 = (BlockWireEntity) holders.get(0);
        var wire2 = (BlockWireEntity) holders.get(1);
        if(wire1 == wire2)
            throw new ConcurrentModificationException();
        var wire1End = this.equals(wire1.getEndpoint2());
        var wire2End = this.equals(wire2.getEndpoint2());

        boolean flipped = false, targetFlipped = false;
        BlockWireEntity target, source;
        if(wire1End || !wire2End) {
            source = wire2;
            if(!wire1End) {
                // New entity must be made with flipped wire1
                target = wire1.flip();
                targetFlipped = true;
            } else {
                // We can append wire2 into wire1
                target = wire1;
            }
            if(wire2End)
                flipped = true;
        } else {
            // Append wire1 onto wire2
            source = wire1;
            target = wire2;
        }

        if(target.segments.isEmpty()) {
            if(source.segments.isEmpty()) {
                source.discard();
            } else {
                if(flipped) {
                    source.setEndpoint2(target.getEndpoint1());
                } else {
                    source.setEndpoint1(target.getEndpoint1());
                }
                source.sendExtraData();
            }
            target.discard();
        } else {
            int lastIndex = target.segments.size() - 1;
            var last = target.segments.get(lastIndex);
            if(!targetFlipped)
                target.segments.set(lastIndex, new BlockWireEntity.Point(last.direction, last.gridLength + 1));

            if(flipped) {
                var segments = new ArrayList<BlockWireEntity.Point>();
                for(var segment : source.segments) {
                    segments.add(0, new BlockWireEntity.Point(segment.direction.getOpposite(), segment.gridLength));
                }
                target.extend(segments, source.getWireCount());
                target.setEndpoint2(source.getEndpoint1());
            } else {
                target.extend(source.segments, source.getWireCount());
                target.setEndpoint2(source.getEndpoint2());
            }
            target.sendExtraData();
            source.discard();
        }
        // The junction is gone; drop everything still listed under it (including a flipped copy of wire1).
        WireConnections.clear(level, this);
    }

    @Override
    public void moveWireEntity(BaseWireEntity entity) {
        WireConnections.remove(entity.level(), this, entity);
    }

    @Override
    public IWireEndpoint makeOffset(BlockPos offset) {
        var newPos = pos.add(offset.getX(), offset.getY(), offset.getZ());
        return new JunctionWireEndpoint(newPos, makeUuid(newPos, getId(id)));
    }

    @Override
    public IWireEndpoint makeOffset(BlockPos blockOffset, Vec3 offset) {
        var newPos = pos.add(offset);
        return new JunctionWireEndpoint(newPos, makeUuid(newPos, getId(id)));
    }

    @Override
    public boolean equals(Object obj) {
        if(obj == this)
            return true;
        if(obj instanceof JunctionWireEndpoint other) {
            // Note: We don't compare position since it might be slightly different due to imprecision.
            return id.equals(other.id);
        }
        return false;
    }

    @Override
    public int hashCode() {
        return id.hashCode();
    }

    @Override
    public String toString() {
        return String.format("Junction(id=%s)", id);
    }
}
//?}
