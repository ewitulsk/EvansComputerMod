package com.example.evanscomputermod.radio.handheld;

//? if <=1.21.1 {
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.CustomData;

/** A handheld's settings, stored on the item (custom data "ecm_radio") so they travel with it. */
public record HandheldSettings(boolean on, HandheldBand band, double freqHz, int volume, int squelch) {

    public static final HandheldSettings DEFAULT = new HandheldSettings(false, HandheldBand.VHF, HandheldBand.VHF.defaultHz, 70, 0);

    public static HandheldSettings read(ItemStack stack) {
        CompoundTag t = stack.getOrDefault(DataComponents.CUSTOM_DATA, CustomData.EMPTY).copyTag().getCompound("ecm_radio");
        if (t.isEmpty()) return DEFAULT;
        HandheldBand band;
        try {
            band = HandheldBand.valueOf(t.getString("band"));
        } catch (IllegalArgumentException e) {
            band = HandheldBand.VHF;
        }
        double f = t.contains("freq") ? band.clamp(t.getDouble("freq")) : band.defaultHz;
        return new HandheldSettings(t.getBoolean("on"), band, f,
                t.contains("volume") ? Math.max(0, Math.min(100, t.getInt("volume"))) : 70,
                Math.max(0, Math.min(100, t.getInt("squelch"))));
    }

    public void write(ItemStack stack) {
        CustomData.update(DataComponents.CUSTOM_DATA, stack, root -> {
            CompoundTag t = new CompoundTag();
            t.putBoolean("on", on);
            t.putString("band", band.name());
            t.putDouble("freq", freqHz);
            t.putInt("volume", volume);
            t.putInt("squelch", squelch);
            root.put("ecm_radio", t);
        });
    }

    public HandheldSettings withOn(boolean v) { return new HandheldSettings(v, band, freqHz, volume, squelch); }
    public HandheldSettings withBand(HandheldBand b) { return new HandheldSettings(on, b, b.defaultHz, volume, squelch); }
    public HandheldSettings withFreq(double f) { return new HandheldSettings(on, band, band.clamp(f), volume, squelch); }
    public HandheldSettings withVolume(int v) { return new HandheldSettings(on, band, freqHz, Math.max(0, Math.min(100, v)), squelch); }
    public HandheldSettings withSquelch(int s) { return new HandheldSettings(on, band, freqHz, volume, Math.max(0, Math.min(100, s))); }
}
//?}
