package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.phys.Ground;
import com.example.evanscomputermod.radio.phys.MaterialAttenuation;
import com.example.evanscomputermod.radio.phys.Materials;

/**
 * What one block does to a radio wave: its per-block attenuation α(f), the
 * ground it makes when a path reflects off it (two-ray), and whether it is a
 * metal reflector. Resolved from the {@code evanscomputermod:rf_attenuation}
 * data map (or the sound-type fallback) by the in-world adapter; pure here.
 *
 * @param fraction share of the block volume that is material (slabs 0.5, panes 0.25...)
 */
public record RfBlock(String name, MaterialAttenuation attenuation, Ground ground, boolean metal, double fraction) {

    public static final RfBlock AIR = new RfBlock("air", Materials.AIR, null, false, 0);
    public static final RfBlock STONE = of("stone");
    public static final RfBlock DIRT = of("dirt");
    public static final RfBlock WATER = of("water");
    public static final RfBlock GLASS = of("glass");
    public static final RfBlock WOOD = of("wood");
    public static final RfBlock IRON = of("iron");
    public static final RfBlock LEAVES = of("leaves");

    /** A full block of a named {@link Materials} default. */
    public static RfBlock of(String material) {
        MaterialAttenuation m = Materials.get(material);
        if (m == null) throw new IllegalArgumentException("unknown RF material " + material);
        boolean metal = material.equals("iron") || material.equals("copper");
        Ground g = metal ? Ground.METAL : m.lowFrequencyMedium();
        return new RfBlock(material, m, g, metal, 1);
    }

    public RfBlock scaled(double f) {
        return new RfBlock(name, attenuation, ground, metal, Math.max(0, Math.min(1, f)));
    }

    public boolean isAir() {
        return fraction <= 0 || attenuation.dbPerBlockAtRef() <= 0 && attenuation.lowFrequencyMedium() == null;
    }

    /** Attenuation through {@code lengthM} metres of this block at {@code freqHz}. */
    public double lossDb(double lengthM, double freqHz) {
        if (fraction <= 0) return 0;
        return attenuation.dbPerBlock(freqHz) * lengthM * fraction;
    }

    /** Ground to use for a reflection off this block (dry ground when the material names none). */
    public Ground groundOrDefault() {
        if (ground != null) return ground;
        return isAir() ? null : Ground.DRY_GROUND;
    }
}
