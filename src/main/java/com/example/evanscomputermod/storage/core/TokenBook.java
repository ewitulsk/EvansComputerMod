package com.example.evanscomputermod.storage.core;

//? if <=1.21.1 {

import org.jetbrains.annotations.Nullable;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.IntConsumer;

/**
 * Bearer tokens: items taken out of a cell and represented by a random,
 * single-use id. The book is the only authority: redeeming deletes the entry,
 * so a copied id is worthless after the first redeem, and ids from another
 * world never match.
 *
 * <p>Every token has an issuer (the drive or module whose cell it came from)
 * and an expiry. An expired token goes back to its origin cell, or to the
 * issuer's lost &amp; found when that cell is gone or full. Tokens and lost
 * items both count toward the issuer's caps, so neither is free storage.
 */
public final class TokenBook {

    public static final String PREFIX = "ecmt1_";

    public record Token(String id, int key, long count, UUID origin, UUID issuer, @Nullable UUID boundTo,
                        long expiresAt) {
        Token withCount(String newId, long newCount) {
            return new Token(newId, key, newCount, origin, issuer, boundTo, expiresAt);
        }
    }

    /** Cell lookup for refunds and redeems. */
    public interface Cells {
        /** Contents of a cell that still exists, or null. */
        @Nullable CellContents get(UUID cell);

        /** A cell's contents changed. */
        void changed(UUID cell);
    }

    /** Per-issuer limits. */
    public record Caps(int maxTokens, long maxItems) {
    }

    private final Map<String, Token> tokens = new LinkedHashMap<>();
    private final Map<UUID, LinkedHashMap<Integer, Long>> lost = new LinkedHashMap<>();
    private final SecureRandom random = new SecureRandom();

    // ------------------------------------------------------------ queries

    @Nullable
    public Token get(String id) {
        return tokens.get(id);
    }

    public Collection<Token> all() {
        return Collections.unmodifiableCollection(tokens.values());
    }

    public List<Token> issuedBy(UUID issuer) {
        List<Token> out = new ArrayList<>();
        for (Token t : tokens.values()) if (t.issuer.equals(issuer)) out.add(t);
        return out;
    }

    public Map<Integer, Long> lostOf(UUID issuer) {
        Map<Integer, Long> m = lost.get(issuer);
        return m == null ? Map.of() : Collections.unmodifiableMap(m);
    }

    public Map<UUID, LinkedHashMap<Integer, Long>> lostAll() {
        return Collections.unmodifiableMap(lost);
    }

    public int tokenCount(UUID issuer) {
        int n = 0;
        for (Token t : tokens.values()) if (t.issuer.equals(issuer)) n++;
        return n;
    }

    /** Items the issuer has out: unspent tokens plus lost &amp; found. */
    public long inFlight(UUID issuer) {
        long n = 0;
        for (Token t : tokens.values()) if (t.issuer.equals(issuer)) n += t.count;
        Map<Integer, Long> l = lost.get(issuer);
        if (l != null) for (long c : l.values()) n += c;
        return n;
    }

    /** Every item key a token or lost entry refers to (so the type table keeps them). */
    public void forEachKey(IntConsumer sink) {
        for (Token t : tokens.values()) sink.accept(t.key);
        for (Map<Integer, Long> m : lost.values()) for (int k : m.keySet()) sink.accept(k);
    }

    // ------------------------------------------------------------ operations

    /** Debit {@code count} of {@code key} from {@code origin} and issue a token for them. */
    public Token issue(Cells cells, UUID origin, UUID issuer, int key, long count, @Nullable UUID boundTo,
                       long now, long ttlTicks, Caps caps) throws StorageException {
        if (count <= 0) throw new StorageException("count must be positive");
        if (ttlTicks <= 0) throw new StorageException("ttl must be positive");
        if (tokenCount(issuer) + 1 > caps.maxTokens) {
            throw new StorageException("too many unspent tokens from this drive (max " + caps.maxTokens + ")");
        }
        if (inFlight(issuer) + count > caps.maxItems) {
            throw new StorageException("too many items in flight from this drive (max " + caps.maxItems + ")");
        }
        CellContents c = cells.get(origin);
        if (c == null) throw new StorageException("no such cell");
        c.remove(key, count);
        cells.changed(origin);
        Token t = new Token(newId(), key, count, origin, issuer, boundTo, now + ttlTicks);
        tokens.put(t.id, t);
        return t;
    }

    /** Spend a token into {@code target}. Fails (token kept) if it's bound elsewhere or the cell lacks room. */
    public Token redeem(Cells cells, String id, UUID target) throws StorageException {
        Token t = tokens.get(id);
        if (t == null) throw new StorageException("unknown or already spent token");
        if (t.boundTo != null && !t.boundTo.equals(target)) {
            throw new StorageException("token is addressed to another cell");
        }
        CellContents c = cells.get(target);
        if (c == null) throw new StorageException("no such cell");
        c.add(t.key, t.count);
        tokens.remove(id);
        cells.changed(target);
        return t;
    }

    /** Replace a token by several whose counts add up to it. */
    public List<Token> split(String id, long[] amounts, Caps caps) throws StorageException {
        Token t = tokens.get(id);
        if (t == null) throw new StorageException("unknown or already spent token");
        if (amounts.length < 2) throw new StorageException("split needs at least two amounts");
        long sum = 0;
        for (long a : amounts) {
            if (a <= 0) throw new StorageException("amounts must be positive");
            sum += a;
        }
        if (sum != t.count) throw new StorageException("amounts add up to " + sum + ", token holds " + t.count);
        if (tokenCount(t.issuer) - 1 + amounts.length > caps.maxTokens) {
            throw new StorageException("too many unspent tokens from this drive (max " + caps.maxTokens + ")");
        }
        tokens.remove(id);
        List<Token> out = new ArrayList<>();
        for (long a : amounts) {
            Token n = t.withCount(newId(), a);
            tokens.put(n.id, n);
            out.add(n);
        }
        return out;
    }

    /** Join tokens of the same item, issuer and binding into one; it keeps the earliest expiry. */
    public Token merge(List<String> ids) throws StorageException {
        if (ids.size() < 2) throw new StorageException("merge needs at least two tokens");
        List<Token> ts = new ArrayList<>();
        for (String id : ids) {
            Token t = tokens.get(id);
            if (t == null) throw new StorageException("unknown or already spent token: " + id);
            if (ts.contains(t)) throw new StorageException("token listed twice: " + id);
            ts.add(t);
        }
        Token first = ts.get(0);
        long total = 0;
        long expiry = Long.MAX_VALUE;
        for (Token t : ts) {
            if (t.key != first.key) throw new StorageException("tokens hold different items");
            if (!t.issuer.equals(first.issuer)) throw new StorageException("tokens come from different drives");
            if (!java.util.Objects.equals(t.boundTo, first.boundTo)) throw new StorageException("tokens have different addresses");
            total += t.count;
            expiry = Math.min(expiry, t.expiresAt);
        }
        for (Token t : ts) tokens.remove(t.id);
        Token merged = new Token(newId(), first.key, total, first.origin, first.issuer, first.boundTo, expiry);
        tokens.put(merged.id, merged);
        return merged;
    }

    /** What happened to one expired token. */
    public record Expired(Token token, boolean refunded) {
    }

    /** Return expired tokens' items to their origin cells (or lost &amp; found). */
    public List<Expired> expire(Cells cells, long now) {
        List<Expired> out = new ArrayList<>();
        var it = tokens.values().iterator();
        while (it.hasNext()) {
            Token t = it.next();
            if (t.expiresAt > now) continue;
            it.remove();
            CellContents c = cells.get(t.origin);
            boolean refunded = false;
            if (c != null && c.maxInsert(t.key) >= t.count) {
                try {
                    c.add(t.key, t.count);
                    cells.changed(t.origin);
                    refunded = true;
                } catch (StorageException ignored) {
                    // checked above
                }
            }
            if (!refunded) lost.computeIfAbsent(t.issuer, k -> new LinkedHashMap<>()).merge(t.key, t.count, Long::sum);
            out.add(new Expired(t, refunded));
        }
        return out;
    }

    /** Move an issuer's lost items into {@code targets} (in order) as far as they fit. Returns items moved. */
    public long claimLost(Cells cells, UUID issuer, List<UUID> targets) {
        LinkedHashMap<Integer, Long> l = lost.get(issuer);
        if (l == null) return 0;
        long moved = 0;
        var it = l.entrySet().iterator();
        while (it.hasNext()) {
            var e = it.next();
            long left = e.getValue();
            for (UUID target : targets) {
                if (left <= 0) break;
                CellContents c = cells.get(target);
                if (c == null) continue;
                long n = Math.min(left, c.maxInsert(e.getKey()));
                if (n <= 0) continue;
                try {
                    c.add(e.getKey(), n);
                } catch (StorageException ex) {
                    continue;
                }
                cells.changed(target);
                left -= n;
                moved += n;
            }
            if (left <= 0) it.remove();
            else e.setValue(left);
        }
        if (l.isEmpty()) lost.remove(issuer);
        return moved;
    }

    // ------------------------------------------------------------ persistence

    /** Restore a saved token (no checks). */
    public void putUnchecked(Token t) {
        tokens.put(t.id, t);
    }

    public void putLostUnchecked(UUID issuer, int key, long count) {
        if (count > 0) lost.computeIfAbsent(issuer, k -> new LinkedHashMap<>()).merge(key, count, Long::sum);
    }

    private String newId() {
        byte[] b = new byte[16];
        String id;
        do {
            random.nextBytes(b);
            id = PREFIX + HexFormat.of().formatHex(b);
        } while (tokens.containsKey(id));
        return id;
    }
}
//?}
