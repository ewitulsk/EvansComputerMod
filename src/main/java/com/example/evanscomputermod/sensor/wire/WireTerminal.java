/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.base.TerminalBoundingBox); dropped the
 * terminal name/colour decoration and the X/Z rotations, which nothing here uses.
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
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * A wire connection point on a block: the box a player clicks (block-local,
 * given in pixels) and the point the wire starts from.
 */
public final class WireTerminal {
    private Vec3 min;
    private Vec3 max;
    private Vec3 origin;
    private final double expand;

    private WireTerminal(WireTerminal other) {
        this.expand = other.expand;
        this.min = other.min;
        this.max = other.max;
        this.origin = other.origin;
    }

    public WireTerminal(double x1, double y1, double z1, double x2, double y2, double z2) {
        this(x1, y1, z1, x2, y2, z2, 0.1);
    }

    public WireTerminal(double x1, double y1, double z1, double x2, double y2, double z2, double expand) {
        min = new Vec3((x1 - expand) / 16.0, (y1 - expand) / 16.0, (z1 - expand) / 16.0);
        max = new Vec3((x2 + expand) / 16.0, (y2 + expand) / 16.0, (z2 + expand) / 16.0);
        origin = new Vec3((x1 + x2) * 0.03125, (y1 + y2) * 0.03125, (z1 + z2) * 0.03125);
        this.expand = expand / 16.0;
    }

    public VoxelShape getShape() {
        return Shapes.box(min.x + expand, min.y + expand, min.z + expand,
                max.x - expand, max.y - expand, max.z - expand);
    }

    public WireTerminal rotateAroundY(Rotation rotation) {
        WireTerminal terminal = new WireTerminal(this);
        switch(rotation) {
            case NONE -> {
            }
            case CLOCKWISE_90 -> {
                terminal.min = new Vec3(1 - max.z, min.y, min.x);
                terminal.max = new Vec3(1 - min.z, max.y, max.x);
                terminal.origin = new Vec3(1 - origin.z, origin.y, origin.x);
            }
            case CLOCKWISE_180 -> {
                terminal.min = new Vec3(1 - max.x, min.y, 1 - max.z);
                terminal.max = new Vec3(1 - min.x, max.y, 1 - min.z);
                terminal.origin = new Vec3(1 - origin.x, origin.y, 1 - origin.z);
            }
            case COUNTERCLOCKWISE_90 -> {
                terminal.min = new Vec3(min.z, min.y, 1 - max.x);
                terminal.max = new Vec3(max.z, max.y, 1 - min.x);
                terminal.origin = new Vec3(origin.z, origin.y, 1 - origin.x);
            }
        }
        return terminal;
    }

    /** Upside down: mirrored in y (and in z, like a block model rotated x=180). */
    public WireTerminal flipUpsideDown() {
        WireTerminal terminal = new WireTerminal(this);
        terminal.min = new Vec3(min.x, 1 - max.y, 1 - max.z);
        terminal.max = new Vec3(max.x, 1 - min.y, 1 - min.z);
        terminal.origin = new Vec3(origin.x, 1 - origin.y, 1 - origin.z);
        return terminal;
    }

    public WireTerminal withOrigin(double x, double y, double z) {
        this.origin = new Vec3(x, y, z).scale(1.0 / 16.0);
        return this;
    }

    public boolean check(Vec3 position) {
        return position.x >= min.x && position.y >= min.y && position.z >= min.z &&
                position.x < max.x && position.y < max.y && position.z < max.z;
    }

    public boolean check(BlockPos blockPos, Vec3 position) {
        return check(position.subtract(blockPos.getX(), blockPos.getY(), blockPos.getZ()));
    }

    public Vec3 getOrigin() {
        return origin;
    }

    public AABB getOutline() {
        return new AABB(min, max);
    }

    public static Rotation rotationFromNorth(net.minecraft.core.Direction facing) {
        return switch(facing) {
            case EAST -> Rotation.CLOCKWISE_90;
            case SOUTH -> Rotation.CLOCKWISE_180;
            case WEST -> Rotation.COUNTERCLOCKWISE_90;
            default -> Rotation.NONE;
        };
    }
}
//?}
