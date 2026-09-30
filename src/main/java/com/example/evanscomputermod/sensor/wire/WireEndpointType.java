/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.WireEndpointType); endpoints are
 * saved with a string id instead of the enum ordinal, and only the block-wire
 * endpoint types are kept.
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
import net.minecraft.nbt.CompoundTag;
import org.jetbrains.annotations.Contract;

import java.util.Locale;
import java.util.function.Supplier;

public enum WireEndpointType {
    BLOCK(BlockWireEndpoint::new, true),
    JUNCTION(JunctionWireEndpoint::new, true),
    BLOCK_WIRE(BlockWireEntityEndpoint::new, false),
    IMAGINARY(ImaginaryWireEndpoint::new, false),
    DEFERRED_JUNCTION(DeferredJunctionWireEndpoint::new, true);

    private final Supplier<IWireEndpoint> factory;
    // This is only used by the block wire placement code.
    private final boolean connectable;

    WireEndpointType(Supplier<IWireEndpoint> factory, boolean connectable) {
        this.factory = factory;
        this.connectable = connectable;
    }

    public boolean isConnectable() {
        return connectable;
    }

    public String id() {
        return name().toLowerCase(Locale.ROOT);
    }

    public CompoundTag serialize(IWireEndpoint endpoint) {
        var tag = new CompoundTag();
        tag.putString("Type", id());
        endpoint.write(tag);
        return tag;
    }

    @Contract("null -> null")
    public static IWireEndpoint deserialize(CompoundTag tag) {
        if(tag == null || !tag.contains("Type"))
            return null;
        String id = tag.getString("Type");
        for(var type : values()) {
            if(type.id().equals(id)) {
                var endpoint = type.factory.get();
                endpoint.read(tag);
                return endpoint;
            }
        }
        return null;
    }
}
//?}
