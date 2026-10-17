package com.example.evanscomputermod.disk;

//? if <=1.21.1 {
import com.example.evanscomputermod.item.TooltipItem;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * A floppy disk: files that travel with the item. Put it in a Disk Drive next
 * to a computer and it appears as {@code disk/}. The disk's number
 * ({@link DiskContent#DISK_ID}) is given the first time it goes into a drive;
 * its label is the item's name (rename it on an anvil, or
 * {@code disk.set_label} from a program).
 */
public class FloppyDiskItem extends TooltipItem {
    public FloppyDiskItem(Properties properties) {
        super(properties);
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        Integer id = stack.get(DiskContent.DISK_ID.get());
        lines.accept((id == null
                ? Component.translatable("item.evanscomputermod.floppy_disk.blank")
                : Component.translatable("item.evanscomputermod.floppy_disk.id", id)).withStyle(ChatFormatting.GRAY));
    }
}
//?}
