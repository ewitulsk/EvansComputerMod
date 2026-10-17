package com.example.evanscomputermod.card.nfc;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardReaderModule;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.cellular.ModemSimStore;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.vehicle.ContainerEntity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.phys.AABB;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.server.ServerStoppedEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.items.IItemHandler;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Cards answer readers from anywhere within range: a hand, a hotbar, a chest, a hopper,
 * an item on the ground, an item frame, a minecart chest, a phone's bays. Rather than give
 * every card an endpoint, this keeps endpoints only for cards inside a live reader's
 * field: each tick with any field on, it scans each field's volume ({@link #RADIUS}
 * blocks; the path model, not the volume, sets the real range) for item entities, block
 * entities with an item handler, container entities and players, and registers or
 * refreshes a {@link CardEndpoint} per card found. Endpoints not seen for
 * {@link #EXPIRE_TICKS} ticks go away. No field anywhere, nothing runs.
 */
public final class CardEndpointTracker {
    public static final double RADIUS = 3;
    public static final int EXPIRE_TICKS = 10;

    private static final Set<NfcRadio> FIELDS = ConcurrentHashMap.newKeySet();
    private static final Map<Object, Tracked> TRACKED = new HashMap<>();
    private static final Random RND = new Random();
    private static long tick;
    private static RadioMedium medium;
    private static volatile long lastScanNanos;

    private record Tracked(CardEndpoint endpoint, long[] lastSeen) {}

    private CardEndpointTracker() {}

    public static void register() {
        NfcRadio.onFieldOn = CardEndpointTracker::fieldOn;
        NfcRadio.onFieldOff = CardEndpointTracker::fieldOff;
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Post e) -> tick(e.getServer()));
        NeoForge.EVENT_BUS.addListener((ServerStoppedEvent e) -> {
            FIELDS.clear();
            TRACKED.clear();
            medium = null;
        });
    }

    static void fieldOn(NfcRadio r) {
        FIELDS.add(r);
    }

    static void fieldOff(NfcRadio r) {
        FIELDS.remove(r);
    }

    /** Endpoints live now (tests). */
    public static int tracked() {
        return TRACKED.size();
    }

    /** How long the last scan took, nanoseconds (tests: the budget). */
    public static long lastScanNanos() {
        return lastScanNanos;
    }

    private static void tick(MinecraftServer server) {
        tick++;
        RadioMedium m = RadioMediumHooks.medium();
        if (m != medium) {
            TRACKED.clear();
            medium = m;
        }
        if (m == null) return;
        if (FIELDS.isEmpty()) {
            if (!TRACKED.isEmpty()) expire(m, Long.MAX_VALUE);
            return;
        }
        long t0 = System.nanoTime();
        for (NfcRadio r : FIELDS) {
            Pose p = r.pose();
            if (p == null) continue;
            ServerLevel level = levelOf(server, p.dimension());
            if (level == null) continue;
            scan(level, p, m);
            r.seen().values().removeIf(last -> tick - last > EXPIRE_TICKS);
        }
        expire(m, EXPIRE_TICKS);
        lastScanNanos = System.nanoTime() - t0;
    }

    private static ServerLevel levelOf(MinecraftServer server, String dimension) {
        for (ServerLevel l : server.getAllLevels()) if (l.dimension().location().toString().equals(dimension)) return l;
        return null;
    }

    private static void expire(RadioMedium m, long age) {
        for (Iterator<Map.Entry<Object, Tracked>> it = TRACKED.entrySet().iterator(); it.hasNext(); ) {
            Tracked t = it.next().getValue();
            if (tick - t.lastSeen[0] > age) {
                m.unregister(t.endpoint);
                it.remove();
            }
        }
    }

    private static void scan(ServerLevel level, Pose at, RadioMedium m) {
        String dim = at.dimension();
        AABB box = new AABB(at.x() - RADIUS, at.y() - RADIUS, at.z() - RADIUS, at.x() + RADIUS, at.y() + RADIUS, at.z() + RADIUS);
        // items on the ground and in item frames / minecart chests
        for (ItemEntity e : level.getEntitiesOfClass(ItemEntity.class, box)) {
            ItemStack s = e.getItem();
            if (CardItem.kindOf(s) != null)
                found(e, s, Pose.at(dim, e.getX(), e.getY() + 0.1, e.getZ()), 0, m, () -> {});
        }
        for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.ItemFrame.class, box)) {
            ItemStack s = e.getItem();
            if (CardItem.kindOf(s) != null) found(e, s, Pose.at(dim, e.getX(), e.getY(), e.getZ()), 0, m, () -> e.setItem(s, false));
        }
        for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.Entity.class, box, en -> en instanceof ContainerEntity)) {
            ContainerEntity c = (ContainerEntity) e;
            for (int i = 0; i < c.getContainerSize(); i++) {
                ItemStack s = c.getItem(i);
                if (CardItem.kindOf(s) != null)
                    found(new Key(e, i), s, Pose.at(dim, e.getX(), e.getY(), e.getZ()), 0, m, c::setChanged);
            }
        }
        // block entities with an item handler (chests, hoppers, readers, printers, barrels, …)
        int r = (int) Math.ceil(RADIUS);
        BlockPos c = BlockPos.containing(at.x(), at.y(), at.z());
        for (BlockPos pos : BlockPos.betweenClosed(c.offset(-r, -r, -r), c.offset(r, r, r))) {
            BlockEntity be = level.getBlockEntity(pos);
            if (be == null) continue;
            IItemHandler h = level.getCapability(Capabilities.ItemHandler.BLOCK, pos, null);
            if (h == null) continue;
            int n = Math.min(h.getSlots(), 64);
            for (int i = 0; i < n; i++) {
                ItemStack s = h.getStackInSlot(i);
                if (CardItem.kindOf(s) != null)
                    found(new Key(be, i), s, Pose.at(dim, pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5), 0, m, be::setChanged);
            }
        }
        // players: inventory, offhand, and cards inside their phones' bays
        for (ServerPlayer p : level.players()) {
            if (!box.contains(p.getX(), p.getY(), p.getZ())) continue;
            var inv = p.getInventory();
            double speed = p.getDeltaMovement().length() * 20;
            for (int i = 0; i < inv.getContainerSize(); i++) {
                ItemStack s = inv.getItem(i);
                if (s.isEmpty()) continue;
                Pose pose = i == inv.selected || i == 40
                        ? Pose.at(dim, p.getX(), p.getEyeY() - 0.4, p.getZ())
                        : Pose.at(dim, p.getX(), p.getY() + 0.9, p.getZ());
                if (CardItem.kindOf(s) != null) found(new Key(p, i), s, pose, speed, m, inv::setChanged);
                else if (s.has(PhoneContent.HARDWARE.get())) {
                    PhoneHardware hw = s.get(PhoneContent.HARDWARE.get());
                    if (hw == null) continue;
                    for (ItemStack mod : new ItemStack[] {hw.module1(), hw.module2()}) {
                        if (!(mod.getItem() instanceof com.example.evanscomputermod.card.CardReaderModuleItem)) continue;
                        String slot = ModemSimStore.slotOf(com.example.evanscomputermod.module.ModuleState.read(mod).getCompound("card"));
                        ItemStack card = ModemSimStore.peek(slot);
                        if (!card.isEmpty()) found(new Key(p, "phone:" + slot), card, pose, speed, m, () -> {});
                    }
                    if (CardItem.kindOf(hw.sim()) != null) found(new Key(p, "sim:" + i), hw.sim(), pose, speed, m, inv::setChanged);
                }
            }
        }
    }

    private record Key(Object owner, Object slot) {}

    /** A card item as the air interface sees it. */
    record StackCard(ItemStack stack) implements NfcCard {
        @Override public com.example.evanscomputermod.card.applet.NfcApplet nfc() { return CardItem.nfc(stack); }
        @Override public long uid() { return CardItem.uid(stack); }
        @Override public int kinds() { return kindsOf(stack); }
    }

    public static int kindsOf(ItemStack card) {
        com.example.evanscomputermod.card.CardKind k = CardItem.kindOf(card);
        if (k == null) return 0;
        return (k.bank ? NfcAir.KIND_BANK : 0) | (k.nfc ? NfcAir.KIND_NFC : 0) | (k.sim ? NfcAir.KIND_SIM : 0);
    }

    private static void found(Object key, ItemStack s, Pose pose, double speed, RadioMedium m, Runnable dirty) {
        NfcCard stack = new StackCard(s);
        Tracked t = TRACKED.get(key);
        if (t == null) {
            CardEndpoint ep = new CardEndpoint(stack, pose, m::nowMicros, RND);
            ep.attach(m);
            m.register(ep);
            t = new Tracked(ep, new long[] {tick});
            TRACKED.put(key, t);
        } else {
            boolean moved = !pose.equals(t.endpoint.pose());
            t.endpoint.refresh(stack, pose, speed, dirty);
            t.lastSeen[0] = tick;
            if (moved) m.invalidate(t.endpoint);   // a card on the move (a dropped item, a player): re-trace
        }
    }

    /** Every tracked endpoint (tests). */
    public static List<CardEndpoint> endpoints() {
        List<CardEndpoint> l = new ArrayList<>();
        for (Tracked t : TRACKED.values()) l.add(t.endpoint);
        return l;
    }

    /** Bays of a module host that may hold cards (unused for now: phone bays are read from the item). */
    static List<ItemStack> cardsIn(ModuleBays bays) {
        List<ItemStack> l = new ArrayList<>();
        for (int i = 0; i < ModuleBays.SLOTS; i++)
            if (bays.getModule(i) instanceof CardReaderModule mod && mod.hasCard()) l.add(mod.card());
        return l;
    }
}
//?}
