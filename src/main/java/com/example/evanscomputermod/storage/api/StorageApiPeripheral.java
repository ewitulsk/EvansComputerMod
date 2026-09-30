package com.example.evanscomputermod.storage.api;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.storage.StorageConfig;
import com.example.evanscomputermod.storage.StorageRules;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.core.StorageException;
import com.example.evanscomputermod.storage.core.TokenBook;
import com.example.evanscomputermod.storage.device.DecoderBlockEntity;
import com.example.evanscomputermod.storage.device.IItemPort;
import com.example.evanscomputermod.storage.device.IStorageDevice;
import com.example.evanscomputermod.storage.device.MountedCell;
import com.example.evanscomputermod.storage.ledger.ItemTypes;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.enchantment.Enchantment;
import net.minecraft.world.item.enchantment.ItemEnchantments;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * The storage methods every storage peripheral offers (Drive, Storage Module,
 * Wired Bus Module). They act on the calling computer's whole
 * {@link StorageNet}, not just this peripheral, so a program can use any one
 * of them. All run on the server thread, one call at a time, and change the
 * ledger fully or not at all.
 *
 * <p>Items are named by key ({@code "k3f"}): one key per distinct item type
 * (item + components). Cells by id (a UUID; any unique 8+ character prefix
 * works).
 */
public abstract class StorageApiPeripheral extends AnnotatedPeripheral implements StorageNet.Node {

    private static final int MAX_LIST = 10_000;

    protected final Set<IComputerAccess> computers = new CopyOnWriteArraySet<>();

    @Nullable
    protected abstract MinecraftServer server();

    @Override
    public void attach(IComputerAccess computer) {
        computers.add(computer);
    }

    @Override
    public void detach(IComputerAccess computer) {
        computers.remove(computer);
    }

    public Collection<IComputerAccess> watchers() {
        return computers;
    }

    /** Tell attached computers which cells changed. */
    protected void postChanged(List<String> cells) {
        if (cells.isEmpty()) return;
        for (IComputerAccess c : computers) c.queueEvent("storage_changed", cells);
    }

    protected StorageNet net(IComputerAccess computer) throws StorageException {
        MinecraftServer server = server();
        if (server == null) throw new StorageException("storage is not loaded");
        return StorageNet.of(computer, server);
    }

    // ------------------------------------------------------------ inventory

    @PeripheralMethod(description = "Cells this computer can reach, with their fill")
    public List<Map<String, Object>> cells(IComputerAccess computer) throws PeripheralException {
        StorageNet net = net(computer);
        List<Map<String, Object>> out = new ArrayList<>();
        for (MountedCell c : net.cells()) {
            CellContents contents = net.ledger.get(c.id());
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", c.id().toString());
            m.put("short", c.shortId());
            m.put("tier", c.tier().label());
            m.put("device", c.device().kind());
            m.put("device_id", c.device().deviceId().toString());
            m.put("slot", c.slot());
            m.put("priority", c.device().priority());
            m.put("bytes_used", contents == null ? 0L : contents.usedBytes());
            m.put("bytes_total", c.tier().bytes());
            m.put("types_used", contents == null ? 0 : contents.types());
            m.put("types_total", CellTier.MAX_TYPES);
            m.put("items", contents == null ? 0L : contents.items());
            out.add(m);
        }
        return out;
    }

    @PeripheralMethod(description = "Items in reach: items(query, sort, offset, limit, cell). sort: count|name|id|mod")
    public List<Map<String, Object>> items(IComputerAccess computer, @Nullable String query, @Nullable String sort,
                                           @Nullable Integer offset, @Nullable Integer limit, @Nullable String cell)
            throws PeripheralException {
        StorageNet net = net(computer);
        List<Entry> all = matching(net, query, cell);
        sortEntries(all, sort);
        int from = Math.max(0, offset == null ? 0 : offset);
        int n = Math.min(MAX_LIST, limit == null || limit <= 0 ? MAX_LIST : limit);
        List<Map<String, Object>> out = new ArrayList<>();
        for (int i = from; i < all.size() && out.size() < n; i++) out.add(all.get(i).toMap());
        return out;
    }

    @PeripheralMethod(description = "Number of item types matching: item_count(query, cell)")
    public int itemCount(IComputerAccess computer, @Nullable String query, @Nullable String cell) throws PeripheralException {
        return matching(net(computer), query, cell).size();
    }

    @PeripheralMethod(description = "How many of an item (by key) are stored in reach")
    public long total(IComputerAccess computer, String key) throws PeripheralException {
        StorageNet net = net(computer);
        int k = key(key);
        long n = 0;
        for (MountedCell c : net.cells()) {
            CellContents contents = net.ledger.get(c.id());
            if (contents != null) n += contents.count(k);
        }
        return n;
    }

    @PeripheralMethod(description = "Keys of stored items with this item id, e.g. find('minecraft:iron_ingot')")
    public List<String> find(IComputerAccess computer, String itemId) throws PeripheralException {
        StorageNet net = net(computer);
        List<String> out = new ArrayList<>();
        for (Entry e : matching(net, null, null)) {
            if (e.id.equals(itemId)) out.add(ItemTypes.keyOf(e.key));
        }
        return out;
    }

    @PeripheralMethod(description = "Details of an item type: name, damage, enchantments, components")
    public Map<String, Object> itemDetail(IComputerAccess computer, String key) throws PeripheralException {
        StorageNet net = net(computer);
        int k = key(key);
        ItemStack proto = net.ledger.types().get(k);
        if (proto == null) throw new StorageException("unknown item key '" + key + "'");
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("key", key);
        m.put("id", StorageRules.itemId(proto));
        m.put("name", StorageRules.displayName(proto));
        m.put("count", total(computer, key));
        m.put("max_stack", proto.getMaxStackSize());
        if (proto.isDamageableItem()) {
            m.put("damage", proto.getDamageValue());
            m.put("max_damage", proto.getMaxDamage());
        }
        ItemEnchantments ench = proto.getEnchantments();
        if (!ench.isEmpty()) {
            Map<String, Object> e = new LinkedHashMap<>();
            for (var entry : ench.entrySet()) {
                Holder<Enchantment> h = entry.getKey();
                e.put(h.unwrapKey().map(r -> r.location().toString()).orElse("?"), entry.getIntValue());
            }
            m.put("enchantments", e);
        }
        List<String> comps = new ArrayList<>();
        for (var c : proto.getComponentsPatch().entrySet()) {
            var id = BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(c.getKey());
            if (id != null) comps.add(id.toString());
        }
        m.put("components", comps);
        return m;
    }

    @PeripheralMethod(description = "Storage devices and ports (encoders/decoders) in reach")
    public Map<String, Object> devices(IComputerAccess computer) throws PeripheralException {
        StorageNet net = net(computer);
        List<Map<String, Object>> devs = new ArrayList<>();
        for (IStorageDevice d : net.devices()) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", d.deviceId().toString());
            m.put("kind", d.kind());
            m.put("priority", d.priority());
            m.put("where", d.describe());
            List<String> cells = new ArrayList<>();
            for (MountedCell c : d.cells()) cells.add(c.id().toString());
            m.put("cells", cells);
            devs.add(m);
        }
        List<Map<String, Object>> ports = new ArrayList<>();
        for (var e : net.ports().entrySet()) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", e.getKey());
            m.put("kind", e.getValue().kind());
            m.put("id", e.getValue().deviceId().toString());
            ports.add(m);
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("devices", devs);
        out.put("ports", ports);
        return out;
    }

    // ------------------------------------------------------------ moving items

    @PeripheralMethod(description = "Move up to count of an item between two reachable cells; returns how many moved")
    public long move(IComputerAccess computer, String fromCell, String toCell, String key, long count)
            throws PeripheralException {
        if (count <= 0) throw new StorageException("count must be positive");
        StorageNet net = net(computer);
        MountedCell from = net.cell(fromCell);
        MountedCell to = net.cell(toCell);
        if (from.id().equals(to.id())) return 0;
        int k = key(key);
        CellContents a = net.contents(from);
        CellContents b = net.contents(to);
        long n = Math.min(count, Math.min(a.count(k), b.maxInsert(k)));
        if (n <= 0) return 0;
        a.remove(k, n);
        b.add(k, n);
        net.ledger.changed(from.id());
        net.ledger.changed(to.id());
        return n;
    }

    @PeripheralMethod(description = "Send up to count of an item out of a decoder: extract(key, count, decoder, cell)")
    public long extract(IComputerAccess computer, String key, long count, @Nullable String decoder, @Nullable String cell)
            throws PeripheralException {
        StorageNet net = net(computer);
        return extractTo(net, net.decoder(decoder), key(key), count, cell);
    }

    /** Debit up to {@code count} from the net's cells (lowest priority first) and deliver through {@code dec}. */
    public static long extractTo(StorageNet net, DecoderBlockEntity dec, int key, long count, @Nullable String cell)
            throws StorageException {
        if (count <= 0) throw new StorageException("count must be positive");
        ItemStack proto = net.ledger.types().get(key);
        if (proto == null) throw new StorageException("unknown item key '" + ItemTypes.keyOf(key) + "'");
        List<MountedCell> sources;
        if (cell != null) {
            sources = List.of(net.cell(cell));
        } else {
            sources = new ArrayList<>(net.cells());
            java.util.Collections.reverse(sources);
        }
        long available = 0;
        for (MountedCell c : sources) {
            CellContents contents = net.ledger.get(c.id());
            if (contents != null) available += contents.count(key);
        }
        long want = Math.min(count, available);
        if (want <= 0) return 0;
        long fits = dec.capacityFor(proto, want);
        if (fits <= 0) return 0;
        long left = fits;
        for (MountedCell c : sources) {
            if (left <= 0) break;
            CellContents contents = net.ledger.get(c.id());
            if (contents == null) continue;
            long n = Math.min(left, contents.count(key));
            if (n <= 0) continue;
            contents.remove(key, n);
            net.ledger.changed(c.id());
            left -= n;
        }
        long taken = fits - left;
        dec.deliver(proto, taken);
        return taken;
    }

    // ------------------------------------------------------------ tokens

    @PeripheralMethod(description = "Take items out as a single-use token: withdraw_token(cell, key, count, to_cell, ttl_seconds)")
    public String withdrawToken(IComputerAccess computer, String cell, String key, long count,
                                @Nullable String toCell, @Nullable Long ttl) throws PeripheralException {
        StorageNet net = net(computer);
        MountedCell from = net.cell(cell);
        UUID bound = null;
        if (toCell != null && !toCell.isEmpty()) {
            try {
                bound = UUID.fromString(toCell);
            } catch (IllegalArgumentException e) {
                throw new StorageException("to_cell must be a full cell id");
            }
        }
        long ttlTicks = ttl == null ? StorageConfig.tokenDefaultTtlTicks() : ttl * 20L;
        if (ttlTicks <= 0) throw new StorageException("ttl must be positive");
        ttlTicks = Math.min(ttlTicks, StorageConfig.tokenMaxTtlTicks());
        TokenBook.Token t = net.ledger.tokens().issue(net.ledger, from.id(), from.device().deviceId(), key(key), count,
                bound, net.ledger.now(), ttlTicks, caps());
        net.ledger.setDirty();
        return t.id();
    }

    @PeripheralMethod(description = "Spend a token into a cell (default: its address, else the first cell with room)")
    public Map<String, Object> redeem(IComputerAccess computer, String token, @Nullable String cell) throws PeripheralException {
        StorageNet net = net(computer);
        TokenBook.Token t = net.ledger.tokens().get(token);
        if (t == null) throw new StorageException("unknown or already spent token");
        MountedCell target;
        if (cell != null) {
            target = net.cell(cell);
        } else if (t.boundTo() != null) {
            target = net.cell(t.boundTo().toString());
        } else {
            target = null;
            for (MountedCell c : net.cells()) {
                CellContents contents = net.ledger.get(c.id());
                if (contents != null && contents.maxInsert(t.key()) >= t.count()) {
                    target = c;
                    break;
                }
            }
            if (target == null) throw new StorageException("no reachable cell has room for " + t.count() + " items");
        }
        net.ledger.tokens().redeem(net.ledger, token, target.id());
        net.ledger.setDirty();
        Map<String, Object> m = tokenMap(net.ledger, t, false);
        m.put("cell", target.id().toString());
        return m;
    }

    @PeripheralMethod(description = "What a token holds (without spending it)")
    public Map<String, Object> tokenInfo(IComputerAccess computer, String token) throws PeripheralException {
        StorageNet net = net(computer);
        TokenBook.Token t = net.ledger.tokens().get(token);
        if (t == null) throw new StorageException("unknown or already spent token");
        return tokenMap(net.ledger, t, false);
    }

    @PeripheralMethod(description = "Split a token into several whose counts add up to it; returns the new tokens")
    public List<String> split(IComputerAccess computer, String token, List<Object> amounts) throws PeripheralException {
        StorageNet net = net(computer);
        long[] a = new long[amounts.size()];
        for (int i = 0; i < a.length; i++) {
            if (!(amounts.get(i) instanceof Number n)) throw new StorageException("amounts must be integers");
            a[i] = n.longValue();
        }
        List<String> out = new ArrayList<>();
        for (TokenBook.Token t : net.ledger.tokens().split(token, a, caps())) out.add(t.id());
        net.ledger.setDirty();
        return out;
    }

    @PeripheralMethod(description = "Merge tokens of the same item and origin into one")
    public String merge(IComputerAccess computer, List<Object> tokens) throws PeripheralException {
        StorageNet net = net(computer);
        List<String> ids = new ArrayList<>();
        for (Object o : tokens) {
            if (!(o instanceof String s)) throw new StorageException("tokens must be strings");
            ids.add(s);
        }
        String id = net.ledger.tokens().merge(ids).id();
        net.ledger.setDirty();
        return id;
    }

    @PeripheralMethod(description = "Unspent tokens issued from cells in reach")
    public List<Map<String, Object>> tokensIssued(IComputerAccess computer) throws PeripheralException {
        StorageNet net = net(computer);
        List<Map<String, Object>> out = new ArrayList<>();
        for (IStorageDevice d : net.devices()) {
            for (TokenBook.Token t : net.ledger.tokens().issuedBy(d.deviceId())) out.add(tokenMap(net.ledger, t, true));
        }
        return out;
    }

    @PeripheralMethod(description = "Items from expired tokens that couldn't go back to their cell")
    public List<Map<String, Object>> lost(IComputerAccess computer) throws PeripheralException {
        StorageNet net = net(computer);
        List<Map<String, Object>> out = new ArrayList<>();
        for (IStorageDevice d : net.devices()) {
            for (var e : net.ledger.tokens().lostOf(d.deviceId()).entrySet()) {
                Map<String, Object> m = itemMap(net.ledger, e.getKey(), e.getValue());
                m.put("issuer", d.deviceId().toString());
                out.add(m);
            }
        }
        return out;
    }

    @PeripheralMethod(description = "Put lost items back into a cell (default: any with room); returns how many")
    public long claimLost(IComputerAccess computer, @Nullable String cell) throws PeripheralException {
        StorageNet net = net(computer);
        List<UUID> targets = new ArrayList<>();
        if (cell != null) targets.add(net.cell(cell).id());
        else for (MountedCell c : net.cells()) targets.add(c.id());
        long moved = 0;
        for (IStorageDevice d : net.devices()) moved += net.ledger.tokens().claimLost(net.ledger, d.deviceId(), targets);
        if (moved > 0) net.ledger.setDirty();
        return moved;
    }

    // ------------------------------------------------------------ helpers

    protected static TokenBook.Caps caps() {
        return new TokenBook.Caps(StorageConfig.maxTokensPerIssuer(), StorageConfig.maxItemsInFlight());
    }

    protected static int key(@Nullable String key) throws StorageException {
        int k = ItemTypes.parseKey(key);
        if (k == 0) throw new StorageException("bad item key '" + key + "' (keys look like 'k3f')");
        return k;
    }

    private static Map<String, Object> tokenMap(StorageLedger ledger, TokenBook.Token t, boolean withId) {
        Map<String, Object> m = itemMap(ledger, t.key(), t.count());
        if (withId) m.put("token", t.id());
        m.put("expires_in", Math.max(0, (t.expiresAt() - ledger.now()) / 20));
        m.put("bound", t.boundTo() == null ? null : t.boundTo().toString());
        m.put("issuer", t.issuer().toString());
        return m;
    }

    static Map<String, Object> itemMap(StorageLedger ledger, int key, long count) {
        Map<String, Object> m = new LinkedHashMap<>();
        ItemStack proto = ledger.types().get(key);
        m.put("key", ItemTypes.keyOf(key));
        m.put("id", proto == null ? "unknown" : StorageRules.itemId(proto));
        m.put("name", proto == null ? "Unknown item" : StorageRules.displayName(proto));
        m.put("count", count);
        return m;
    }

    private record Entry(int key, String id, String name, String mod, long count) {
        Map<String, Object> toMap() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("key", ItemTypes.keyOf(key));
            m.put("id", id);
            m.put("name", name);
            m.put("mod", mod);
            m.put("count", count);
            return m;
        }
    }

    private static List<Entry> matching(StorageNet net, @Nullable String query, @Nullable String cell) throws StorageException {
        Map<Integer, Long> totals = new LinkedHashMap<>();
        List<MountedCell> cells = cell == null ? net.cells() : List.of(net.cell(cell));
        for (MountedCell c : cells) {
            CellContents contents = net.ledger.get(c.id());
            if (contents == null) continue;
            for (var e : contents.view().entrySet()) totals.merge(e.getKey(), e.getValue(), Long::sum);
        }
        String q = query == null ? "" : query.trim().toLowerCase(Locale.ROOT);
        String mod = null;
        if (q.startsWith("@")) {
            int sp = q.indexOf(' ');
            mod = sp < 0 ? q.substring(1) : q.substring(1, sp);
            q = sp < 0 ? "" : q.substring(sp + 1).trim();
        }
        List<Entry> out = new ArrayList<>();
        for (var e : totals.entrySet()) {
            ItemStack proto = net.ledger.types().get(e.getKey());
            if (proto == null) continue;
            String id = StorageRules.itemId(proto);
            String ns = id.substring(0, id.indexOf(':'));
            String name = StorageRules.displayName(proto);
            if (mod != null && !ns.startsWith(mod)) continue;
            if (!q.isEmpty() && !id.contains(q) && !name.toLowerCase(Locale.ROOT).contains(q)) continue;
            out.add(new Entry(e.getKey(), id, name, ns, e.getValue()));
        }
        return out;
    }

    private static void sortEntries(List<Entry> list, @Nullable String sort) {
        Comparator<Entry> byName = Comparator.comparing(e -> e.name.toLowerCase(Locale.ROOT));
        Comparator<Entry> c = switch (sort == null ? "count" : sort) {
            case "name" -> byName;
            case "id" -> Comparator.comparing(e -> e.id);
            case "mod" -> Comparator.<Entry, String>comparing(e -> e.mod).thenComparing(byName);
            default -> Comparator.<Entry>comparingLong(e -> e.count).reversed().thenComparing(byName);
        };
        list.sort(c);
    }

    /** Ports in a net by kind (for subclasses' listings). */
    protected static List<IItemPort> portsOf(StorageNet net, String kind) {
        List<IItemPort> out = new ArrayList<>();
        for (IItemPort p : net.ports().values()) if (p.kind().equals(kind)) out.add(p);
        return out;
    }
}
//?}
