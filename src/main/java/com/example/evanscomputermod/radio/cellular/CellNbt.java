package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.SimIdentity;
import net.minecraft.nbt.CompoundTag;

/**
 * SIM cards in NBT (a modem's saved state keeps its SIM this way). The key is not
 * written here: the state travels to clients, so it holds a {@link SimVault}
 * reference and the key stays on the server.
 */
public final class CellNbt {
    private CellNbt() {}

    public static CompoundTag writeSim(SimCard s) {
        CompoundTag t = new CompoundTag();
        t.putString("carrier", s.carrier());
        t.putLong("imsi", s.imsi());
        String ref = SimVault.put(s);
        if (ref != null) t.putString("vault", ref);
        return t;
    }

    /** The SIM in {@code t} with its key (server only), or null if it holds none or the key is unknown. */
    public static SimCard readSim(CompoundTag t) {
        if (t == null || !t.contains("carrier")) return null;
        String key = t.contains("key") ? t.getString("key") : SimVault.key(t.getString("vault"));
        if (key == null) return null;
        try {
            return new SimCard(t.getString("carrier"), t.getLong("imsi"), key);
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    /** Only carrier and IMSI (what a client can know), or null. */
    public static SimIdentity readIdentity(CompoundTag t) {
        if (t == null || !t.contains("carrier")) return null;
        try {
            return new SimIdentity(t.getString("carrier"), t.getLong("imsi"));
        } catch (IllegalArgumentException e) {
            return null;
        }
    }
}
//?}
