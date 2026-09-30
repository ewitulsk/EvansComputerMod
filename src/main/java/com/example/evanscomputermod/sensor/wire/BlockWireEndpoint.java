/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.BlockWireEndpoint); connection
 * points come from IWireHost and attached wires are indexed in
 * WireConnections instead of an ElectricBehaviour.
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
import net.minecraft.core.SectionPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

import java.util.Objects;

/** A connection point ({@code terminal}) on the block at {@code pos}. */
public class BlockWireEndpoint implements IWireEndpoint {
    private BlockPos pos;
    private int terminal;

    public BlockWireEndpoint() {
        this(null, 0);
    }

    public BlockWireEndpoint(BlockPos pos, int terminal) {
        this.pos = pos;
        this.terminal = terminal;
    }

    @Override
    public WireEndpointType type() {
        return WireEndpointType.BLOCK;
    }

    public BlockPos getPos() {
        return pos;
    }

    public int getTerminal() {
        return terminal;
    }

    @Override
    public void read(CompoundTag nbt) {
        var posArr = nbt.getIntArray("Pos");
        pos = new BlockPos(posArr[0], posArr[1], posArr[2]);
        terminal = nbt.getInt("Terminal");
    }

    @Override
    public void write(CompoundTag nbt) {
        nbt.putIntArray("Pos", new int[] { pos.getX(), pos.getY(), pos.getZ() });
        nbt.putInt("Terminal", terminal);
    }

    private boolean isLoaded(Level world) {
        return world.hasChunk(SectionPos.blockToSectionCoord(pos.getX()), SectionPos.blockToSectionCoord(pos.getZ()));
    }

    @Nullable
    public IWireHost getHost(Level world) {
        if(!isLoaded(world))
            return null;
        return IWireHost.getAt(world, pos);
    }

    @Override
    @NotNull
    public Vec3 getExactPosition(Level world) {
        return IWireHost.getTerminalPos(world, pos, this.terminal);
    }

    @Override
    public boolean isValid(Level world) {
        var host = getHost(world);
        return host != null && host.terminal(world.getBlockState(pos), terminal) != null;
    }

    /** The chunk is loaded but the connection point is gone (block broken, module removed). */
    public boolean isGone(Level world) {
        return isLoaded(world) && !isValid(world);
    }

    @Override
    public <T extends BaseWireEntity> boolean canAcceptType(Class<T> clazz) {
        return BaseWireEntity.class.isAssignableFrom(clazz);
    }

    @Override
    public void assignWireEntity(BaseWireEntity entity) {
        WireConnections.add(entity.level(), this, entity);
    }

    @Override
    public void removeWireEntity(BaseWireEntity entity) {
        WireConnections.remove(entity.level(), this, entity);
    }

    @Override
    public boolean equals(Object obj) {
        if(obj == this) return true;
        if(obj instanceof BlockWireEndpoint other) {
            return pos.equals(other.pos) && terminal == other.terminal;
        }
        return false;
    }

    @Override
    public int hashCode() {
        return Objects.hash(pos, terminal);
    }

    @Override
    public String toString() {
        return String.format("Block(pos=%s, n=%d)", pos, terminal);
    }

    @Nullable
    public WireTerminal getTerminalPlacement(Level world) {
        var host = getHost(world);
        if(host == null)
            return null;
        return host.terminal(world.getBlockState(pos), terminal);
    }

    @Override
    public IWireEndpoint makeOffset(BlockPos offset) {
        return new BlockWireEndpoint(pos.offset(offset), terminal);
    }

    @Override
    public MoveAction shouldMove(Level level, Iterable<BlockPos> allBlocks) {
        for(var block : allBlocks) {
            if(block.equals(pos))
                return MoveAction.MOVE;
        }
        return MoveAction.STAY;
    }
}
//?}
