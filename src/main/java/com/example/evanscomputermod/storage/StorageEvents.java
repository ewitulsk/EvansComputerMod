package com.example.evanscomputermod.storage;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.storage.core.TokenBook;
import com.example.evanscomputermod.storage.device.IStorageDevice;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.item.StorageModuleItem;
import com.example.evanscomputermod.storage.ledger.ItemTypes;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.neoforged.bus.api.EventPriority;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.entity.EntityLeaveLevelEvent;
import net.neoforged.neoforge.event.entity.item.ItemExpireEvent;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;
import net.neoforged.neoforge.event.server.ServerStoppedEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * World-level storage upkeep: the ledger's end-of-tick work, voiding cells
 * whose item is destroyed, and telling computers about expired tokens.
 *
 * <p>A cell's contents are voided when its item is destroyed: burnt, blown
 * up or cactus'd (the item's {@code onDestroyed}), despawned
 * ({@link ItemExpireEvent}), fallen out of the world or {@code /kill}ed.
 * Anything else (being picked up by a player, hopper or mob) leaves the cell
 * alone; cells nobody mounts for a long time are collected by the ledger.
 */
public final class StorageEvents {

    /** Devices with mounted cells, by id (for token-expiry messages). */
    private static final Map<UUID, IStorageDevice> LIVE_DEVICES = new ConcurrentHashMap<>();

    private StorageEvents() {
    }

    public static void register() {
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Post e) ->
                StorageLedger.get(e.getServer()).endTick(e.getServer().getTickCount()));
        NeoForge.EVENT_BUS.addListener((ServerStoppedEvent e) -> {
            StorageLedger.release();
            LIVE_DEVICES.clear();
        });
        NeoForge.EVENT_BUS.addListener(EventPriority.LOWEST, (ItemExpireEvent e) -> {
            if (e.getExtraLife() <= 0) itemGone(e.getEntity());
        });
        NeoForge.EVENT_BUS.addListener(StorageEvents::onLeaveLevel);
        NeoForge.EVENT_BUS.addListener((PlayerEvent.PlayerLoggedInEvent e) -> touchInventory(e.getEntity()));
        NeoForge.EVENT_BUS.addListener((PlayerEvent.PlayerLoggedOutEvent e) -> touchInventory(e.getEntity()));
    }

    public static void deviceLive(IStorageDevice device, boolean live) {
        if (live) LIVE_DEVICES.put(device.deviceId(), device);
        else LIVE_DEVICES.remove(device.deviceId(), device);
    }

    private static void onLeaveLevel(EntityLeaveLevelEvent e) {
        if (!(e.getEntity() instanceof ItemEntity item) || !(e.getLevel() instanceof ServerLevel level)) return;
        Entity.RemovalReason reason = item.getRemovalReason();
        boolean belowWorld = item.getY() < level.getMinBuildHeight() - 32;
        if (reason == Entity.RemovalReason.KILLED || (belowWorld && reason != null && reason.shouldDestroy())) {
            itemGone(item);
        }
    }

    /** An item entity holding {@code stack} was destroyed. */
    public static void itemGone(ItemEntity entity) {
        if (!(entity.level() instanceof ServerLevel level)) return;
        ItemStack stack = entity.getItem();
        if (stack.isEmpty()) return;
        StorageLedger ledger = StorageLedger.get(level.getServer());
        UUID cell = StorageCellItem.cellId(stack);
        if (cell != null) ledger.voidCell(cell);
        if (stack.getItem() instanceof StorageModuleItem) {
            UUID inner = StorageCellItem.cellId(StorageModuleItem.cellIn(stack, level.registryAccess()));
            if (inner != null) ledger.voidCell(inner);
        }
    }

    /** Players carrying cells keep them from being garbage-collected. */
    private static void touchInventory(Player player) {
        if (!(player.level() instanceof ServerLevel level)) return;
        StorageLedger ledger = StorageLedger.get(level.getServer());
        for (ItemStack s : player.getInventory().items) {
            UUID id = StorageCellItem.cellId(s);
            if (id != null) ledger.touch(id);
        }
    }

    /** Called by the ledger when a token expires. */
    public static void tokenExpired(StorageLedger ledger, TokenBook.Expired e) {
        IStorageDevice issuer = LIVE_DEVICES.get(e.token().issuer());
        if (issuer == null) return;
        for (IComputerAccess c : issuer.watchers()) {
            c.queueEvent("token_expired", e.token().id(), ItemTypes.keyOf(e.token().key()), e.token().count(),
                    e.refunded() ? "refunded" : "lost");
        }
    }
}
//?}
