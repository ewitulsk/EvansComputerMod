package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.CustomData;

import java.util.HexFormat;
import java.util.UUID;

/**
 * What AirPods and their case remember, on the items (custom data): the AirPods'
 * id, the phone they are paired with and the link key, their battery; the case's
 * AirPods (inside or not) and when its lid closes.
 */
public final class AirPodsData {
    private static final String PODS = "ecm_airpods", CASE = "ecm_airpods_case";

    private AirPodsData() {}

    /** The AirPods' state. */
    public record Pods(UUID id, long pairedTo, byte[] key, String phoneName, double battery) {
        public static Pods fresh() {
            return new Pods(UUID.randomUUID(), 0, null, "", 100);
        }

        public long address() {
            return BtAddress.of(id);
        }

        public Pods paired(long phone, byte[] k, String name) {
            return new Pods(id, phone, k, name, battery);
        }

        public Pods withBattery(double b) {
            return new Pods(id, pairedTo, key, phoneName, Math.max(0, Math.min(100, b)));
        }

        CompoundTag tag() {
            CompoundTag t = new CompoundTag();
            t.putUUID("id", id);
            if (pairedTo != 0) {
                t.putString("paired", BtAddress.format(pairedTo));
                t.putString("key", HexFormat.of().formatHex(key));
                t.putString("phone", phoneName);
            }
            t.putDouble("battery", battery);
            return t;
        }

        static Pods of(CompoundTag t) {
            if (t == null || !t.hasUUID("id")) return null;
            long paired = 0;
            byte[] key = null;
            if (t.contains("paired")) {
                try {
                    key = HexFormat.of().parseHex(t.getString("key"));
                    paired = Math.max(0, BtAddress.parse(t.getString("paired")));
                } catch (IllegalArgumentException e) {
                    paired = 0;
                }
            }
            return new Pods(t.getUUID("id"), paired, key, t.getString("phone"), t.contains("battery") ? t.getDouble("battery") : 100);
        }
    }

    /** The AirPods' state, creating an id the first time. */
    public static Pods pods(ItemStack stack) {
        Pods p = Pods.of(stack.getOrDefault(DataComponents.CUSTOM_DATA, CustomData.EMPTY).copyTag().getCompound(PODS));
        if (p == null) {
            p = Pods.fresh();
            setPods(stack, p);
        }
        return p;
    }

    /** The AirPods' state if it was ever set (tooltips: no id is made client-side). */
    public static Pods peekPods(ItemStack stack) {
        return Pods.of(stack.getOrDefault(DataComponents.CUSTOM_DATA, CustomData.EMPTY).copyTag().getCompound(PODS));
    }

    public static void setPods(ItemStack stack, Pods p) {
        CustomData.update(DataComponents.CUSTOM_DATA, stack, root -> root.put(PODS, p.tag()));
    }

    // ------------------------------------------------------------------ the case

    private static CompoundTag caseTag(ItemStack stack) {
        return stack.getOrDefault(DataComponents.CUSTOM_DATA, CustomData.EMPTY).copyTag().getCompound(CASE);
    }

    /** The AirPods in the case, or null when it is empty. */
    public static Pods inCase(ItemStack caseStack) {
        CompoundTag t = caseTag(caseStack);
        return t.contains("pods") ? Pods.of(t.getCompound("pods")) : null;
    }

    public static void setInCase(ItemStack caseStack, Pods p) {
        CustomData.update(DataComponents.CUSTOM_DATA, caseStack, root -> {
            CompoundTag t = root.getCompound(CASE);
            if (p == null) t.remove("pods");
            else t.put("pods", p.tag());
            root.put(CASE, t);
        });
    }

    /** Game time the lid closes by itself (0: closed). */
    public static long lidUntil(ItemStack caseStack) {
        return caseTag(caseStack).getLong("lid_until");
    }

    public static void setLidUntil(ItemStack caseStack, long gameTime) {
        CustomData.update(DataComponents.CUSTOM_DATA, caseStack, root -> {
            CompoundTag t = root.getCompound(CASE);
            t.putLong("lid_until", gameTime);
            root.put(CASE, t);
        });
    }
}
//?}
