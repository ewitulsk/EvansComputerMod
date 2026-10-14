package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;
import net.neoforged.neoforge.event.server.ServerStoppingEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.WeakHashMap;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Runs every ePhone on a server. Each tick it looks through every player's
 * hotbar and off hand for phones and tells each phone's {@link PhoneHost}
 * where it is: held (main hand or off hand) = awake, in the hotbar = asleep,
 * not found = off. When duplicates of one phone are in several places the
 * best placement wins (held over hotbar), and on a tie the current holder
 * keeps it, so two players holding copies don't make it flap.
 *
 * <p>Players that the server doesn't tick or list (the scenario and test
 * {@code FakePlayer}s) are included with {@link #track}.
 */
public final class PhoneServer {

    private static final Map<MinecraftServer, Map<UUID, PhoneHost>> HOSTS = new WeakHashMap<>();
    private static final Set<ServerPlayer> TRACKED = Collections.newSetFromMap(new WeakHashMap<>());
    private static final Map<UUID, PhonePackets.Prefs> PREFS = new ConcurrentHashMap<>();

    private PhoneServer() {}

    /** The host of phone {@code id} on {@code server} (created on first use). */
    public static synchronized PhoneHost host(MinecraftServer server, UUID id) {
        return HOSTS.computeIfAbsent(server, k -> new HashMap<>()).computeIfAbsent(id, k -> new PhoneHost(server, id));
    }

    /** The host of phone {@code id}, or null if this server never saw it. */
    @Nullable
    public static synchronized PhoneHost find(MinecraftServer server, UUID id) {
        var m = HOSTS.get(server);
        return m == null ? null : m.get(id);
    }

    public static synchronized List<PhoneHost> hosts(MinecraftServer server) {
        return List.copyOf(HOSTS.getOrDefault(server, Map.of()).values());
    }

    /** Include a player the server doesn't list (a scenario's FakePlayer) in the scan. */
    public static synchronized void track(ServerPlayer player) {
        TRACKED.add(player);
    }

    public static synchronized void untrack(ServerPlayer player) {
        TRACKED.remove(player);
    }

    /** A player's visibility choices (defaults until their client says otherwise). */
    public static PhonePackets.Prefs prefs(UUID player) {
        return PREFS.getOrDefault(player, PhonePackets.Prefs.DEFAULT);
    }

    /** Test hook: set a (fake) player's choices. */
    public static void setPrefs(UUID player, PhonePackets.Prefs prefs) {
        PREFS.put(player, prefs);
    }

    /** Every player the scan sees: the server's list plus tracked fake players. */
    public static synchronized List<ServerPlayer> players(MinecraftServer server) {
        List<ServerPlayer> all = new ArrayList<>(server.getPlayerList().getPlayers());
        for (ServerPlayer p : TRACKED) if (p.getServer() == server && !all.contains(p)) all.add(p);
        return all;
    }

    /** Players (listed or tracked) in {@code level} within {@code range} of {@code at}. */
    public static List<ServerPlayer> playersNear(ServerLevel level, Vec3 at, double range) {
        List<ServerPlayer> out = new ArrayList<>();
        for (ServerPlayer p : players(level.getServer())) {
            if (p.level() == level && p.position().distanceToSqr(at) <= range * range) out.add(p);
        }
        return out;
    }

    // ------------------------------------------------------------ the scan

    private record Found(ServerPlayer player, ItemStack stack, boolean held) {}

    public static void onServerTick(ServerTickEvent.Post event) {
        tick(event.getServer());
    }

    static void tick(MinecraftServer server) {
        Map<UUID, PhoneHost> hosts;
        synchronized (PhoneServer.class) {
            hosts = HOSTS.computeIfAbsent(server, k -> new HashMap<>());
        }
        Map<UUID, Found> best = new HashMap<>();
        for (ServerPlayer p : players(server)) {
            if (p.isRemoved() || p.isDeadOrDying() || p.isSpectator()) continue;
            Inventory inv = p.getInventory();
            for (int slot = 0; slot < Inventory.getSelectionSize(); slot++) {
                consider(best, hosts, p, inv.getItem(slot), slot == inv.selected);
            }
            consider(best, hosts, p, p.getItemInHand(InteractionHand.OFF_HAND), true);
        }
        List<PhoneHost> all;
        synchronized (PhoneServer.class) {
            for (UUID id : best.keySet()) hosts.computeIfAbsent(id, k -> new PhoneHost(server, k));
            all = List.copyOf(hosts.values());
        }
        for (PhoneHost h : all) {
            Found f = best.get(h.getComputerId());
            try {
                h.tick(f == null ? null : new PhoneHost.Holding(f.player(), f.stack(), f.held()));
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("ePhone {} tick failed", h.getComputerId(), e);
            }
        }
    }

    private static void consider(Map<UUID, Found> best, Map<UUID, PhoneHost> hosts, ServerPlayer p, ItemStack stack, boolean held) {
        UUID id = PhoneItemData.id(stack);
        if (id == null) return;
        Found cur = best.get(id);
        if (cur != null) {
            if (cur.held() && !held) return;
            if (cur.held() == held) {
                // Tie: whoever has it now keeps it.
                PhoneHost h = hosts.get(id);
                if (h == null || !p.getUUID().equals(h.holderId())) return;
            }
        }
        best.put(id, new Found(p, stack, held));
    }

    // ------------------------------------------------------------ events and packets

    /** A holder left: their phones' computers stop (they cold-boot next time). */
    public static void onLogout(PlayerEvent.PlayerLoggedOutEvent event) {
        if (!(event.getEntity() instanceof ServerPlayer player)) return;
        MinecraftServer server = player.getServer();
        if (server == null) return;
        for (PhoneHost h : hosts(server)) {
            if (player.getUUID().equals(h.holderId()) && (h.isAttached() || h.instance() != null)) h.powerOff("holder logged out");
        }
        PREFS.remove(player.getUUID());
    }

    public static void onServerStopping(ServerStoppingEvent event) {
        synchronized (PhoneServer.class) {
            HOSTS.remove(event.getServer());
        }
    }

    /** The holder pressed a hotkey: only for a phone they hold in a hand. */
    public static void handleHotkey(PhonePackets.Hotkey packet, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer p) deliverHotkey(p, packet);
        });
    }

    /** Hand a hotkey from {@code player} to its phone. Returns false if refused (not held, not awake). */
    public static boolean deliverHotkey(ServerPlayer player, PhonePackets.Hotkey packet) {
        boolean inHand = false;
        for (InteractionHand hand : InteractionHand.values()) {
            inHand |= packet.phone().equals(PhoneItemData.id(player.getItemInHand(hand)));
        }
        if (!inHand) return false;
        PhoneHost h = find(player.server, packet.phone());
        if (h == null || h.state() != PhoneState.AWAKE || !player.getUUID().equals(h.holderId())) return false;
        h.hotkey(packet.slot(), packet.down());
        return true;
    }

    public static void handlePrefs(PhonePackets.Prefs packet, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer p) PREFS.put(p.getUUID(), packet);
        });
    }
}
//?}
