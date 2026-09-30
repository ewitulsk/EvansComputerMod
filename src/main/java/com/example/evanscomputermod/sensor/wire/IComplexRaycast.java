/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.utility.IComplexRaycast).
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
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

/** An entity picked by the player's crosshair with its own shape test instead of its bounding box. */
public interface IComplexRaycast {
    /** Closest hit between {@code min} and {@code max}, or null. Coordinates are the entity's own (plot space on a Sable structure). */
    @Nullable
    Vec3 raycast(Vec3 min, Vec3 max);

    /** Bounding box in the entity's own coordinates. */
    AABB getDeSabledBB();
}
//?}
