/*
 * Copyright 2025 patryk3211
 *
 * Modified 2026 for Evans Computer Mod: ported from PowerGrid
 * (org.patryk3211.powergrid.electricity.wire.BaseWireEntity and WireEntity);
 * removed temperature, overheating, current and the electrical network link,
 * client sync uses IEntityWithComplexSpawn plus a NeoForge payload, endpoints
 * that disappear (block broken, module taken out) are noticed by a periodic
 * check, and wire cutters are the evanscomputermod:wire_cutters tag.
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
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtUtils;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.DyeItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.material.PushReaction;
import net.minecraft.network.syncher.SynchedEntityData;
import net.neoforged.neoforge.entity.IEntityWithComplexSpawn;
import net.neoforged.neoforge.network.PacketDistributor;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

public abstract class BaseWireEntity extends Entity implements IEntityWithComplexSpawn {
    public static final int DEFAULT_COLOR = 0x3a3d42;
    private static final int VALIDATE_INTERVAL = 10;

    protected IWireEndpoint endpoint1;
    protected IWireEndpoint endpoint2;
    protected byte deferEndpointResolution = 0;

    @NotNull
    private Item item = net.minecraft.world.item.Items.AIR;
    protected int itemCount;
    private int color = DEFAULT_COLOR;
    private int dataVersion = 0;

    protected boolean sublevelMove;

    public BaseWireEntity(EntityType<?> type, Level world) {
        super(type, world);
        this.noPhysics = true;
    }

    @Override
    protected void defineSynchedData(SynchedEntityData.Builder builder) {
    }

    @Override
    public void baseTick() {
        // We don't need Entity#baseTick() in wires
    }

    @Override
    public void tick() {
        super.tick();
        var world = level();
        if(world.isClientSide) {
            this.firstTick = false;
            return;
        }

        if((deferEndpointResolution & 1) != 0) {
            if(endpoint1 != null && endpoint1.isValid(world)) {
                endpoint1.assignWireEntity(this);
                deferEndpointResolution &= ~1;
            }
        }
        if((deferEndpointResolution & 2) != 0) {
            if(endpoint2 != null && endpoint2.isValid(world)) {
                endpoint2.assignWireEntity(this);
                deferEndpointResolution &= ~2;
            }
        }

        if(sublevelMove && deferEndpointResolution == 0) {
            sendExtraData();
            sublevelMove = false;
        }

        // A connection point can vanish without telling us (a module taken out of a
        // computer, a block replaced): drop that end like PowerGrid does when the block breaks.
        if((tickCount + getId()) % VALIDATE_INTERVAL == 0) {
            if(endpoint1 instanceof BlockWireEndpoint b && (deferEndpointResolution & 1) == 0 && b.isGone(world)) {
                endpointRemoved(b);
            } else if(endpoint2 instanceof BlockWireEndpoint b && (deferEndpointResolution & 2) == 0 && b.isGone(world)) {
                endpointRemoved(b);
            }
        }

        this.firstTick = false;
    }

    public void setEndpoint1(IWireEndpoint endpoint) {
        endpoint1 = changeEndpoint(endpoint1, endpoint, 1);
    }

    public void setEndpoint2(IWireEndpoint endpoint) {
        endpoint2 = changeEndpoint(endpoint2, endpoint, 2);
    }

    private IWireEndpoint changeEndpoint(IWireEndpoint current, IWireEndpoint endpoint, int bit) {
        assert endpoint == null || endpoint.canAcceptType(this.getClass()) : "Endpoint doesn't accept this entity type";
        if(current == endpoint)
            return current;
        var world = level();
        if(world.isClientSide)
            return endpoint;
        if(current != null && (deferEndpointResolution & bit) == 0)
            current.removeWireEntity(this);
        deferEndpointResolution &= (byte) ~bit;
        if(endpoint != null) {
            if(endpoint.type() == WireEndpointType.DEFERRED_JUNCTION)
                endpoint = ((DeferredJunctionWireEndpoint) endpoint).resolve(world);
            if(endpoint != null) {
                if(endpoint.isValid(world)) {
                    endpoint.assignWireEntity(this);
                } else {
                    deferEndpointResolution |= (byte) bit;
                }
            }
        }
        return endpoint;
    }

    public void sublevelMove(IWireEndpoint endpoint1, IWireEndpoint endpoint2) {
        if(this.endpoint1 != null && this.endpoint1 != endpoint1 && (deferEndpointResolution & 1) == 0)
            this.endpoint1.moveWireEntity(this);
        if(this.endpoint2 != null && this.endpoint2 != endpoint2 && (deferEndpointResolution & 2) == 0)
            this.endpoint2.moveWireEntity(this);
        this.endpoint1 = endpoint1;
        this.endpoint2 = endpoint2;
        deferEndpointResolution = 3;
        sublevelMove = true;
    }

    public void sublevelRotate(Rotation rotation) {
    }

    public IWireEndpoint getEndpoint1() {
        return endpoint1;
    }

    public IWireEndpoint getEndpoint2() {
        return endpoint2;
    }

    public void endpointRemoved(IWireEndpoint endpoint) {
    }

    // ------------------------------------------------------------ client sync

    private CompoundTag syncTag() {
        var tag = new CompoundTag();
        addAdditionalSaveData(tag);
        tag.putInt("Version", dataVersion++);
        return tag;
    }

    public void sendExtraData() {
        if(level().isClientSide)
            return;
        PacketDistributor.sendToPlayersTrackingEntity(this,
                new WirePackets.WireData(getId(), getX(), getY(), getZ(), syncTag()));
    }

    @Override
    public void writeSpawnData(RegistryFriendlyByteBuf buffer) {
        buffer.writeNbt(syncTag());
    }

    @Override
    public void readSpawnData(RegistryFriendlyByteBuf buffer) {
        var tag = buffer.readNbt();
        if(tag != null)
            onEntityDataPacket(tag);
    }

    /** Client: full state from the server. */
    public void onEntityDataPacket(CompoundTag data) {
        int version = data.getInt("Version");
        if(version < dataVersion) {
            // Discard outdated packet.
            return;
        }
        readAdditionalSaveData(data);
        dataVersion = version + 1;
    }

    // ------------------------------------------------------------ persistence

    @Override
    protected void readAdditionalSaveData(CompoundTag nbt) {
        if(nbt.contains("Item")) {
            var itemTag = nbt.getCompound("Item");
            var readItem = BuiltInRegistries.ITEM.get(ResourceLocation.parse(itemTag.getString("Id")));
            setItem(readItem, itemTag.getInt("Count"));
        } else {
            setItem(SensorContent.SENSOR_WIRE.get(), 0);
        }
        if(nbt.contains("Color"))
            color = nbt.getInt("Color");

        BlockPos lastPos = null;
        if(nbt.contains("LastKnownPos")) {
            lastPos = NbtUtils.readBlockPos(nbt, "LastKnownPos").orElse(null);
        }

        IWireEndpoint endpoint1 = null, endpoint2 = null;
        if(nbt.contains("Endpoint1")) {
            endpoint1 = WireEndpointType.deserialize(nbt.getCompound("Endpoint1"));
        }
        if(nbt.contains("Endpoint2")) {
            endpoint2 = WireEndpointType.deserialize(nbt.getCompound("Endpoint2"));
        }

        if(!level().isClientSide) {
            // Moved or copied (e.g. a structure): shift the ends with the wire.
            var currentPos = blockPosition();
            if(lastPos != null && !lastPos.equals(currentPos)) {
                var diff = currentPos.subtract(lastPos);
                if(endpoint1 != null)
                    endpoint1 = endpoint1.makeOffset(diff);
                if(endpoint2 != null)
                    endpoint2 = endpoint2.makeOffset(diff);
            }
        }

        setEndpoint1(endpoint1);
        setEndpoint2(endpoint2);
    }

    @Override
    protected void addAdditionalSaveData(CompoundTag nbt) {
        if(endpoint1 != null)
            nbt.put("Endpoint1", endpoint1.serialize());
        if(endpoint2 != null)
            nbt.put("Endpoint2", endpoint2.serialize());

        var itemTag = new CompoundTag();
        itemTag.putString("Id", BuiltInRegistries.ITEM.getKey(item).toString());
        itemTag.putInt("Count", itemCount);
        nbt.put("Item", itemTag);
        nbt.putInt("Color", color);
        nbt.put("LastKnownPos", NbtUtils.writeBlockPos(blockPosition()));
    }

    public void setColor(int color) {
        this.color = color;
    }

    public void setColor(DyeColor color) {
        this.color = color.getTextureDiffuseColor() & 0xFFFFFF;
    }

    public int getColor() {
        return color;
    }

    public void setItem(Item item, int count) {
        this.item = item;
        this.itemCount = count;
    }

    public Item getItem() {
        return item;
    }

    public int getWireCount() {
        return itemCount;
    }

    public void incrementWireCount(int count) {
        itemCount += count;
        if(itemCount < 0)
            itemCount = 0;
    }

    public float thickness() {
        return SensorWireItem.THICKNESS;
    }

    // ------------------------------------------------------------ lifecycle

    @Override
    public @Nullable ItemStack getPickResult() {
        return new ItemStack(item, 1);
    }

    @Override
    public void remove(@NotNull RemovalReason reason) {
        super.remove(reason);
        if(reason.shouldDestroy() && !level().isClientSide) {
            if(endpoint1 != null && (deferEndpointResolution & 1) == 0)
                endpoint1.removeWireEntity(this);
            if(endpoint2 != null && (deferEndpointResolution & 2) == 0)
                endpoint2.removeWireEntity(this);
        }
    }

    @Override
    public void kill() {
        for(int i = itemCount; i > 0; i -= 64) {
            spawnAtLocation(new ItemStack(item, Math.min(i, 64)));
        }
        itemCount = 0;
        super.kill();
    }

    private void cut() {
        level().playSound(null, getX(), getY(), getZ(), SoundEvents.SHEEP_SHEAR, SoundSource.BLOCKS, 0.75f, 1.25f);
        kill();
    }

    @Override
    public InteractionResult interact(Player player, InteractionHand hand) {
        var stack = player.getItemInHand(hand);
        if(stack.is(SensorContent.WIRE_CUTTERS)) {
            if(!level().isClientSide)
                cut();
            return InteractionResult.sidedSuccess(level().isClientSide);
        } else if(stack.getItem() instanceof DyeItem dye) {
            if(!level().isClientSide) {
                setColor(dye.getDyeColor());
                sendExtraData();
            }
            return InteractionResult.sidedSuccess(level().isClientSide);
        }
        return super.interact(player, hand);
    }

    @Override
    public boolean skipAttackInteraction(Entity attacker) {
        return true;
    }

    @Override
    public boolean hurt(DamageSource source, float amount) {
        return false;
    }

    @Override
    public void setSharedFlagOnFire(boolean onFire) {
    }

    @Override
    public PushReaction getPistonPushReaction() {
        return PushReaction.IGNORE;
    }

    @Override
    public boolean isNoGravity() {
        return true;
    }
}
//?}
