/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.mixin.client.ComplexEntityRaycastMixin); only
 * this mod's wires are tested and Sable is optional.
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
package com.example.evanscomputermod.sensor.mixin;

//? if <=1.21.1 {
import com.example.evanscomputermod.sensor.SensorSable;
import com.example.evanscomputermod.sensor.wire.IComplexRaycast;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.projectile.ProjectileUtil;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.Optional;
import java.util.function.Predicate;

/**
 * The crosshair picks sensor wires by their thin segments instead of their
 * (large) bounding box. The hit location is in the wire's own coordinates.
 */
@Mixin(ProjectileUtil.class)
public abstract class WirePickMixin {
    @Unique
    @Nullable
    private static Vec3 evanscomputermod$complexRaycast(Entity entity, Vec3 min, Vec3 max, double distance) {
        IComplexRaycast checker = (IComplexRaycast) entity;
        AABB entityBB = checker.getDeSabledBB().inflate(entity.getPickRadius());
        Optional<Vec3> potentialHit = entityBB.clip(min, max);
        if(entityBB.contains(min)) {
            return checker.raycast(min, max);
        } else if(potentialHit.isPresent() && min.distanceToSqr(potentialHit.get()) < distance) {
            return checker.raycast(min, max);
        }
        return null;
    }

    @Inject(
            method = "getEntityHitResult(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/Vec3;Lnet/minecraft/world/phys/Vec3;Lnet/minecraft/world/phys/AABB;Ljava/util/function/Predicate;D)Lnet/minecraft/world/phys/EntityHitResult;",
            at = @At("RETURN"),
            cancellable = true
    )
    private static void evanscomputermod$pickWires(Entity shooter, Vec3 startVec, Vec3 endVec, AABB boundingBox,
                                                   Predicate<Entity> filter, double distance,
                                                   CallbackInfoReturnable<EntityHitResult> cir) {
        EntityHitResult baseResult = cir.getReturnValue();
        var world = shooter.level();
        if(!world.isClientSide())
            return;
        double currentHitDistance = distance;
        Entity currentHitEntity = null;
        Vec3 currentHitPoint = null;

        if(baseResult != null) {
            currentHitPoint = baseResult.getLocation();
            currentHitDistance = startVec.distanceToSqr(currentHitPoint);
        }

        for(var potentialHitEntity : ((net.minecraft.client.multiplayer.ClientLevel) world).entitiesForRendering()) {
            if(!(potentialHitEntity instanceof IComplexRaycast complex) || potentialHitEntity.isRemoved())
                continue;
            Vec3 startVec1 = SensorSable.toEntityLocal(potentialHitEntity, startVec);
            Vec3 endVec1 = SensorSable.toEntityLocal(potentialHitEntity, endVec);
            // Perform a cheap bounding box distance check first before going for the complex cast.
            var bb = complex.getDeSabledBB();
            var cX = Mth.clamp(startVec1.x, bb.minX, bb.maxX);
            var cY = Mth.clamp(startVec1.y, bb.minY, bb.maxY);
            var cZ = Mth.clamp(startVec1.z, bb.minZ, bb.maxZ);
            if(startVec1.distanceToSqr(cX, cY, cZ) >= currentHitDistance)
                continue;
            Vec3 hit = evanscomputermod$complexRaycast(potentialHitEntity, startVec1, endVec1, currentHitDistance);
            if(hit != null) {
                double hitSquaredDistance = startVec1.distanceToSqr(hit);
                if(hitSquaredDistance < currentHitDistance) {
                    currentHitEntity = potentialHitEntity;
                    currentHitPoint = hit;
                    currentHitDistance = hitSquaredDistance;
                }
            }
        }

        if(currentHitEntity != null) {
            cir.setReturnValue(new EntityHitResult(currentHitEntity, currentHitPoint));
        }
    }
}
//?}
