/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.WireItem, the terminal handling
 * of electricity.base.IElectric and utility.PlayerUtilities); one wire type
 * with fixed properties, block wires only (no hanging wires or cords), NeoForge
 * events instead of Architectury ones.
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
import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.ChatFormatting;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.ContainerHelper;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.InteractionResultHolder;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.DyeItem;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;

/**
 * Sensor Wire: right-click a connection point (a lidar sensor, or a computer
 * bay with a Wired Sensor Module), then block faces to route the wire, then
 * another connection point or wire. Sneak-right-click the air to cancel.
 */
public class SensorWireItem extends TooltipItem {
    public static final float THICKNESS = 1 / 16f;
    public static final float ITEMS_PER_METER = 0.5f;
    public static final ResourceLocation TEXTURE = EvansComputerMod.id("textures/entity/sensor_wire.png");

    /** Players holding the alternate-placement key (straight L/Z paths), as reported by their clients. */
    private static final Set<UUID> ALTERNATE = ConcurrentHashMap.newKeySet();

    public SensorWireItem(Properties settings) {
        super(settings);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.sensor_wire.tooltip").withStyle(ChatFormatting.GRAY));
        if(stack.has(SensorContent.CONNECTION_DATA.get()))
            lines.accept(Component.translatable("item.evanscomputermod.sensor_wire.pending").withStyle(ChatFormatting.DARK_GRAY));
    }

    @Override
    public boolean isFoil(ItemStack stack) {
        return super.isFoil(stack) || stack.has(SensorContent.CONNECTION_DATA.get());
    }

    public static void setAlternate(Player player, boolean on) {
        if(on)
            ALTERNATE.add(player.getUUID());
        else
            ALTERNATE.remove(player.getUUID());
    }

    public static boolean alternateWirePlacement(@Nullable Player player) {
        if(player == null)
            return false;
        if(player.level().isClientSide)
            return com.example.evanscomputermod.sensor.client.SensorClient.alternatePlacementHeld();
        return ALTERNATE.contains(player.getUUID());
    }

    private static void message(@Nullable Player player, String key, ChatFormatting style) {
        if(player != null)
            player.displayClientMessage(Component.translatable("message.evanscomputermod.wire." + key).withStyle(style), true);
    }

    // ------------------------------------------------------------ items

    public static boolean hasEnoughItems(@Nullable Player player, ItemStack usedStack, int requiredCount) {
        if(player != null) {
            if(player.isCreative())
                return true;
            return player.getInventory().countItem(usedStack.getItem()) >= requiredCount;
        }
        return usedStack.getCount() >= requiredCount;
    }

    public static void removeItems(@Nullable Player player, ItemStack usedStack, int count) {
        if(count <= 0)
            return;
        if(player != null) {
            if(player.isCreative())
                return;
            ContainerHelper.clearOrCountMatchingItems(player.getInventory(), s -> s.is(usedStack.getItem()), count, false);
            return;
        }
        usedStack.shrink(Math.min(count, usedStack.getCount()));
    }

    // ------------------------------------------------------------ placement

    public static InteractionResultHolder<BlockWireEntity> connect(Level world, ItemStack stack, @Nullable Player player,
                                                                   IWireEndpoint endpoint1, IWireEndpoint endpoint2) {
        if(!SensorSable.sameSubLevel(world, endpoint1.getExactPosition(world), endpoint2.getExactPosition(world))) {
            // Abort, block wires must be in the same sublevel.
            message(player, "connection_failed", ChatFormatting.RED);
            return InteractionResultHolder.fail(null);
        }
        if(endpoint1.type() == WireEndpointType.BLOCK_WIRE && endpoint2.type() == WireEndpointType.BLOCK_WIRE)
            return mergeWires(world, stack, player, (BlockWireEntityEndpoint) endpoint1, (BlockWireEntityEndpoint) endpoint2);

        if(endpoint1.type() == WireEndpointType.BLOCK && endpoint2.type() == WireEndpointType.BLOCK_WIRE) {
            var e = endpoint1;
            endpoint1 = endpoint2;
            endpoint2 = e;
        }

        var lastPoint = endpoint1.getExactPosition(world);
        if(endpoint1.type() != WireEndpointType.BLOCK_WIRE)
            lastPoint = BlockTrace.alignPosition(lastPoint);
        var targetPoint = endpoint2.getExactPosition(world);

        Direction continueDir = null;
        if(endpoint1 instanceof BlockWireEntityEndpoint bwe) {
            var entity = bwe.getEntity(world);
            if(entity == null)
                return InteractionResultHolder.fail(null);
            var segments = entity.segments;
            if(segments.isEmpty())
                return InteractionResultHolder.fail(null);
            if(bwe.getEnd()) {
                continueDir = segments.get(segments.size() - 1).direction;
            } else {
                continueDir = segments.get(0).direction.getOpposite();
            }
        }

        WireTerminal terminal = null;
        if(endpoint2 instanceof BlockWireEndpoint wireEndpoint) {
            terminal = wireEndpoint.getTerminalPlacement(world);
        }

        var result = alternateWirePlacement(player)
                ? BlockTrace.alternatePath(lastPoint, targetPoint)
                : BlockTrace.findPath(world, lastPoint, targetPoint, terminal, continueDir);
        if(result != null && result.reachedTarget()) {
            float addedLength = 0;
            for(var point : result.points())
                addedLength += point.length();

            if(endpoint1.type() != WireEndpointType.BLOCK_WIRE) {
                // New entity must be created.
                var newItems = Math.max((int) Math.ceil(addedLength * ITEMS_PER_METER), 1);
                if(!hasEnoughItems(player, stack, newItems)) {
                    message(player, "connection_missing_items", ChatFormatting.RED);
                    return InteractionResultHolder.fail(null);
                }
                if(!world.isClientSide) {
                    var entity = BlockWireEntity.create(world, endpoint1, stack.copyWithCount(newItems), result.points());
                    if(endpoint2.type().isConnectable())
                        entity.setEndpoint2(endpoint2);
                    if(player != null && player.getOffhandItem().getItem() instanceof DyeItem dye)
                        entity.setColor(dye.getDyeColor());
                    if(!((ServerLevel) world).tryAddFreshEntityWithPassengers(entity)) {
                        EvansComputerMod.LOGGER.error("Failed to spawn new block wire entity.");
                        message(player, "connection_failed", ChatFormatting.RED);
                        return InteractionResultHolder.fail(null);
                    }
                    removeItems(player, stack, newItems);
                    return InteractionResultHolder.success(entity);
                }
            } else {
                // Entity exists, we just need to extend it.
                var bwEndpoint = (BlockWireEntityEndpoint) endpoint1;
                var wire = bwEndpoint.getEntity(world);
                if(wire.getItem() != stack.getItem()) {
                    message(player, "connection_incorrect_wire_type", ChatFormatting.RED);
                    return InteractionResultHolder.fail(null);
                }

                var newItems = (int) Math.ceil((wire.getTotalLength() + addedLength) * ITEMS_PER_METER - wire.getWireCount());
                if(!hasEnoughItems(player, stack, newItems)) {
                    message(player, "connection_missing_items", ChatFormatting.RED);
                    return InteractionResultHolder.fail(null);
                }

                if(!world.isClientSide) {
                    if(!bwEndpoint.getEnd()) {
                        EvansComputerMod.LOGGER.error("Cannot extend wire at start (must be flipped beforehand)");
                        return InteractionResultHolder.fail(null);
                    }
                    wire.extend(result.points(), Math.max(newItems, 0));
                    if(endpoint2.type().isConnectable())
                        wire.setEndpoint2(endpoint2);
                    wire.sendExtraData();
                    removeItems(player, stack, newItems);
                    return InteractionResultHolder.success(wire);
                }
            }
            return InteractionResultHolder.success(null);
        }
        return InteractionResultHolder.fail(null);
    }

    public static InteractionResultHolder<BlockWireEntity> mergeWires(Level world, ItemStack stack, @Nullable Player player,
                                                                      BlockWireEntityEndpoint endpoint1, BlockWireEntityEndpoint endpoint2) {
        if(world.isClientSide)
            return InteractionResultHolder.success(null);

        var lastPoint = endpoint1.getExactPosition(world);
        var targetPoint = endpoint2.getExactPosition(world);

        var entity1 = endpoint1.getEntity(world);
        var entity2 = endpoint2.getEntity(world);
        if(entity1 == null || entity2 == null || entity1 == entity2 || entity1.segments.isEmpty())
            return InteractionResultHolder.fail(null);
        if(entity1.getItem() != entity2.getItem() || entity1.getItem() != stack.getItem()) {
            message(player, "connection_incorrect_wire_type", ChatFormatting.RED);
            return InteractionResultHolder.fail(null);
        }

        Direction continueDir;
        var currentSegments = entity1.segments;
        if(endpoint1.getEnd()) {
            continueDir = currentSegments.get(currentSegments.size() - 1).direction;
        } else {
            continueDir = currentSegments.get(0).direction.getOpposite();
        }

        var result = alternateWirePlacement(player)
                ? BlockTrace.alternatePath(lastPoint, targetPoint)
                : BlockTrace.findPath(world, lastPoint, targetPoint, null, continueDir);
        if(result == null || !result.reachedTarget())
            return InteractionResultHolder.fail(null);

        float addedLength = 0;
        for(var point : result.points())
            addedLength += point.length();

        var newItems = (int) Math.ceil((entity1.getTotalLength() + entity2.getTotalLength() + addedLength) * ITEMS_PER_METER
                - entity1.getWireCount() - entity2.getWireCount());
        if(!hasEnoughItems(player, stack, newItems)) {
            message(player, "connection_missing_items", ChatFormatting.RED);
            return InteractionResultHolder.fail(null);
        }

        BlockWireEntity targetEntity, sourceEntity;
        boolean flipped = false;
        if(endpoint1.getEnd() || !endpoint2.getEnd()) {
            // Merge endpoint2 into endpoint1
            targetEntity = entity1;
            if(!endpoint1.getEnd())
                targetEntity = targetEntity.flip();
            sourceEntity = entity2;
            if(endpoint2.getEnd())
                flipped = true;
        } else {
            // Merge endpoint1 into endpoint2
            targetEntity = entity2;
            sourceEntity = entity1;
        }

        // Add connecting path
        targetEntity.extend(result.points(), Math.max(newItems, 0), false);

        if(flipped) {
            var segments = new ArrayList<BlockWireEntity.Point>();
            for(var segment : sourceEntity.segments) {
                segments.add(0, new BlockWireEntity.Point(segment.direction.getOpposite(), segment.gridLength));
            }
            targetEntity.extend(segments, sourceEntity.getWireCount());
            targetEntity.setEndpoint2(sourceEntity.getEndpoint1());
        } else {
            targetEntity.extend(sourceEntity.segments, sourceEntity.getWireCount());
            targetEntity.setEndpoint2(sourceEntity.getEndpoint2());
        }
        targetEntity.sendExtraData();
        removeItems(player, stack, newItems);

        sourceEntity.discard();
        return InteractionResultHolder.success(targetEntity);
    }

    /** Sneak-right-click the air: forget the pending first end. */
    @Override
    public InteractionResultHolder<ItemStack> use(Level level, Player user, InteractionHand hand) {
        var stack = user.getItemInHand(hand);
        if(stack.has(SensorContent.CONNECTION_DATA.get()) && user.isShiftKeyDown()) {
            stack.remove(SensorContent.CONNECTION_DATA.get());
            if(!level.isClientSide)
                message(user, "connection_reset", ChatFormatting.GRAY);
            return InteractionResultHolder.sidedSuccess(stack, level.isClientSide);
        }
        return super.use(level, user, hand);
    }

    /**
     * Right-click on a block with the wire (called from the block right-click
     * event, before the block's own use). PASS lets the click through.
     */
    public static InteractionResult useOnBlock(Player player, InteractionHand hand, BlockPos blockPos, Direction direction) {
        if(player == null || player.isShiftKeyDown() || hand != InteractionHand.MAIN_HAND)
            return InteractionResult.PASS;
        var stack = player.getMainHandItem();
        if(!(stack.getItem() instanceof SensorWireItem))
            return InteractionResult.PASS;
        var hit = player.pick(player.blockInteractionRange() + 1.0, 0.0f, false);
        if(!(hit instanceof BlockHitResult blockHit) || hit.getType() != HitResult.Type.BLOCK
                || !blockHit.getBlockPos().equals(blockPos))
            return InteractionResult.PASS;
        return useOnBlock(player, stack, blockHit);
    }

    /** {@link #useOnBlock(Player, InteractionHand, BlockPos, Direction)} for a known hit (also used by tests). */
    public static InteractionResult useOnBlock(Player player, ItemStack stack, BlockHitResult blockHit) {
        var level = player.level();
        var blockPos = blockHit.getBlockPos();
        var state = level.getBlockState(blockPos);
        var host = IWireHost.getAt(level, blockPos);
        if(host != null) {
            var local = blockHit.getLocation().subtract(blockPos.getX(), blockPos.getY(), blockPos.getZ());
            var terminal = host.terminalIndexAt(state, local);
            if(terminal >= 0) {
                var endpoint = new BlockWireEndpoint(blockPos, terminal);
                var pending = stack.get(SensorContent.CONNECTION_DATA.get());
                if(pending != null) {
                    // Continuing a connection.
                    var first = pending.endpoint();
                    if(first == null)
                        return InteractionResult.FAIL;
                    if(first.equals(endpoint))
                        return InteractionResult.FAIL;
                    var result = connect(level, stack, player, first, endpoint);
                    if(result.getResult().consumesAction())
                        stack.remove(SensorContent.CONNECTION_DATA.get());
                    return result.getResult().consumesAction() ? InteractionResult.SUCCESS : InteractionResult.FAIL;
                } else {
                    // Must be the first connection.
                    stack.set(SensorContent.CONNECTION_DATA.get(), WireConnection.of(endpoint));
                    if(!level.isClientSide)
                        message(player, "connection_next", ChatFormatting.GRAY);
                    return InteractionResult.SUCCESS;
                }
            }
        }
        var connection = stack.get(SensorContent.CONNECTION_DATA.get());
        if(connection != null) {
            // Route along the clicked face; the wire ends there for now.
            var endpoint = connection.endpoint();
            if(endpoint == null)
                return InteractionResult.FAIL;

            var result = connect(level, stack, player, endpoint,
                    new ImaginaryWireEndpoint(blockHit.getLocation().relative(blockHit.getDirection(), 1 / 32f)));
            if(result.getResult().consumesAction()) {
                var entity = result.getObject();
                if(entity != null)
                    stack.set(SensorContent.CONNECTION_DATA.get(), WireConnection.of(new BlockWireEntityEndpoint(entity, true)));
                return InteractionResult.SUCCESS;
            }
            return InteractionResult.FAIL;
        }
        return InteractionResult.PASS;
    }
}
//?}
