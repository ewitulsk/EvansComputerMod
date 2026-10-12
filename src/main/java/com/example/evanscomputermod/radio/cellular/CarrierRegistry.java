package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.saveddata.SavedData;
import org.jetbrains.annotations.Nullable;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Every carrier on the server and its subscriber table (the HSS/HLR of a real
 * network), saved with the overworld. Carriers are players: whoever first names
 * a carrier at a tower owns it, issues its SIMs and sets their data limits.
 *
 * <p>It is also the {@link TowerLink.Core} every tower asks: is this IMSI one of
 * the tower's carrier's SIMs, what is its key, has it data left. It keeps which
 * tower serves which IMSI (not saved), so attaching to one tower releases the
 * device from the one before (handover).
 *
 * <p>IMSIs are 15 digits: 310 (test MCC), a 3-digit carrier number and a
 * 9-digit serial, so every SIM on the server is unique.
 */
public final class CarrierRegistry extends SavedData implements TowerLink.Core {
    private static final String NAME = "evanscomputermod_carriers";
    static final long IMSI_BASE = 310_000_000_000_000L;
    public static final long NO_QUOTA = -1;

    /** A carrier: owner and subscribers. */
    public static final class Carrier {
        public final String id;
        public UUID owner;
        public String ownerName;
        public final int number;
        long nextSerial = 1;
        final Map<Long, Subscriber> subs = new LinkedHashMap<>();

        Carrier(String id, UUID owner, String ownerName, int number) {
            this.id = id;
            this.owner = owner;
            this.ownerName = ownerName;
            this.number = number;
        }

        public List<Subscriber> subscribers() {
            List<Subscriber> l = new ArrayList<>(subs.values());
            l.sort(Comparator.comparingLong(s -> s.imsi));
            return l;
        }

        public Subscriber subscriber(long imsi) {
            return subs.get(imsi);
        }
    }

    /** One issued SIM. */
    public static final class Subscriber {
        public final long imsi;
        final String keyHex;
        public String label;
        public long quotaBytes = NO_QUOTA;
        public long usedBytes;
        public boolean revoked;

        Subscriber(long imsi, String keyHex, String label) {
            this.imsi = imsi;
            this.keyHex = keyHex;
            this.label = label;
        }

        public boolean overQuota() {
            return quotaBytes >= 0 && usedBytes >= quotaBytes;
        }
    }

    private final Map<String, Carrier> carriers = new LinkedHashMap<>();
    private int nextNumber = 1;
    /** Which tower serves each IMSI right now (live only). */
    private final Map<Long, TowerLink> serving = new ConcurrentHashMap<>();
    private final SecureRandom rng = new SecureRandom();

    /** The server's registry (one per world, on the overworld). */
    public static CarrierRegistry get(MinecraftServer server) {
        return server.overworld().getDataStorage().computeIfAbsent(
                new SavedData.Factory<>(CarrierRegistry::new, CarrierRegistry::load, null), NAME);
    }

    // ------------------------------------------------------------ carriers

    public @Nullable Carrier carrier(String id) {
        return id == null ? null : carriers.get(id);
    }

    public List<Carrier> carriers() {
        return List.copyOf(carriers.values());
    }

    /** May {@code p} run carrier {@code c} (issue SIMs, set limits, put towers on it)? */
    public static boolean mayManage(Carrier c, Player p) {
        return c == null || c.owner == null || c.owner.equals(p.getUUID()) || p.hasPermissions(2) || p.isCreative();
    }

    /**
     * The carrier named {@code id} for a player who wants to use it: created (owned by
     * them) if new; null if someone else owns it and the player may not manage it.
     */
    public @Nullable Carrier claim(String id, Player p) {
        Carrier c = carriers.get(id);
        if (c == null) {
            if (nextNumber > 999) return null;
            c = new Carrier(id, p.getUUID(), p.getGameProfile().getName(), nextNumber++);
            carriers.put(id, c);
            setDirty();
            return c;
        }
        return mayManage(c, p) ? c : null;
    }

    /** Issue a new SIM of {@code carrier}. */
    public SimCard issue(Carrier c, String label) {
        long imsi = IMSI_BASE + c.number * 1_000_000_000L + c.nextSerial++;
        String key = SimCard.newKeyHex(rng);
        c.subs.put(imsi, new Subscriber(imsi, key, label == null || label.isBlank() ? "SIM " + (imsi % 1_000_000_000L) : label));
        setDirty();
        return new SimCard(c.id, imsi, key);
    }

    /** Set a SIM's data limit in bytes ({@link #NO_QUOTA} = unlimited); usage is kept. Returns false if no such SIM. */
    public boolean setQuota(Carrier c, long imsi, long bytes) {
        Subscriber s = c.subs.get(imsi);
        if (s == null) return false;
        s.quotaBytes = bytes < 0 ? NO_QUOTA : bytes;
        setDirty();
        return true;
    }

    /** Zero a SIM's usage counter (a new billing period). */
    public boolean resetUsage(Carrier c, long imsi) {
        Subscriber s = c.subs.get(imsi);
        if (s == null) return false;
        s.usedBytes = 0;
        setDirty();
        return true;
    }

    public boolean setRevoked(Carrier c, long imsi, boolean revoked) {
        Subscriber s = c.subs.get(imsi);
        if (s == null) return false;
        s.revoked = revoked;
        setDirty();
        if (revoked) {
            TowerLink t = serving.remove(imsi);
            if (t != null) t.dropDevice(imsi);
        }
        return true;
    }

    /** The tower serving {@code imsi} now, or null. */
    public @Nullable TowerLink servingTower(long imsi) {
        return serving.get(imsi);
    }

    // ------------------------------------------------------------ TowerLink.Core

    @Override
    public Decision authorize(String carrier, long imsi) {
        Carrier c = carriers.get(carrier);
        if (c == null) return Decision.reject(CellFrame.Cause.UNKNOWN_SIM);
        Subscriber s = c.subs.get(imsi);
        if (s == null) {
            for (Carrier other : carriers.values())
                if (other.subs.containsKey(imsi)) return Decision.reject(CellFrame.Cause.WRONG_CARRIER);
            return Decision.reject(CellFrame.Cause.UNKNOWN_SIM);
        }
        if (s.revoked) return Decision.reject(CellFrame.Cause.REVOKED);
        if (s.overQuota()) return Decision.reject(CellFrame.Cause.QUOTA);
        return new Decision(java.util.HexFormat.of().parseHex(s.keyHex), CellFrame.Cause.NONE);
    }

    @Override
    public void attached(TowerLink tower, long imsi) {
        TowerLink before = serving.put(imsi, tower);
        if (before != null && before != tower) before.dropDevice(imsi);
    }

    @Override
    public void detached(TowerLink tower, long imsi) {
        serving.remove(imsi, tower);
    }

    @Override
    public boolean account(String carrier, long imsi, long bytes) {
        Carrier c = carriers.get(carrier);
        Subscriber s = c == null ? null : c.subs.get(imsi);
        if (s == null) return false;
        s.usedBytes += bytes;
        setDirty();
        return !s.overQuota();
    }

    // ------------------------------------------------------------ NBT

    private static CarrierRegistry load(CompoundTag tag, HolderLookup.Provider registries) {
        CarrierRegistry r = new CarrierRegistry();
        r.nextNumber = Math.max(1, tag.getInt("NextNumber"));
        ListTag list = tag.getList("Carriers", Tag.TAG_COMPOUND);
        for (int i = 0; i < list.size(); i++) {
            CompoundTag t = list.getCompound(i);
            String id = SimCard.normaliseCarrier(t.getString("Id"));
            if (id == null) continue;
            Carrier c = new Carrier(id, t.hasUUID("Owner") ? t.getUUID("Owner") : null, t.getString("OwnerName"), t.getInt("Number"));
            c.nextSerial = Math.max(1, t.getLong("NextSerial"));
            ListTag subs = t.getList("Sims", Tag.TAG_COMPOUND);
            for (int j = 0; j < subs.size(); j++) {
                CompoundTag s = subs.getCompound(j);
                Subscriber sub = new Subscriber(s.getLong("Imsi"), s.getString("Key"), s.getString("Label"));
                sub.quotaBytes = s.contains("Quota") ? s.getLong("Quota") : NO_QUOTA;
                sub.usedBytes = s.getLong("Used");
                sub.revoked = s.getBoolean("Revoked");
                c.subs.put(sub.imsi, sub);
            }
            r.carriers.put(id, c);
            r.nextNumber = Math.max(r.nextNumber, c.number + 1);
        }
        return r;
    }

    @Override
    public CompoundTag save(CompoundTag tag, HolderLookup.Provider registries) {
        tag.putInt("NextNumber", nextNumber);
        ListTag list = new ListTag();
        for (Carrier c : carriers.values()) {
            CompoundTag t = new CompoundTag();
            t.putString("Id", c.id);
            if (c.owner != null) t.putUUID("Owner", c.owner);
            t.putString("OwnerName", c.ownerName == null ? "" : c.ownerName);
            t.putInt("Number", c.number);
            t.putLong("NextSerial", c.nextSerial);
            ListTag subs = new ListTag();
            for (Subscriber s : c.subs.values()) {
                CompoundTag st = new CompoundTag();
                st.putLong("Imsi", s.imsi);
                st.putString("Key", s.keyHex);
                st.putString("Label", s.label);
                st.putLong("Quota", s.quotaBytes);
                st.putLong("Used", s.usedBytes);
                st.putBoolean("Revoked", s.revoked);
                subs.add(st);
            }
            t.put("Sims", subs);
            list.add(t);
        }
        tag.put("Carriers", list);
        return tag;
    }
}
//?}
