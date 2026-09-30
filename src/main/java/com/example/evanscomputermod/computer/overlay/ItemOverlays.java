package com.example.evanscomputermod.computer.overlay;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.ledger.ItemTypes;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import it.unimi.dsi.fastutil.ints.IntOpenHashSet;
import it.unimi.dsi.fastutil.objects.Object2IntOpenCustomHashMap;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.ItemStackLinkedSet;
import net.minecraft.world.item.Items;
import net.neoforged.neoforge.network.PacketDistributor;
import org.jetbrains.annotations.Nullable;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Item overlays of one computer: per display (the terminal's framebuffer and
 * its Screen cluster), a list of items to draw on top of the pixels. Programs
 * set a whole list at once ({@code gfx_items_set}); the Minecraft client
 * draws each item with its own item renderer, so every mod's items look right
 * (3D blocks, enchantment glint, custom renderers) without the program ever
 * seeing a texture.
 *
 * <p>An item is named either by a storage key ({@code "k3f"}) or by an item
 * id ({@code "minecraft:diamond"}). Overlays are display only.
 */
public final class ItemOverlays {

    public static final int TARGET_TERMINAL = 0;
    public static final int TARGET_SCREEN = 1;
    public static final int MAX_ENTRIES = 512;
    private static final int MAX_REF = 96;
    private static final int MAX_LABEL = 16;

    /** Entry flags (same bits as ecm-ui's ITEM_FLAG_*). */
    public static final int FLAG_SELECTED = 0x01, FLAG_HOVERED = 0x02, FLAG_DIMMED = 0x80;

    /** One placed item, in framebuffer pixels; {@code proto} indexes the prototype table. */
    public record Entry(int x, int y, int size, int clipX, int clipY, int clipW, int clipH, int flags,
                        String label, int proto) {
    }

    private static final class Layer {
        volatile List<Entry> entries = List.of();
        volatile long generation;
        volatile int ownerPid = -1;
        final Map<UUID, Long> sent = new ConcurrentHashMap<>();
        final Map<UUID, IntOpenHashSet> sentProtos = new ConcurrentHashMap<>();
    }

    private final Layer[] layers = {new Layer(), new Layer()};
    private final Map<Integer, ItemStack> protos = new HashMap<>();
    private final Object2IntOpenCustomHashMap<ItemStack> protoIds =
            new Object2IntOpenCustomHashMap<>(ItemStackLinkedSet.TYPE_AND_TAG);
    private int nextProto = 1;

    public List<Entry> entries(int target) {
        return layers[target].entries;
    }

    public long generation(int target) {
        return layers[target].generation;
    }

    /**
     * Replace a display's overlay with the list encoded in {@code data}.
     * Items that can't be resolved are skipped. Returns the number of entries
     * kept, or -1 if the list is malformed. Any thread.
     */
    public int set(int target, byte[] data, int pid) {
        if (target != TARGET_TERMINAL && target != TARGET_SCREEN) return -1;
        List<Entry> out = new ArrayList<>();
        try {
            ByteBuffer b = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
            int count = Short.toUnsignedInt(b.getShort());
            if (count > MAX_ENTRIES) return -1;
            for (int i = 0; i < count; i++) {
                int x = b.getShort(), y = b.getShort(), size = Short.toUnsignedInt(b.getShort());
                int cx = b.getShort(), cy = b.getShort();
                int cw = Short.toUnsignedInt(b.getShort()), ch = Short.toUnsignedInt(b.getShort());
                int flags = b.get() & 0xFF;
                String ref = string(b, MAX_REF);
                String label = string(b, MAX_LABEL);
                if (size <= 0 || size > 1024) continue;
                ItemStack stack = resolve(ref);
                if (stack.isEmpty()) continue;
                out.add(new Entry(x, y, size, cx, cy, cw, ch, flags, label, intern(stack)));
            }
        } catch (RuntimeException e) {
            return -1;
        }
        Layer l = layers[target];
        l.entries = List.copyOf(out);
        l.ownerPid = out.isEmpty() ? -1 : pid;
        l.generation++;
        return out.size();
    }

    public void clear(int target) {
        Layer l = layers[target];
        if (l.entries.isEmpty()) return;
        l.entries = List.of();
        l.ownerPid = -1;
        l.generation++;
    }

    /** A program exited: drop the overlays it set. Returns true if anything was cleared. */
    public boolean clearOwnedBy(int pid) {
        boolean any = false;
        for (int t = 0; t < layers.length; t++) {
            if (layers[t].ownerPid == pid && pid >= 0) {
                clear(t);
                any = true;
            }
        }
        return any;
    }

    private static String string(ByteBuffer b, int max) {
        int n = b.get() & 0xFF;
        if (n > max) throw new IllegalArgumentException("string too long");
        byte[] bytes = new byte[n];
        b.get(bytes);
        return new String(bytes, StandardCharsets.UTF_8);
    }

    /** A storage key or an item id to a stack (EMPTY if unknown). */
    public static ItemStack resolve(String ref) {
        int key = ItemTypes.parseKey(ref);
        if (key != 0) {
            StorageLedger ledger = StorageLedger.peek();
            ItemStack proto = ledger == null ? null : ledger.types().get(key);
            return proto == null ? ItemStack.EMPTY : proto.copyWithCount(1);
        }
        Identifier id = Identifier.tryParse(ref);
        if (id == null) return ItemStack.EMPTY;
        Item item = BuiltInRegistries.ITEM.getOptional(id).orElse(Items.AIR);
        return item == Items.AIR ? ItemStack.EMPTY : new ItemStack(item);
    }

    private synchronized int intern(ItemStack stack) {
        int id = protoIds.getInt(stack);
        if (id != 0) return id;
        id = nextProto++;
        protos.put(id, stack);
        protoIds.put(stack, id);
        return id;
    }

    @Nullable
    private synchronized ItemStack proto(int id) {
        return protos.get(id);
    }

    // ------------------------------------------------------------ sync (server thread)

    /** Send {@code player} the overlay of {@code target} if it changed since last time. */
    public void sync(ServerPlayer player, BlockPos pos, int target) {
        Layer l = layers[target];
        long gen = l.generation;
        Long sent = l.sent.get(player.getUUID());
        if (sent != null && sent == gen) return;
        List<Entry> entries = l.entries;
        IntOpenHashSet known = l.sentProtos.computeIfAbsent(player.getUUID(), k -> new IntOpenHashSet());
        Map<Integer, ItemStack> fresh = new HashMap<>();
        for (Entry e : entries) {
            if (known.contains(e.proto())) continue;
            ItemStack s = proto(e.proto());
            if (s != null) fresh.put(e.proto(), s);
        }
        PacketDistributor.sendToPlayer(player, new ItemOverlayPacket(pos, (byte) target, gen, entries, fresh));
        known.addAll(fresh.keySet());
        l.sent.put(player.getUUID(), gen);
    }

    /** Forget players no longer receiving this display (they get a full send if they come back). */
    public void retain(int target, Collection<UUID> players) {
        Layer l = layers[target];
        l.sent.keySet().retainAll(players);
        l.sentProtos.keySet().retainAll(players);
    }

    public void forget(UUID player, int target) {
        layers[target].sent.remove(player);
        layers[target].sentProtos.remove(player);
    }
}
//?}
