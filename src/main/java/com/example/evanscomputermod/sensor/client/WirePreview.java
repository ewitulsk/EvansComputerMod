/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.WirePreview); block wires only,
 * rendered with the vanilla buffer source from a NeoForge render stage event.
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
import com.example.evanscomputermod.sensor.wire.BlockTrace;
import com.example.evanscomputermod.sensor.wire.BlockWireEntity;
import com.example.evanscomputermod.sensor.wire.BlockWireEntityEndpoint;
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.SensorWireItem;
import com.example.evanscomputermod.sensor.wire.WireTerminal;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.core.Direction;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import org.jetbrains.annotations.Nullable;

/** The translucent green (reachable) or red path shown while placing Sensor Wire. */
public final class WirePreview {
    private static boolean render;
    private static BlockTrace.Trace renderedTrace;
    private static Vec3 renderedPos1, renderedPos2;

    private WirePreview() {
    }

    @Nullable
    public static ItemStack getUsedWireStack(Player player) {
        var stack = player.getMainHandItem();
        if(stack.getItem() instanceof SensorWireItem && stack.has(SensorContent.CONNECTION_DATA.get()))
            return stack;
        return null;
    }

    public static void tick() {
        render = false;
        var mc = Minecraft.getInstance();
        var player = mc.player;
        if(player == null)
            return;
        ItemStack wireStack = getUsedWireStack(player);
        if(wireStack == null)
            return;
        var target = mc.hitResult;
        if(target == null)
            return;
        if(target.getType() != HitResult.Type.BLOCK) {
            if(!(target.getType() == HitResult.Type.ENTITY && ((EntityHitResult) target).getEntity() instanceof BlockWireEntity))
                return;
        }

        var connection = wireStack.get(SensorContent.CONNECTION_DATA.get());
        var endpoint = connection == null ? null : connection.endpoint();
        if(endpoint == null)
            return;

        var world = mc.level;
        var currentPos = endpoint.getExactPosition(world);
        Direction continueDir = null;
        if(endpoint instanceof BlockWireEntityEndpoint bwe) {
            var entity = bwe.getEntity(world);
            if(entity != null) {
                var segments = entity.segments;
                if(segments.isEmpty())
                    return;
                continueDir = bwe.getEnd() ? segments.get(segments.size() - 1).direction
                        : segments.get(0).direction.getOpposite();
            }
        }

        var hitPoint = target.getLocation();
        WireTerminal hitTerminal = null;
        if(target.getType() == HitResult.Type.BLOCK) {
            var blockTarget = (BlockHitResult) target;
            var pos = blockTarget.getBlockPos();
            var host = IWireHost.getAt(world, pos);
            var terminal = host == null ? null : host.terminal(world.getBlockState(pos),
                    host.terminalIndexAt(world.getBlockState(pos), hitPoint.subtract(pos.getX(), pos.getY(), pos.getZ())));
            if(terminal != null) {
                hitPoint = terminal.getOrigin().add(pos.getX(), pos.getY(), pos.getZ());
                hitTerminal = terminal;
            } else {
                hitPoint = hitPoint.relative(blockTarget.getDirection(), 1 / 32f);
            }
        }

        if(!SensorSable.sameSubLevel(world, currentPos, hitPoint))
            return;
        var projCurrentPos = SensorSable.toWorld(world, currentPos);
        // Stop rendering the preview far away to keep the game from freezing.
        if(projCurrentPos.distanceTo(SensorSable.toWorld(world, hitPoint)) > 64)
            return;

        currentPos = BlockTrace.alignPosition(currentPos);
        renderedPos1 = projCurrentPos;
        renderedPos2 = currentPos;
        renderedTrace = SensorWireItem.alternateWirePlacement(player)
                ? new BlockTrace.Trace(null, BlockTrace.alternatePath(currentPos, hitPoint))
                : BlockTrace.findPathWithState(world, currentPos, hitPoint, hitTerminal, continueDir);
        render = renderedTrace != null;
    }

    public static void render(RenderLevelStageEvent event) {
        if(event.getStage() != RenderLevelStageEvent.Stage.AFTER_TRANSLUCENT_BLOCKS || !render || renderedTrace == null)
            return;
        var points = renderedTrace.result();
        if(points == null)
            return;
        var mc = Minecraft.getInstance();
        var cameraPos = event.getCamera().getPosition();
        PoseStack matrixStack = event.getPoseStack();
        var buffer = mc.renderBuffers().bufferSource();
        matrixStack.pushPose();
        matrixStack.translate(renderedPos1.x - cameraPos.x, renderedPos1.y - cameraPos.y, renderedPos1.z - cameraPos.z);
        var orientation = SensorSable.clientOrientation(renderedPos2, event.getPartialTick().getGameTimeDeltaPartialTick(false));
        if(orientation != null)
            matrixStack.mulPose(orientation);
        var currentPos = Vec3.ZERO;
        var consumer = buffer.getBuffer(RenderType.entityTranslucent(SensorWireItem.TEXTURE));
        int color = points.reachedTarget() ? 0x80AAFFAA : 0x80FFAAAA;
        for(var p : points.points()) {
            var nextPos = currentPos.add(p.vector());
            BlockWireRenderer.renderSegment(matrixStack, consumer, LightTexture.FULL_BRIGHT, color, currentPos,
                    p.direction, SensorWireItem.THICKNESS, p.length(), 0);
            currentPos = nextPos;
        }
        matrixStack.popPose();
        buffer.endBatch(RenderType.entityTranslucent(SensorWireItem.TEXTURE));
    }
}
//?}
