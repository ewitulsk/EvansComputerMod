/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.BlockWireEntity); removed the
 * overheating particles and entity touch damage, registry lookups point at
 * this mod's entity type and items.
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
import com.example.evanscomputermod.sensor.SensorContent;
import net.minecraft.core.Direction;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;

/**
 * A wire lying along block surfaces: axis-aligned segments on a 1/16-block
 * grid starting at the entity's position.
 */
public class BlockWireEntity extends BaseWireEntity implements IComplexRaycast {
    public AABB mainBoundingBox;
    public final List<AABB> boundingBoxes = new ArrayList<>();
    public final List<Point> segments = new ArrayList<>();

    private float totalLength = 0;

    public BlockWireEntity(EntityType<?> type, Level world) {
        super(type, world);
    }

    public static BlockWireEntity create(Level world, IWireEndpoint endpoint1, ItemStack item, List<Point> segments) {
        var entity = new BlockWireEntity(SensorContent.BLOCK_WIRE.get(), world);
        entity.setItem(item.getItem(), item.getCount());

        var pos = BlockTrace.alignPosition(endpoint1.getExactPosition(world));
        entity.setPosRaw(pos.x, pos.y, pos.z);
        entity.segments.addAll(segments);
        entity.bakeBoundingBoxes();

        entity.setEndpoint1(endpoint1);

        entity.setYRot(0);
        entity.setXRot(0);
        entity.setOldPosAndRot();
        entity.reapplyPosition();
        return entity;
    }

    public static BlockWireEntity create(Level world, Vec3 pos, ItemStack item, List<Point> segments) {
        var entity = new BlockWireEntity(SensorContent.BLOCK_WIRE.get(), world);
        entity.setItem(item.getItem(), item.getCount());

        entity.setPosRaw(pos.x, pos.y, pos.z);
        entity.segments.addAll(segments);
        entity.bakeBoundingBoxes();

        entity.setYRot(0);
        entity.setXRot(0);
        entity.setOldPosAndRot();
        entity.reapplyPosition();
        return entity;
    }

    private void optimizeSegments() {
        if(segments.size() <= 1)
            return;
        int i = 1;
        while(i < segments.size()) {
            var prev = segments.get(i - 1);
            var segment = segments.get(i);
            if(prev.direction == segment.direction) {
                segments.set(i - 1, new Point(prev.direction, prev.gridLength + segment.gridLength));
                segments.remove(i);
            } else {
                ++i;
            }
        }
    }

    @Override
    protected AABB makeBoundingBox() {
        if(mainBoundingBox != null) {
            return mainBoundingBox.move(position());
        } else {
            return super.makeBoundingBox();
        }
    }

    public void bakeBoundingBoxes() {
        optimizeSegments();
        boundingBoxes.clear();
        totalLength = 0;

        // Starting from zero will make the bounding boxes independent of entity position,
        // but they will need to be offset before using them.
        var currentPos = Vec3.ZERO;
        double minX = 0, minY = 0, minZ = 0;
        double maxX = 0, maxY = 0, maxZ = 0;
        for(var segment : segments) {
            segment.start = currentPos.add(position());
            var nextPos = currentPos.add(segment.vector());
            maxX = Math.max(maxX, nextPos.x);
            maxY = Math.max(maxY, nextPos.y);
            maxZ = Math.max(maxZ, nextPos.z);
            minX = Math.min(minX, nextPos.x);
            minY = Math.min(minY, nextPos.y);
            minZ = Math.min(minZ, nextPos.z);

            boundingBoxes.add(new AABB(currentPos, nextPos).inflate(0.0625f));
            currentPos = nextPos;
            totalLength += segment.length();
        }

        mainBoundingBox = new AABB(minX, minY, minZ, maxX, maxY, maxZ).inflate(0.0625f);
        setBoundingBox(makeBoundingBox());
    }

    @Override
    public void setPos(double x, double y, double z) {
        super.setPos(x, y, z);
        // Segment starts are absolute; keep them with the entity.
        if(mainBoundingBox != null)
            bakeBoundingBoxes();
    }

    @Override
    public AABB getDeSabledBB() {
        if(mainBoundingBox == null)
            return getBoundingBox();
        return mainBoundingBox.move(position());
    }

    public float getTotalLength() {
        return totalLength;
    }

    @Override
    public @Nullable ItemEntity spawnAtLocation(ItemStack stack, float yOffset) {
        if(stack.isEmpty() || this.level().isClientSide || mainBoundingBox == null)
            return null;
        var center = mainBoundingBox.getCenter();
        ItemEntity itemEntity = new ItemEntity(this.level(),
                this.getX() + center.x,
                this.getY() + (double) yOffset + center.y,
                this.getZ() + center.z,
                stack);
        itemEntity.setDefaultPickUpDelay();
        this.level().addFreshEntity(itemEntity);
        return itemEntity;
    }

    @Override
    protected void readAdditionalSaveData(CompoundTag nbt) {
        super.readAdditionalSaveData(nbt);

        segments.clear();
        var segmentList = nbt.getList("Segments", Tag.TAG_COMPOUND);
        for(var segment : segmentList) {
            segments.add(new Point((CompoundTag) segment));
        }
        bakeBoundingBoxes();
    }

    @Override
    protected void addAdditionalSaveData(CompoundTag nbt) {
        super.addAdditionalSaveData(nbt);

        var segmentList = new ListTag();
        for(var segment : segments) {
            segmentList.add(segment.serialize());
        }
        nbt.put("Segments", segmentList);
    }

    @Override
    public InteractionResult interact(Player player, InteractionHand hand) {
        if(hand != InteractionHand.MAIN_HAND)
            return InteractionResult.PASS;
        var stack = player.getItemInHand(hand);
        if(stack.getItem() instanceof SensorWireItem) {
            // Connect wire to wire.
            if(player.level().isClientSide) {
                return com.example.evanscomputermod.sensor.client.ClientWireInteractions.attachWire(this);
            } else {
                return InteractionResult.CONSUME;
            }
        } else if(stack.is(SensorContent.WIRE_CUTTERS)) {
            if(!player.isShiftKeyDown()) {
                if(player.level().isClientSide) {
                    // Cut a segment of the wire.
                    return com.example.evanscomputermod.sensor.client.ClientWireInteractions.segmentCut(this);
                } else {
                    return InteractionResult.CONSUME;
                }
            }
        }
        return super.interact(player, hand);
    }

    @Override
    public @Nullable Vec3 raycast(Vec3 min, Vec3 max) {
        Vec3 closestHit = null;
        var localPos = position();
        min = min.subtract(localPos);
        max = max.subtract(localPos);
        double distance = max.distanceToSqr(min);
        for(var bb : boundingBoxes) {
            var hit = bb.clip(min, max);
            if(hit.isEmpty())
                continue;
            var hitDistance = hit.get().distanceToSqr(min);
            if(hitDistance < distance) {
                distance = hitDistance;
                closestHit = hit.get().add(localPos);
            }
        }
        return closestHit;
    }

    @Override
    public void endpointRemoved(IWireEndpoint endpoint) {
        if(segments.isEmpty()) {
            if(endpoint.equals(getEndpoint1()))
                setEndpoint1(null);
            if(endpoint.equals(getEndpoint2()))
                setEndpoint2(null);
            return;
        }
        Point removedSegment;
        if(endpoint.equals(getEndpoint2())) {
            removedSegment = segments.remove(segments.size() - 1);
            setEndpoint2(null);
        } else if(endpoint.equals(getEndpoint1())) {
            removedSegment = segments.remove(0);
            setPos(position().add(removedSegment.vector()));
            setEndpoint1(null);
        } else {
            return;
        }
        int items = (int) removedSegment.length();
        if(items > 0 && !level().isClientSide) {
            var start = removedSegment.start;
            var vector = removedSegment.vector();
            if(getWireCount() <= items)
                items = items - 1;
            if(items > 0) {
                ItemEntity itemEntity = new ItemEntity(this.level(),
                        start.x + vector.x, start.y + vector.y, start.z + vector.z,
                        new ItemStack(getItem(), items));
                itemEntity.setDefaultPickUpDelay();
                this.level().addFreshEntity(itemEntity);
                incrementWireCount(-items);
            }
        }
        if(segments.isEmpty()) {
            kill();
            return;
        }
        bakeBoundingBoxes();
        sendExtraData();
    }

    public BlockWireEntity flip() {
        var entity = new BlockWireEntity(SensorContent.BLOCK_WIRE.get(), level());
        entity.setItem(getItem(), getWireCount());
        entity.setColor(getColor());

        var pos = position();
        for(var segment : segments) {
            pos = pos.add(segment.vector());
            var dir = segment.direction.getOpposite();
            var length = segment.gridLength;
            if(length > 0)
                entity.segments.add(0, new Point(dir, length));
        }

        entity.setPosRaw(pos.x, pos.y, pos.z);
        entity.bakeBoundingBoxes();
        var end1 = getEndpoint2();
        var end2 = getEndpoint1();

        entity.setYRot(0);
        entity.setXRot(0);
        entity.setOldPosAndRot();
        entity.reapplyPosition();

        // Attach the copy before dropping this one, so a junction never sees its holder count dip.
        entity.setEndpoint1(end1);
        entity.setEndpoint2(end2);
        this.discard();
        ((ServerLevel) level()).tryAddFreshEntityWithPassengers(entity);
        return entity;
    }

    public void extend(List<Point> points, int newItems, boolean notify) {
        if(level().isClientSide)
            return;
        incrementWireCount(newItems);
        this.segments.addAll(points);
        bakeBoundingBoxes();

        if(notify)
            sendExtraData();
    }

    public void extend(List<Point> points, int newItems) {
        extend(points, newItems, true);
    }

    public JunctionWireEndpoint split(int segmentIndex, int segmentPoint) {
        var world = level();
        if(world.isClientSide)
            return null;

        var segment = segments.get(segmentIndex);

        var junctionPos = segment.start.relative(segment.direction, segmentPoint / 16f);
        var junction = new JunctionWireEndpoint(junctionPos);

        var wire2 = new BlockWireEntity(SensorContent.BLOCK_WIRE.get(), world);
        wire2.setColor(getColor());
        wire2.setItem(getItem(), 0);
        var splitSegment = new Point(segment.direction, segment.gridLength - segmentPoint);
        wire2.segments.add(splitSegment);
        this.segments.set(segmentIndex, new Point(segment.direction, Math.max(segmentPoint - 1, 0)));
        float movedLength = splitSegment.length();

        int removeCount = segments.size() - segmentIndex - 1;
        for(int i = 0; i < removeCount; ++i) {
            var removed = segments.remove(segmentIndex + 1);
            wire2.segments.add(removed);
            movedLength += removed.length();
        }

        wire2.setPosRaw(junctionPos.x, junctionPos.y, junctionPos.z);
        wire2.bakeBoundingBoxes();
        this.bakeBoundingBoxes();

        wire2.setYRot(0);
        wire2.setXRot(0);
        wire2.setOldPosAndRot();
        wire2.reapplyPosition();

        int items = Math.min((int) movedLength, getWireCount());
        wire2.incrementWireCount(items);
        incrementWireCount(-items);

        wire2.setEndpoint2(getEndpoint2());
        wire2.setEndpoint1(junction);
        this.setEndpoint2(junction);
        ((ServerLevel) world).tryAddFreshEntityWithPassengers(wire2);

        sendExtraData();
        return junction;
    }

    @Override
    public void sublevelRotate(Rotation rotation) {
        for(int i = 0; i < segments.size(); ++i) {
            var point = segments.get(i);
            segments.set(i, new Point(rotation.rotate(point.direction), point.gridLength));
        }
        bakeBoundingBoxes();
        sendExtraData();
    }

    public static class Point {
        public Vec3 start;
        public final Direction direction;
        public final int gridLength;

        public Point(Direction direction, int gridLength) {
            this.direction = direction;
            this.gridLength = gridLength;
        }

        public Point(CompoundTag tag) {
            this.direction = Direction.from3DDataValue(tag.getInt("Direction"));
            this.gridLength = tag.getInt("Length");
        }

        public CompoundTag serialize() {
            var tag = new CompoundTag();
            tag.putInt("Direction", direction.get3DDataValue());
            tag.putInt("Length", gridLength);
            return tag;
        }

        public float length() {
            return gridLength / 16f;
        }

        public Vec3 vector() {
            var length = length();
            return switch(direction) {
                case EAST -> new Vec3(length, 0, 0);
                case WEST -> new Vec3(-length, 0, 0);
                case UP -> new Vec3(0, length, 0);
                case DOWN -> new Vec3(0, -length, 0);
                case SOUTH -> new Vec3(0, 0, length);
                case NORTH -> new Vec3(0, 0, -length);
            };
        }

        public static Point x(float length) {
            return new Point(length >= 0 ? Direction.EAST : Direction.WEST, (int) (Math.abs(length) * 16));
        }

        public static Point y(float length) {
            return new Point(length >= 0 ? Direction.UP : Direction.DOWN, (int) (Math.abs(length) * 16));
        }

        public static Point z(float length) {
            return new Point(length >= 0 ? Direction.SOUTH : Direction.NORTH, (int) (Math.abs(length) * 16));
        }
    }
}
//?}
