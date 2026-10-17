package com.example.evanscomputermod.computer.pipeline;

import net.minecraft.server.level.ServerPlayer;

import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/**
 * Who watches and who may use a computer's displays, as of one server tick
 * (DISPLAY_PIPELINE_SPEC.md Part A2). The host builds one on the server
 * thread every tick from player positions, open menus, Sable plot tracking
 * and its own range rules, and publishes it through a volatile field. The
 * display pump, acks and input read the latest one from any thread, so they
 * never wait for a tick. A player who left between two ticks gets at most one
 * more frame, which is harmless.
 *
 * <p>Immutable.
 */
public final class ViewerSnapshot {
    /** One display's players: who gets its frames, who may ack/see it, who may give it input. */
    public record Display(Map<UUID, ServerPlayer> recipients, Set<UUID> interact, Set<UUID> input) {
        public List<UUID> recipientIds() {
            return List.copyOf(recipients.keySet());
        }
    }

    public static final ViewerSnapshot EMPTY = new ViewerSnapshot(0, Map.of());

    private final long tick;
    private final Map<Integer, Display> displays;

    private ViewerSnapshot(long tick, Map<Integer, Display> displays) {
        this.tick = tick;
        this.displays = displays;
    }

    /** The server tick this was taken at. */
    public long tick() {
        return tick;
    }

    public Display display(int displayId) {
        Display d = displays.get(displayId);
        return d != null ? d : new Display(Map.of(), Set.of(), Set.of());
    }

    /** Players that get display {@code displayId}'s frames, in a stable order. */
    public List<UUID> recipients(int displayId) {
        return display(displayId).recipientIds();
    }

    /** The online player behind a recipient id (null if not one). */
    public ServerPlayer player(int displayId, UUID id) {
        return display(displayId).recipients().get(id);
    }

    /** Any player of this snapshot with that id (recipient of any display). */
    public ServerPlayer player(UUID id) {
        for (Display d : displays.values()) {
            ServerPlayer p = d.recipients().get(id);
            if (p != null) return p;
        }
        return null;
    }

    /** May {@code player} ack or ask for frames of the display (the host's range rule at the last tick)? */
    public boolean mayInteract(UUID player, int displayId) {
        return display(displayId).interact().contains(player);
    }

    /** May {@code player} send input to the display (the host's input rule at the last tick)? */
    public boolean mayInput(UUID player, int displayId) {
        return display(displayId).input().contains(player);
    }

    /** Same viewers as {@code other} for every display (the tick doesn't count)? */
    public boolean sameRecipients(ViewerSnapshot other) {
        if (other == null) return false;
        Set<Integer> keys = new HashSet<>(displays.keySet());
        keys.addAll(other.displays.keySet());
        for (int k : keys) {
            if (!display(k).recipients().keySet().equals(other.display(k).recipients().keySet())) return false;
        }
        return true;
    }

    public static Builder builder(long tick) {
        return new Builder(tick);
    }

    public static final class Builder {
        private final long tick;
        private final Map<Integer, Map<UUID, ServerPlayer>> recipients = new HashMap<>();
        private final Map<Integer, Set<UUID>> interact = new HashMap<>();
        private final Map<Integer, Set<UUID>> input = new HashMap<>();

        private Builder(long tick) {
            this.tick = tick;
        }

        public Builder recipient(int displayId, ServerPlayer p) {
            recipients.computeIfAbsent(displayId, k -> new LinkedHashMap<>()).put(p.getUUID(), p);
            return this;
        }

        public Builder interact(int displayId, UUID p) {
            interact.computeIfAbsent(displayId, k -> new HashSet<>()).add(p);
            return this;
        }

        public Builder input(int displayId, UUID p) {
            input.computeIfAbsent(displayId, k -> new HashSet<>()).add(p);
            return this;
        }

        public ViewerSnapshot build() {
            Set<Integer> keys = new HashSet<>(recipients.keySet());
            keys.addAll(interact.keySet());
            keys.addAll(input.keySet());
            Map<Integer, Display> out = new HashMap<>();
            for (int k : keys) {
                out.put(k, new Display(Collections.unmodifiableMap(new LinkedHashMap<>(recipients.getOrDefault(k, Map.of()))),
                        Set.copyOf(interact.getOrDefault(k, Set.of())), Set.copyOf(input.getOrDefault(k, Set.of()))));
            }
            return new ViewerSnapshot(tick, Map.copyOf(out));
        }
    }
}
