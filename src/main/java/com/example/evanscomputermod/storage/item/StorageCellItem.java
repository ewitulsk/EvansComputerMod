package com.example.evanscomputermod.storage.item;

//? if <=1.21.1 {

import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.StorageEvents;
import com.example.evanscomputermod.storage.core.CellTier;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.UUID;
import java.util.function.Consumer;

/**
 * A Storage Cell. The item only carries an id ({@link StorageContent#CELL_ID},
 * given on first mount) and a display summary; what the cell holds lives in
 * the world's storage ledger. Copying the item (creative pick, dupes) copies
 * the id, not the items, and only one copy can be mounted at a time.
 */
public class StorageCellItem extends TooltipItem {

    private final CellTier tier;

    public StorageCellItem(CellTier tier, Properties properties) {
        super(properties);
        this.tier = tier;
    }

    public CellTier tier() {
        return tier;
    }

    @Nullable
    public static UUID cellId(ItemStack stack) {
        return stack.getItem() instanceof StorageCellItem ? stack.get(StorageContent.CELL_ID.get()) : null;
    }

    /** The cell's id, giving it one if it has none yet. */
    public static UUID ensureId(ItemStack stack) {
        UUID id = stack.get(StorageContent.CELL_ID.get());
        if (id == null) {
            id = UUID.randomUUID();
            stack.set(StorageContent.CELL_ID.get(), id);
        }
        return id;
    }

    @Nullable
    public static CellTier tierOf(ItemStack stack) {
        return stack.getItem() instanceof StorageCellItem c ? c.tier : null;
    }

    /** Burnt, blown up, cactus'd: the cell's contents are gone. */
    @Override
    public void onDestroyed(ItemEntity entity) {
        super.onDestroyed(entity);
        StorageEvents.itemGone(entity);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        CellSummary s = stack.get(StorageContent.CELL_SUMMARY.get());
        long used = s == null ? 0 : s.bytes();
        int types = s == null ? 0 : s.types();
        lines.accept(Component.translatable("tooltip.evanscomputermod.cell.bytes", used, tier.bytes())
                .withStyle(ChatFormatting.GRAY));
        lines.accept(Component.translatable("tooltip.evanscomputermod.cell.types", types, CellTier.MAX_TYPES)
                .withStyle(ChatFormatting.GRAY));
        if (s != null && s.items() > 0) {
            lines.accept(Component.translatable("tooltip.evanscomputermod.cell.items", s.items())
                    .withStyle(ChatFormatting.GRAY));
        }
        UUID id = stack.get(StorageContent.CELL_ID.get());
        if (id != null) {
            lines.accept(Component.translatable("tooltip.evanscomputermod.cell.id", id.toString().substring(0, 8))
                    .withStyle(ChatFormatting.DARK_GRAY));
        }
    }
}
//?}
