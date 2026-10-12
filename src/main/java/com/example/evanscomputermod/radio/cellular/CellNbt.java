package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import net.minecraft.nbt.CompoundTag;

/** SIM cards in NBT (a modem's saved state keeps its SIM this way). */
public final class CellNbt {
    private CellNbt() {}

    public static CompoundTag writeSim(SimCard s) {
        CompoundTag t = new CompoundTag();
        t.putString("carrier", s.carrier());
        t.putLong("imsi", s.imsi());
        t.putString("key", s.keyHex());
        return t;
    }

    /** The SIM in {@code t}, or null if it holds none (or a malformed one). */
    public static SimCard readSim(CompoundTag t) {
        if (t == null || !t.contains("carrier")) return null;
        try {
            return new SimCard(t.getString("carrier"), t.getLong("imsi"), t.getString("key"));
        } catch (IllegalArgumentException e) {
            return null;
        }
    }
}
//?}
