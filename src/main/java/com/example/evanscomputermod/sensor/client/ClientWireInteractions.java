/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.ClientWireInteractions); cords
 * removed, catnip helpers replaced, packets are this mod's payloads.
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
package com.example.evanscomputermod.sensor.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.SensorSable;
import com.example.evanscomputermod.sensor.wire.BaseWireEntity;
import com.example.evanscomputermod.sensor.wire.BlockWireEntity;
import com.example.evanscomputermod.sensor.wire.WirePackets;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.core.particles.DustParticleOptions;
import net.minecraft.network.chat.Component;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.network.PacketDistributor;
import org.joml.Vector3f;
import org.jetbrains.annotations.Nullable;

public final class ClientWireInteractions {
    private static BlockWireEntity currentEntity = null;
    private static int firstSegmentIndex;
    private static int firstSegmentPoint;

    private static final RandomSource r = RandomSource.create();

    private ClientWireInteractions() {
    }

    public static void handleWireData(WirePackets.WireData packet) {
        var level = Minecraft.getInstance().level;
        if(level == null)
            return;
        if(level.getEntity(packet.entityId()) instanceof BaseWireEntity wire) {
            wire.setPos(packet.x(), packet.y(), packet.z());
            wire.setOldPosAndRot();
            wire.onEntityDataPacket(packet.data());
        }
    }

    /** Particles along the part of a wire selected for cutting. */
    public static void clientTick() {
        var mc = Minecraft.getInstance();
        var target = mc.hitResult;
        if(currentEntity != null && (currentEntity.isRemoved() || mc.player == null
                || !mc.player.getMainHandItem().is(SensorContent.WIRE_CUTTERS))) {
            currentEntity = null;
            return;
        }
        if(target == null || target.getType() != HitResult.Type.ENTITY)
            return;
        var entityHit = (EntityHitResult) target;
        if(currentEntity != entityHit.getEntity())
            return;

        var segment = getSegment(currentEntity, target.getLocation());
        if(segment != null) {
            int index1, index2, point1, point2;
            if(segment[0] < firstSegmentIndex) {
                index1 = segment[0];
                point1 = segment[1];
                index2 = firstSegmentIndex;
                point2 = firstSegmentPoint;
            } else {
                index1 = firstSegmentIndex;
                index2 = segment[0];
                if(index1 == index2 && segment[1] < firstSegmentPoint) {
                    point1 = segment[1];
                    point2 = firstSegmentPoint;
                } else {
                    point1 = firstSegmentPoint;
                    point2 = segment[1];
                }
            }
            index1 = Mth.clamp(index1, 0, currentEntity.segments.size() - 1);
            index2 = Mth.clamp(index2, 0, currentEntity.segments.size() - 1);
            for(int i = index1; i <= index2; ++i) {
                var wireSegment = currentEntity.segments.get(i);
                Vec3 start = wireSegment.start, end = wireSegment.start.add(wireSegment.vector());
                if(i == index1)
                    start = wireSegment.start.relative(wireSegment.direction, point1 / 16f);
                if(i == index2)
                    end = wireSegment.start.relative(wireSegment.direction, point2 / 16f);
                var pos = start.lerp(end, r.nextFloat()).offsetRandom(r, 1 / 16f);
                pos = SensorSable.toWorld(mc.level, pos);
                mc.level.addAlwaysVisibleParticle(new DustParticleOptions(new Vector3f(1.0f, 0.5f, 0.5f), 0.5f),
                        pos.x, pos.y, pos.z, 0, 0, 0);
            }
        }
    }

    /** (segment index, grid point) under {@code hitPos} (entity-local coordinates), or null. */
    @Nullable
    private static int[] getSegment(BlockWireEntity entity, Vec3 hitPos) {
        var localPos = hitPos.subtract(entity.position());
        var thickness = entity.thickness();
        // Bounding boxes haven't been baked.
        if(entity.segments.size() != entity.boundingBoxes.size())
            return null;
        for(int i = 0; i < entity.boundingBoxes.size(); ++i) {
            var bb = entity.boundingBoxes.get(i);
            // Test with slightly larger bounding boxes.
            if(bb.inflate(thickness * 0.2f).contains(localPos)) {
                var segment = entity.segments.get(i);
                int segmentPoint = switch(segment.direction.getAxis()) {
                    case X -> (int) Math.round(Math.abs(segment.start.x - hitPos.x) * 16);
                    case Y -> (int) Math.round(Math.abs(segment.start.y - hitPos.y) * 16);
                    case Z -> (int) Math.round(Math.abs(segment.start.z - hitPos.z) * 16);
                };
                segmentPoint = Mth.clamp(segmentPoint, 0, segment.gridLength);
                return new int[] {i, segmentPoint};
            }
        }
        return null;
    }

    public static InteractionResult segmentCut(BlockWireEntity entity) {
        var mc = Minecraft.getInstance();
        var target = mc.hitResult;
        if(target == null || target.getType() != HitResult.Type.ENTITY)
            return InteractionResult.FAIL;
        // The wire pick (WirePickMixin) reports hits in the wire's own coordinates.
        var hitPos = target.getLocation();

        if(currentEntity != entity) {
            // First cut.
            var segment = getSegment(entity, hitPos);
            if(segment != null) {
                firstSegmentIndex = segment[0];
                firstSegmentPoint = segment[1];
                currentEntity = entity;
                mc.player.displayClientMessage(Component.translatable("message.evanscomputermod.wire.cut_next")
                        .withStyle(ChatFormatting.GRAY), true);
            }
            return InteractionResult.CONSUME;
        } else {
            var secondSegment = getSegment(entity, hitPos);
            if(secondSegment != null) {
                PacketDistributor.sendToServer(WirePackets.Cut.ordered(entity, firstSegmentIndex, firstSegmentPoint,
                        secondSegment[0], secondSegment[1]));
                currentEntity = null;
                return InteractionResult.SUCCESS;
            }
        }
        return InteractionResult.FAIL;
    }

    /** Sneak-use with cutters: forget the first cut point. */
    public static void cutClear() {
        var mc = Minecraft.getInstance();
        if(currentEntity != null) {
            mc.player.displayClientMessage(Component.translatable("message.evanscomputermod.wire.cut_reset"), true);
            mc.player.swing(InteractionHand.MAIN_HAND);
            currentEntity = null;
        }
    }

    public static InteractionResult attachWire(BlockWireEntity entity) {
        var mc = Minecraft.getInstance();
        var target = mc.hitResult;
        if(target == null || target.getType() != HitResult.Type.ENTITY)
            return InteractionResult.FAIL;
        var stack = mc.player.getItemInHand(InteractionHand.MAIN_HAND);
        if(entity.getItem() != stack.getItem()) {
            mc.player.displayClientMessage(Component.translatable("message.evanscomputermod.wire.connection_incorrect_wire_type")
                    .withStyle(ChatFormatting.RED), true);
            return InteractionResult.FAIL;
        }

        var connection = stack.get(SensorContent.CONNECTION_DATA.get());
        var existingEndpoint = connection == null ? null : connection.endpoint();
        if(existingEndpoint != null && !SensorSable.sameSubLevel(mc.level, existingEndpoint.getExactPosition(mc.level), entity.position())) {
            mc.player.displayClientMessage(Component.translatable("message.evanscomputermod.wire.connection_failed")
                    .withStyle(ChatFormatting.RED), true);
            return InteractionResult.FAIL;
        }

        var segment = getSegment(entity, target.getLocation());
        if(segment == null)
            return InteractionResult.FAIL;

        PacketDistributor.sendToServer(new WirePackets.Attach(entity.getId(), segment[0], segment[1]));
        return InteractionResult.SUCCESS;
    }
}
//?}
