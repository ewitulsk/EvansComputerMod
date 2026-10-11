package com.example.evanscomputermod.radio.api;

/**
 * The radio bands of the Radio &amp; Wireless spec, from slow far-reaching VLF to
 * fast short-range Wi-Fi and microwave links. Frequencies are real (1 block = 1 m).
 */
public enum Band {
    VLF_LF(3e3, 300e3),
    MF(300e3, 3e6),
    HF(3e6, 30e6),
    VHF(30e6, 300e6),
    UHF(300e6, 1e9),
    WIFI_2G4(2.400e9, 2.4835e9),
    WIFI_5G(5.150e9, 5.895e9),
    MICROWAVE(10e9, 60e9);

    public final double minHz;
    public final double maxHz;

    Band(double minHz, double maxHz) {
        this.minHz = minHz;
        this.maxHz = maxHz;
    }

    public boolean contains(double hz) {
        return hz >= minHz && hz <= maxHz;
    }

    /** The band a frequency falls in, or null outside every band (e.g. 1–2.4 GHz). */
    public static Band of(double hz) {
        for (Band b : values()) if (b.contains(hz)) return b;
        return null;
    }
}
