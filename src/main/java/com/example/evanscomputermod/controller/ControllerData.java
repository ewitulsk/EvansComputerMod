package com.example.evanscomputermod.controller;

import net.minecraft.core.BlockPos;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.CustomData;
import org.jetbrains.annotations.Nullable;

import java.util.EnumMap;
import java.util.Map;
import java.util.UUID;

/**
 * What a Wireless Controller item remembers, stored as NBT in the stack's
 * custom data under {@code ecm_controller}, so it travels with the item:
 *
 * <pre>
 * ecm_controller: {
 *   v: 1,
 *   id: "uuid",                       // this controller (given when first paired)
 *   computer: "uuid",                 // the paired computer
 *   pos: [x, y, z], dim: "minecraft:overworld",   // where it was paired (shown in the tooltip)
 *   bindings: { a: "key.keyboard.j", ls_up: "key.keyboard.w", guide: "", ... }
 * }
 * </pre>
 *
 * A missing binding uses its default; {@code ""} means deliberately unbound.
 */
public final class ControllerData {

    public static final String KEY = "ecm_controller";
    private static final int VERSION = 1;
    /** Longest key name accepted (Minecraft's are well under this). */
    public static final int MAX_KEY_NAME = 64;

    private ControllerData() {}

    private static CompoundTag root(ItemStack stack) {
        CustomData data = stack.get(DataComponents.CUSTOM_DATA);
        if (data == null) return new CompoundTag();
        CompoundTag tag = data.copyTag();
        //? if >=26.1 {
        return tag.getCompoundOrEmpty(KEY);
        //?} else
        /*return tag.getCompound(KEY);*/
    }

    private static void writeRoot(ItemStack stack, CompoundTag controller) {
        CustomData data = stack.get(DataComponents.CUSTOM_DATA);
        CompoundTag tag = data == null ? new CompoundTag() : data.copyTag();
        controller.putInt("v", VERSION);
        tag.put(KEY, controller);
        stack.set(DataComponents.CUSTOM_DATA, CustomData.of(tag));
    }

    private static String str(CompoundTag tag, String key) {
        //? if >=26.1 {
        return tag.getStringOr(key, "");
        //?} else
        /*return tag.getString(key);*/
    }

    private static CompoundTag compound(CompoundTag tag, String key) {
        //? if >=26.1 {
        return tag.getCompoundOrEmpty(key);
        //?} else
        /*return tag.getCompound(key);*/
    }

    @Nullable
    private static UUID uuid(String s) {
        if (s == null || s.isEmpty()) return null;
        try {
            return UUID.fromString(s);
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    /** This controller's id, or null if it was never paired. */
    @Nullable
    public static UUID id(ItemStack stack) {
        return uuid(str(root(stack), "id"));
    }

    /** The paired computer, or null. */
    @Nullable
    public static UUID computer(ItemStack stack) {
        return uuid(str(root(stack), "computer"));
    }

    /** Where the paired computer was when paired, or null. */
    @Nullable
    public static BlockPos pairedPos(ItemStack stack) {
        CompoundTag r = root(stack);
        if (!r.contains("pos")) return null;
        int[] p;
        //? if >=26.1 {
        p = r.getIntArray("pos").orElse(new int[0]);
        //?} else
        /*p = r.getIntArray("pos");*/
        return p.length == 3 ? new BlockPos(p[0], p[1], p[2]) : null;
    }

    /** Dimension the computer was paired in ("" if unpaired). */
    public static String pairedDimension(ItemStack stack) {
        return str(root(stack), "dim");
    }

    /** Pair with a computer. Gives the controller an id if it has none. */
    public static UUID pair(ItemStack stack, UUID computerId, BlockPos pos, String dimension) {
        CompoundTag r = root(stack);
        UUID id = uuid(str(r, "id"));
        if (id == null) {
            id = UUID.randomUUID();
            r.putString("id", id.toString());
        }
        r.putString("computer", computerId.toString());
        r.putIntArray("pos", new int[]{pos.getX(), pos.getY(), pos.getZ()});
        r.putString("dim", dimension);
        writeRoot(stack, r);
        return id;
    }

    /** Every input's key name ("" = unbound), defaults filled in. */
    public static Map<ControllerInput, String> bindings(ItemStack stack) {
        CompoundTag b = compound(root(stack), "bindings");
        Map<ControllerInput, String> out = new EnumMap<>(ControllerInput.class);
        for (ControllerInput in : ControllerInput.values()) {
            out.put(in, b.contains(in.id()) ? str(b, in.id()) : in.defaultKey());
        }
        return out;
    }

    /** The default bindings. */
    public static Map<ControllerInput, String> defaultBindings() {
        Map<ControllerInput, String> out = new EnumMap<>(ControllerInput.class);
        for (ControllerInput in : ControllerInput.values()) out.put(in, in.defaultKey());
        return out;
    }

    /** Store all bindings explicitly. Invalid names are dropped (unbound). */
    public static void setBindings(ItemStack stack, Map<ControllerInput, String> bindings) {
        CompoundTag r = root(stack);
        CompoundTag b = new CompoundTag();
        for (ControllerInput in : ControllerInput.values()) {
            String key = bindings.getOrDefault(in, "");
            b.putString(in.id(), isValidKeyName(key) ? key : "");
        }
        r.put("bindings", b);
        writeRoot(stack, r);
    }

    /** A plausible Minecraft key name (keyboard or mouse button), or "". */
    public static boolean isValidKeyName(String key) {
        if (key == null) return false;
        if (key.isEmpty()) return true;
        if (key.length() > MAX_KEY_NAME) return false;
        if (!key.startsWith("key.keyboard.") && !key.startsWith("key.mouse.")) return false;
        for (int i = 0; i < key.length(); i++) {
            char c = key.charAt(i);
            if (!(c >= 'a' && c <= 'z') && !(c >= '0' && c <= '9') && c != '.' && c != '_') return false;
        }
        return true;
    }
}
