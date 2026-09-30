/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: the wire packets of PowerGrid
 * (network.packets.EntityDataS2CPacket, BlockWireAttachC2SPacket,
 * BlockWireCutC2SPacket, AlternatePlacementStatusC2SPacket) rewritten as
 * NeoForge payloads.
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
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.ArrayList;

public final class WirePackets {
    private WirePackets() {
    }

    public static void register(RegisterPayloadHandlersEvent event) {
        var registrar = event.registrar("1");
        registrar.playToClient(WireData.TYPE, WireData.STREAM_CODEC,
                (p, ctx) -> com.example.evanscomputermod.sensor.client.ClientWireInteractions.handleWireData(p));
        registrar.playToServer(Attach.TYPE, Attach.STREAM_CODEC, WirePackets::handleAttach);
        registrar.playToServer(Cut.TYPE, Cut.STREAM_CODEC, WirePackets::handleCut);
        registrar.playToServer(Alternate.TYPE, Alternate.STREAM_CODEC,
                (p, ctx) -> SensorWireItem.setAlternate(ctx.player(), p.held()));
    }

    // ------------------------------------------------------------ server -> client

    /** A wire's full state (and position) changed. */
    public record WireData(int entityId, double x, double y, double z, CompoundTag data) implements CustomPacketPayload {
        public static final Type<WireData> TYPE = new Type<>(EvansComputerMod.id("wire_data"));
        public static final StreamCodec<FriendlyByteBuf, WireData> STREAM_CODEC = StreamCodec.of(
                (buf, p) -> {
                    buf.writeVarInt(p.entityId);
                    buf.writeDouble(p.x);
                    buf.writeDouble(p.y);
                    buf.writeDouble(p.z);
                    buf.writeNbt(p.data);
                },
                buf -> new WireData(buf.readVarInt(), buf.readDouble(), buf.readDouble(), buf.readDouble(), buf.readNbt()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    // ------------------------------------------------------------ client -> server

    /** Clicked an existing wire with Sensor Wire: extend it, or branch off it. */
    public record Attach(int entityId, int index, int gridPoint) implements CustomPacketPayload {
        public static final Type<Attach> TYPE = new Type<>(EvansComputerMod.id("wire_attach"));
        public static final StreamCodec<FriendlyByteBuf, Attach> STREAM_CODEC = StreamCodec.of(
                (buf, p) -> {
                    buf.writeVarInt(p.entityId);
                    buf.writeVarInt(p.index);
                    buf.writeVarInt(p.gridPoint);
                },
                buf -> new Attach(buf.readVarInt(), buf.readVarInt(), buf.readVarInt()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Cut the part of a wire between two points. */
    public record Cut(int entityId, int index1, int point1, int index2, int point2) implements CustomPacketPayload {
        public static final Type<Cut> TYPE = new Type<>(EvansComputerMod.id("wire_cut"));
        public static final StreamCodec<FriendlyByteBuf, Cut> STREAM_CODEC = StreamCodec.of(
                (buf, p) -> {
                    buf.writeVarInt(p.entityId);
                    buf.writeVarInt(p.index1);
                    buf.writeVarInt(p.point1);
                    buf.writeVarInt(p.index2);
                    buf.writeVarInt(p.point2);
                },
                buf -> new Cut(buf.readVarInt(), buf.readVarInt(), buf.readVarInt(), buf.readVarInt(), buf.readVarInt()));

        /** Ordered so that (index1, point1) comes first along the wire. */
        public static Cut ordered(BlockWireEntity entity, int index1, int point1, int index2, int point2) {
            if(index2 < index1 || (index1 == index2 && point2 < point1))
                return new Cut(entity.getId(), index2, point2, index1, point1);
            return new Cut(entity.getId(), index1, point1, index2, point2);
        }

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** The alternate-placement key went down or up. */
    public record Alternate(boolean held) implements CustomPacketPayload {
        public static final Type<Alternate> TYPE = new Type<>(EvansComputerMod.id("wire_alternate"));
        public static final StreamCodec<FriendlyByteBuf, Alternate> STREAM_CODEC = StreamCodec.of(
                (buf, p) -> buf.writeBoolean(p.held),
                buf -> new Alternate(buf.readBoolean()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    // ------------------------------------------------------------ handlers

    private static boolean inReach(ServerPlayer player, BlockWireEntity wire) {
        return player.canInteractWithEntity(wire.getDeSabledBB(), 4.0)
                || SensorSable.inPlot(player.level(), wire.position());
    }

    private static void handleAttach(Attach p, IPayloadContext ctx) {
        if(!(ctx.player() instanceof ServerPlayer player))
            return;
        var entity = player.serverLevel().getEntity(p.entityId);
        if(!(entity instanceof BlockWireEntity wire) || !inReach(player, wire))
            return;
        var stack = player.getItemInHand(InteractionHand.MAIN_HAND);
        if(!(stack.getItem() instanceof SensorWireItem))
            return;
        if(p.index < 0 || p.index >= wire.segments.size())
            return;
        var segment = wire.segments.get(p.index);
        if(p.gridPoint < 0 || p.gridPoint > segment.gridLength)
            return;

        var existingConnection = stack.get(SensorContent.CONNECTION_DATA.get());
        var existingEndpoint = existingConnection == null ? null : existingConnection.endpoint();
        if(existingEndpoint != null && !SensorSable.sameSubLevel(player.level(),
                existingEndpoint.getExactPosition(player.level()), wire.position()))
            return;

        IWireEndpoint endpoint;
        if(p.gridPoint <= 1 && p.index == 0) {
            // Extend wire at start.
            if(wire.getEndpoint1() == null) {
                wire = wire.flip();
                endpoint = new BlockWireEntityEndpoint(wire, true);
            } else {
                // Possibly a junction.
                endpoint = wire.getEndpoint1();
            }
        } else if(p.gridPoint >= segment.gridLength - 1 && p.index == wire.segments.size() - 1) {
            // Extend wire at end.
            if(wire.getEndpoint2() == null) {
                endpoint = new BlockWireEntityEndpoint(wire, true);
            } else {
                // Possibly a junction.
                endpoint = wire.getEndpoint2();
            }
        } else {
            // Junction.
            endpoint = new DeferredJunctionWireEndpoint(wire, p.index, p.gridPoint);
        }
        if(endpoint != null && existingEndpoint == null) {
            stack.set(SensorContent.CONNECTION_DATA.get(), WireConnection.of(endpoint));
        } else if(endpoint != null) {
            var result = SensorWireItem.connect(player.level(), stack, player, existingEndpoint, endpoint);
            if(result.getResult().consumesAction())
                stack.remove(SensorContent.CONNECTION_DATA.get());
        }
    }

    private static BlockWireEntity spawnWire2(BlockWireEntity wire1, Vec3 start, int wireCount, java.util.List<BlockWireEntity.Point> segments) {
        if(start == null || segments.isEmpty())
            return null;
        var entity = BlockWireEntity.create(wire1.level(), start, new ItemStack(wire1.getItem(), wireCount), segments);
        entity.setColor(wire1.getColor());
        ((ServerLevel) wire1.level()).addFreshEntityWithPassengers(entity);
        entity.setEndpoint2(wire1.getEndpoint2());
        wire1.setEndpoint2(null);
        return entity;
    }

    private static void handleCut(Cut p, IPayloadContext ctx) {
        if(!(ctx.player() instanceof ServerPlayer player))
            return;
        var entity = player.serverLevel().getEntity(p.entityId);
        if(!(entity instanceof BlockWireEntity wire) || !inReach(player, wire))
            return;
        if(!player.getMainHandItem().is(SensorContent.WIRE_CUTTERS))
            return;
        int index1 = p.index1, point1 = p.point1, index2 = p.index2, point2 = p.point2;
        if(index1 < 0 || index2 >= wire.segments.size() || index1 > index2)
            return;
        int wireCount = wire.getWireCount();
        int gridLength2 = 0;
        var secondSegments = new ArrayList<BlockWireEntity.Point>();
        Vec3 secondStart = null;
        for(int i = index2; i < wire.segments.size(); ++i) {
            var segment = wire.segments.get(i);
            if(i == index2) {
                secondStart = segment.start.relative(segment.direction, point2 / 16f);
                var len = segment.gridLength - point2;
                if(len > 0) {
                    secondSegments.add(new BlockWireEntity.Point(segment.direction, len));
                    gridLength2 += len;
                }
            } else {
                secondSegments.add(new BlockWireEntity.Point(segment.direction, segment.gridLength));
                gridLength2 += segment.gridLength;
            }
        }
        int wire2Count = (int) Math.ceil(gridLength2 / 16f * SensorWireItem.ITEMS_PER_METER);
        while(wire.segments.size() > index1 + 1) {
            // Remove all segments above index1
            wire.segments.remove(wire.segments.size() - 1);
        }
        var last = wire.segments.remove(wire.segments.size() - 1);
        wire.segments.add(new BlockWireEntity.Point(last.direction, Math.min(last.gridLength, point1)));
        int gridLength1 = 0;
        for(var segment : wire.segments) {
            gridLength1 += segment.gridLength;
        }
        int wire1Count = (int) Math.ceil(gridLength1 / 16f * SensorWireItem.ITEMS_PER_METER);
        if(wire1Count >= wire2Count) {
            // Wire1 is the largest
            if(gridLength1 < 3) {
                // Too short, discard, no wire2 since it's even shorter.
                wire.discard();
            } else {
                wire.setItem(wire.getItem(), Math.min(wireCount, wire1Count));
                wireCount -= wire1Count;
                if(wireCount <= 0 || gridLength2 < 3) {
                    // Wire2 is discarded - not enough items
                    wire.setEndpoint2(null);
                } else {
                    spawnWire2(wire, secondStart, Math.min(wire2Count, wireCount), secondSegments);
                    wireCount -= wire2Count;
                }
            }
        } else {
            // Wire2 is the largest
            if(gridLength2 < 3) {
                // Too short, discard wire2 but also wire1 since it's even shorter
                wire.discard();
            } else {
                spawnWire2(wire, secondStart, Math.min(wireCount, wire2Count), secondSegments);
                wireCount -= wire2Count;
                if(wireCount <= 0 || gridLength1 < 3) {
                    // Wire1 is discarded - not enough items
                    wire.discard();
                } else {
                    // Keep wire1
                    wire.setItem(wire.getItem(), Math.min(wire1Count, wireCount));
                    wireCount -= wire1Count;
                }
            }
        }
        // Give back what the cut freed.
        for(; wireCount > 0; wireCount -= 64) {
            player.getInventory().placeItemBackInInventory(new ItemStack(wire.getItem(), Math.min(wireCount, 64)));
        }
        if(!wire.isRemoved()) {
            wire.bakeBoundingBoxes();
            wire.sendExtraData();
        }
    }
}
//?}
