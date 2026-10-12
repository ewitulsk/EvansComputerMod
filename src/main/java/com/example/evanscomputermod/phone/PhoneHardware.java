package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.item.DisplayPanelItem;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.radio.wifi.RadioWifiContent;
import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

/**
 * What is fitted in an ePhone: the display panel (empty = headless), two
 * module slots and the SIM tray. Stored on the phone item as
 * {@code evanscomputermod:phone_hardware}; module state lives in each module
 * stack's custom data, as in a Terminal's bays. A phone without the component
 * (fresh from the creative tab or {@code /give}) has the factory fit:
 * a Color LCD panel and a Wi-Fi Module in slot 1.
 */
public record PhoneHardware(ItemStack panel, ItemStack module1, ItemStack module2, ItemStack sim) {

    public static final Codec<PhoneHardware> CODEC = RecordCodecBuilder.create(i -> i.group(
            ItemStack.OPTIONAL_CODEC.fieldOf("panel").forGetter(PhoneHardware::panel),
            ItemStack.OPTIONAL_CODEC.fieldOf("module1").forGetter(PhoneHardware::module1),
            ItemStack.OPTIONAL_CODEC.fieldOf("module2").forGetter(PhoneHardware::module2),
            ItemStack.OPTIONAL_CODEC.fieldOf("sim").forGetter(PhoneHardware::sim)
    ).apply(i, PhoneHardware::new));

    public static final StreamCodec<RegistryFriendlyByteBuf, PhoneHardware> STREAM_CODEC = StreamCodec.composite(
            ItemStack.OPTIONAL_STREAM_CODEC, PhoneHardware::panel,
            ItemStack.OPTIONAL_STREAM_CODEC, PhoneHardware::module1,
            ItemStack.OPTIONAL_STREAM_CODEC, PhoneHardware::module2,
            ItemStack.OPTIONAL_STREAM_CODEC, PhoneHardware::sim,
            PhoneHardware::new);

    public static final PhoneHardware EMPTY = new PhoneHardware(ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY);

    public PhoneHardware {
        panel = one(panel);
        module1 = one(module1);
        module2 = one(module2);
        sim = one(sim);
    }

    private static ItemStack one(ItemStack s) {
        return s == null || s.isEmpty() ? ItemStack.EMPTY : s.copyWithCount(1);
    }

    /** The factory fit: Color LCD panel, a Wi-Fi Module in slot 1, no SIM. */
    public static PhoneHardware factory() {
        return new PhoneHardware(new ItemStack(ModItems.COLOR_LCD_PANEL.get()), new ItemStack(RadioWifiContent.WIFI_MODULE.get()),
                ItemStack.EMPTY, ItemStack.EMPTY);
    }

    /** The phone's hardware (the factory fit when it has none recorded). */
    public static PhoneHardware of(ItemStack phone) {
        PhoneHardware h = phone.get(PhoneContent.HARDWARE.get());
        return h != null ? h : factory();
    }

    /** Panel tier, or null when no panel is fitted (headless). */
    @Nullable
    public PanelTier tier() {
        return DisplayPanelItem.tierOf(panel);
    }

    public PanelTier.Tone tone() {
        return DisplayPanelItem.toneOf(panel);
    }

    public ItemStack module(int i) {
        return i == 0 ? module1 : module2;
    }

    public PhoneHardware withPanel(ItemStack s) {
        return new PhoneHardware(s, module1, module2, sim);
    }

    public PhoneHardware withModule(int i, ItemStack s) {
        return i == 0 ? new PhoneHardware(panel, s, module2, sim) : new PhoneHardware(panel, module1, s, sim);
    }

    public PhoneHardware withSim(ItemStack s) {
        return new PhoneHardware(panel, module1, module2, s);
    }

    /** Slot {@code i} in menu order: 0 panel, 1 module 1, 2 module 2, 3 SIM. */
    public ItemStack slot(int i) {
        return switch (i) {
            case 0 -> panel;
            case 1 -> module1;
            case 2 -> module2;
            default -> sim;
        };
    }

    public PhoneHardware withSlot(int i, ItemStack s) {
        return switch (i) {
            case 0 -> withPanel(s);
            case 1 -> withModule(0, s);
            case 2 -> withModule(1, s);
            default -> withSim(s);
        };
    }

    // ItemStack has identity equality; data components need value equality.
    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (!(o instanceof PhoneHardware h)) return false;
        for (int i = 0; i < 4; i++) if (!ItemStack.matches(slot(i), h.slot(i))) return false;
        return true;
    }

    @Override
    public int hashCode() {
        int h = 0;
        for (int i = 0; i < 4; i++) h = 31 * h + ItemStack.hashItemAndComponents(slot(i));
        return h;
    }
}
//?}
