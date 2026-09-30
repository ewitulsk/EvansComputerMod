package com.example.evanscomputermod.storage;

//? if <=1.21.1 {

import com.example.evanscomputermod.module.InstalledModules;
import com.example.evanscomputermod.module.ModDataComponents;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.item.StorageModuleItem;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.BundleContents;
import net.minecraft.world.item.component.CustomData;
import net.minecraft.world.item.component.ItemContainerContents;
import org.jetbrains.annotations.Nullable;

import java.util.Locale;
import java.util.UUID;

/**
 * What may be encoded, and how items are named for programs.
 *
 * <p>Containers (shulker boxes, bundles) may be stored, as in AE2: each full
 * container is one unique type, so the gain is bounded. A Storage Cell that
 * holds items may not be stored anywhere inside another cell (directly, in a
 * shulker, in a bundle, in a storage module or a drive item): its contents
 * live in the ledger, so nesting would be unbounded storage.
 */
public final class StorageRules {

    private static final int MAX_DEPTH = 8;

    private StorageRules() {
    }

    /** Why {@code stack} can't be encoded, or null if it can. */
    @Nullable
    public static String whyNotEncodable(ItemStack stack, StorageLedger ledger) {
        return check(stack, ledger, ledger.registries(), 0);
    }

    @Nullable
    private static String check(ItemStack stack, StorageLedger ledger, HolderLookup.Provider registries, int depth) {
        if (stack.isEmpty()) return null;
        if (depth > MAX_DEPTH) return "items are nested too deeply";
        if (stack.is(StorageContent.STORAGE_BLACKLIST)) return stack.getHoverName().getString() + " can't be stored";

        UUID cell = StorageCellItem.cellId(stack);
        if (cell != null) {
            CellContents c = ledger.get(cell);
            if (c != null && !c.isEmpty()) return "a Storage Cell with items in it can't be stored";
        }
        if (stack.getItem() instanceof StorageModuleItem) {
            String why = check(StorageModuleItem.cellIn(stack, registries), ledger, registries, depth + 1);
            if (why != null) return why;
        }
        InstalledModules modules = stack.get(ModDataComponents.INSTALLED_MODULES.get());
        if (modules != null) {
            for (ItemStack inner : modules.slots()) {
                String why = check(inner, ledger, registries, depth + 1);
                if (why != null) return why;
            }
        }
        ItemContainerContents container = stack.get(DataComponents.CONTAINER);
        if (container != null) {
            for (ItemStack inner : container.nonEmptyItems()) {
                String why = check(inner, ledger, registries, depth + 1);
                if (why != null) return why;
            }
        }
        BundleContents bundle = stack.get(DataComponents.BUNDLE_CONTENTS);
        if (bundle != null) {
            for (ItemStack inner : bundle.items()) {
                String why = check(inner, ledger, registries, depth + 1);
                if (why != null) return why;
            }
        }
        CustomData be = stack.get(DataComponents.BLOCK_ENTITY_DATA);
        if (be != null) {
            // A Drive or Decoder picked with its contents (creative ctrl+pick).
            CompoundTag tag = be.copyTag();
            for (String key : new String[] {"cells", "buffer"}) {
                if (!tag.contains(key)) continue;
                ListTag items = tag.getCompound(key).getList("Items", Tag.TAG_COMPOUND);
                for (int i = 0; i < items.size(); i++) {
                    ItemStack inner = ItemStack.parse(registries, items.getCompound(i)).orElse(ItemStack.EMPTY);
                    String why = check(inner, ledger, registries, depth + 1);
                    if (why != null) return why;
                }
            }
        }
        return null;
    }

    /** Registry id, e.g. {@code minecraft:iron_ingot}. */
    public static String itemId(ItemStack stack) {
        return BuiltInRegistries.ITEM.getKey(stack.getItem()).toString();
    }

    /**
     * Display name. On a dedicated server, modded names aren't translated (the
     * server has no mod language files), so a raw translation key becomes a
     * title-cased id: {@code create:brass_ingot} is "Brass Ingot".
     */
    public static String displayName(ItemStack stack) {
        String name = stack.getHoverName().getString();
        if (!name.equals(stack.getItem().getDescriptionId())) return name;
        String path = BuiltInRegistries.ITEM.getKey(stack.getItem()).getPath();
        StringBuilder out = new StringBuilder();
        for (String word : path.split("_")) {
            if (word.isEmpty()) continue;
            if (!out.isEmpty()) out.append(' ');
            out.append(word.substring(0, 1).toUpperCase(Locale.ROOT)).append(word.substring(1));
        }
        return out.toString();
    }
}
//?}
