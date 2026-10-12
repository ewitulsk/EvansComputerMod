package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.UUID;

/** Reading and writing an ePhone stack's components (computer id, battery, hardware). */
public final class PhoneItemData {
    private PhoneItemData() {}

    public static boolean isPhone(ItemStack stack) {
        return !stack.isEmpty() && stack.getItem() instanceof EPhoneItem;
    }

    /** The computer this phone runs, or null before its first use. */
    @Nullable
    public static UUID id(ItemStack stack) {
        return isPhone(stack) ? stack.get(PhoneContent.PHONE_ID.get()) : null;
    }

    /**
     * Give a phone its computer (first use): a fresh id, the factory hardware
     * written out and a new phone's charge. Returns the id. Server side.
     */
    public static UUID ensureId(ItemStack stack) {
        UUID id = id(stack);
        if (id != null) return id;
        id = UUID.randomUUID();
        stack.set(PhoneContent.PHONE_ID.get(), id);
        if (!stack.has(PhoneContent.HARDWARE.get())) stack.set(PhoneContent.HARDWARE.get(), PhoneHardware.factory());
        if (!stack.has(PhoneContent.BATTERY.get())) stack.set(PhoneContent.BATTERY.get(), PhoneConfig.newPhonePercent() * 100);
        return id;
    }

    /** Charge in percent (a phone never switched on: the new-phone charge). */
    public static double batteryPercent(ItemStack stack) {
        Integer centi = stack.get(PhoneContent.BATTERY.get());
        return centi != null ? Math.max(0, Math.min(10_000, centi)) / 100.0 : PhoneConfig.newPhonePercent();
    }

    public static void setBatteryPercent(ItemStack stack, double percent) {
        stack.set(PhoneContent.BATTERY.get(), (int) Math.round(Math.max(0, Math.min(100, percent)) * 100));
    }
}
//?}
