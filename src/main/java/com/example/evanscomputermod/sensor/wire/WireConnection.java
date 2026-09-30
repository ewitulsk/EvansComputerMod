/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.WireConnection); only the pending
 * endpoint is kept (no transformer windings).
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
import com.mojang.serialization.Codec;
import io.netty.buffer.ByteBuf;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import org.jetbrains.annotations.Nullable;

import java.util.Objects;

/** The first end of a wire being placed, stored on the held Sensor Wire stack. */
public final class WireConnection {
    public static final Codec<WireConnection> CODEC = CompoundTag.CODEC.xmap(WireConnection::new, WireConnection::tag);
    public static final StreamCodec<ByteBuf, WireConnection> STREAM_CODEC =
            ByteBufCodecs.COMPOUND_TAG.map(WireConnection::new, WireConnection::tag);
    public static final WireConnection EMPTY = new WireConnection(new CompoundTag());

    private final CompoundTag tag;

    public WireConnection(CompoundTag tag) {
        this.tag = tag;
    }

    public static WireConnection of(IWireEndpoint endpoint) {
        var tag = new CompoundTag();
        tag.put("Connection", endpoint.serialize());
        return new WireConnection(tag);
    }

    private CompoundTag tag() {
        return tag;
    }

    @Nullable
    public IWireEndpoint endpoint() {
        return WireEndpointType.deserialize(tag.getCompound("Connection"));
    }

    @Override
    public boolean equals(Object obj) {
        if(obj == this)
            return true;
        return obj instanceof WireConnection other && tag.equals(other.tag);
    }

    @Override
    public int hashCode() {
        return Objects.hash(tag);
    }
}
//?}
