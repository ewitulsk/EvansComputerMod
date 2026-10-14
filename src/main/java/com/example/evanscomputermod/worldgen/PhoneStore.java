package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.card.SimSecret;
import com.example.evanscomputermod.computer.CellPlan;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.radio.wifi.RadioWifiContent;

import net.minecraft.core.HolderLookup;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

/**
 * What a Tech Village's Phone Store holds (Cellular v2 phase 5): SIM cards preloaded with the
 * carrier network (written with the identity and key {@link CellPlan} provisions in the core's
 * database) and ePhones fitted with an LTE Modem and such a SIM. The keys live only in the
 * server-only {@code sim_secret} component: chests, item frames and dropped items reach clients
 * through the item network codec, which redacts it ({@code SimSecret.REDACTED_STREAM_CODEC}).
 */
public final class PhoneStore {
    private PhoneStore() {}

    /** Marker on the template's display frames: the phone's number (1-4). */
    public static final String PHONE_MARKER = "ecmStorePhone";
    /** Marker on the template's chest. */
    public static final String CHEST_MARKER = "ecmStore";

    /** A SIM card written for the carrier: its identity and its key. */
    public static ItemStack sim(CellPlan plan, CellPlan.Sim m) {
        ItemStack sim = new ItemStack(CellularContent.SIM_CARD.get());
        sim.set(CellularContent.SIM_CARD_DATA.get(), new SimIdentity(plan.carrier, Long.parseLong(m.imsi())));
        sim.set(CellularContent.SIM_SECRET.get(), SimSecret.of(m.key()));
        return sim;
    }

    /** An ePhone on display: Color LCD panel, Wi-Fi Module, LTE Modem and the store's SIM, full battery. */
    public static ItemStack phone(CellPlan plan, CellPlan.Sim m) {
        ItemStack phone = new ItemStack(PhoneContent.EPHONE.get());
        phone.set(PhoneContent.HARDWARE.get(), new PhoneHardware(new ItemStack(ModItems.COLOR_LCD_PANEL.get()),
                new ItemStack(RadioWifiContent.WIFI_MODULE.get()), new ItemStack(CellularContent.modem(PhyProfile.LTE)), sim(plan, m)));
        phone.set(DataComponents.ITEM_NAME, Component.literal("ePhone (" + plan.displayName() + ")"));
        return phone;
    }

    /** The chest's Items list: the village's SIMs, one per slot. */
    public static ListTag chestItems(CellPlan plan, int village, HolderLookup.Provider registries) {
        ListTag items = new ListTag();
        for (int i = 1; i <= CellPlan.SIMS; i++) {
            CompoundTag slot = (CompoundTag) sim(plan, plan.sim(village, false, i)).save(registries);
            slot.putByte("Slot", (byte) (i - 1));
            items.add(slot);
        }
        return items;
    }
}
//?}
