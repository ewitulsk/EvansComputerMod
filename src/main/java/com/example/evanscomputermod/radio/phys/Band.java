package com.example.evanscomputermod.radio.phys;

/** The spec's seven bands (Bands and channels) with frequency limits and channel plans. */
public enum Band {
    VLF_LF(3e3, 300e3, SpectralMask.GENERIC),
    MF(300e3, 3e6, SpectralMask.GENERIC),
    HF(3e6, 30e6, SpectralMask.GENERIC),
    VHF(30e6, 300e6, SpectralMask.GENERIC),
    UHF(300e6, 1e9, SpectralMask.GENERIC),
    WIFI_24(2.400e9, 2.4835e9, SpectralMask.DSSS_22MHZ),
    WIFI_5(5.150e9, 5.925e9, SpectralMask.OFDM_20MHZ),
    MICROWAVE(10e9, 60e9, SpectralMask.GENERIC);

    public final double minHz, maxHz;
    public final SpectralMask defaultMask;

    Band(double minHz, double maxHz, SpectralMask defaultMask) {
        this.minHz = minHz;
        this.maxHz = maxHz;
        this.defaultMask = defaultMask;
    }

    public boolean contains(double freqHz) {
        return freqHz >= minHz && freqHz <= maxHz;
    }

    /** Band containing {@code freqHz}, or null if it falls between bands. */
    public static Band of(double freqHz) {
        for(Band b : values()) if(b.contains(freqHz)) return b;
        return null;
    }

    /** 2.4 GHz Wi-Fi channel centre: 2407 + 5n MHz for 1..13, 2484 MHz for 14. */
    public static double wifi24ChannelHz(int channel) {
        if(channel == 14) return 2484e6;
        if(channel < 1 || channel > 13) throw new IllegalArgumentException("2.4 GHz channel 1..14");
        return 2407e6 + 5e6 * channel;
    }

    /** 5 GHz Wi-Fi channel centre: 5000 + 5n MHz (UNII channels 36..165). */
    public static double wifi5ChannelHz(int channel) {
        if(channel < 32 || channel > 177) throw new IllegalArgumentException("5 GHz channel 32..177");
        return 5000e6 + 5e6 * channel;
    }
}
