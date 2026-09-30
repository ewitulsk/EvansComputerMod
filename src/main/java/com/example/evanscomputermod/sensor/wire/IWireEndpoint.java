/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.IWireEndpoint); removed the
 * electrical simulation hooks, Sable calls go through SensorSable.
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
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.NotNull;

/**
 * One end of a wire: a connection point on a block, the free end of another
 * wire, a junction in the middle of wires, or a bare point on a block face.
 */
public interface IWireEndpoint {
    WireEndpointType type();

    void read(CompoundTag nbt);

    void write(CompoundTag nbt);

    @NotNull
    Vec3 getExactPosition(Level world);

    default boolean isValid(Level world) {
        return true;
    }

    default <T extends BaseWireEntity> boolean canAcceptType(Class<T> clazz) {
        return false;
    }

    default void assignWireEntity(BaseWireEntity entity) {
    }

    default void removeWireEntity(BaseWireEntity entity) {
    }

    default void moveWireEntity(BaseWireEntity entity) {
        removeWireEntity(entity);
    }

    default CompoundTag serialize() {
        return type().serialize(this);
    }

    default IWireEndpoint makeOffset(BlockPos offset) {
        return null;
    }

    default IWireEndpoint makeOffset(BlockPos blockOffset, Vec3 offset) {
        return makeOffset(blockOffset);
    }

    /** What happens to this end when {@code allBlocks} are moved by Sable. */
    default MoveAction shouldMove(Level level, Iterable<BlockPos> allBlocks) {
        return MoveAction.BREAK;
    }

    enum MoveAction {
        MOVE,
        STAY,
        BREAK
    }
}
//?}
