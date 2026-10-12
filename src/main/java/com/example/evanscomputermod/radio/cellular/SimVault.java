package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.saveddata.SavedData;
import net.neoforged.neoforge.server.ServerLifecycleHooks;
import org.jetbrains.annotations.Nullable;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HashMap;
import java.util.HexFormat;
import java.util.Map;

/**
 * Server-side keys of SIMs sitting in a v1 modem's own slot. The modem's state is
 * custom data on its item stack, which is synced to clients (in an inventory and in the
 * Terminal's update tag), so it keeps only the SIM's identity and a reference here:
 * {@code ref = hex(SHA-256("ecm-sim-vault" | carrier | imsi | key))[0..32)}. Entries are
 * idempotent and never removed (a few dozen bytes per SIM ever fitted). Goes away with
 * the v1 modem engine (CELLULAR_V2.md, phase 4).
 */
public final class SimVault extends SavedData {
    static final String NAME = "evanscomputermod_sim_vault";
    private final Map<String, String> keys = new HashMap<>();

    private static @Nullable SimVault get() {
        MinecraftServer server = ServerLifecycleHooks.getCurrentServer();
        if (server == null) return null;
        return server.overworld().getDataStorage().computeIfAbsent(
                new SavedData.Factory<>(SimVault::new, SimVault::load, null), NAME);
    }

    static String ref(SimCard s) {
        try {
            MessageDigest d = MessageDigest.getInstance("SHA-256");
            d.update(("ecm-sim-vault|" + s.carrier() + "|" + s.imsi() + "|" + s.keyHex()).getBytes(StandardCharsets.US_ASCII));
            return HexFormat.of().formatHex(d.digest()).substring(0, 32);
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Store {@code s}'s key; returns its reference, or null without a server. */
    static @Nullable String put(SimCard s) {
        SimVault v = get();
        if (v == null) return null;
        String ref = ref(s);
        if (!s.keyHex().equals(v.keys.put(ref, s.keyHex()))) v.setDirty();
        return ref;
    }

    static @Nullable String key(String ref) {
        SimVault v = get();
        return v == null || ref == null ? null : v.keys.get(ref);
    }

    private static SimVault load(CompoundTag tag, HolderLookup.Provider registries) {
        SimVault v = new SimVault();
        CompoundTag k = tag.getCompound("keys");
        for (String ref : k.getAllKeys()) v.keys.put(ref, k.getString(ref));
        return v;
    }

    @Override
    public CompoundTag save(CompoundTag tag, HolderLookup.Provider registries) {
        CompoundTag k = new CompoundTag();
        keys.forEach(k::putString);
        tag.put("keys", k);
        return tag;
    }
}
//?}
