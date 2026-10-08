package com.example.evanscomputermod.radio.phys;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Default per-block attenuation of common materials, referenced to 1 GHz, for a
 * full 1 m block. These are defaults for the data-driven {@code rf_attenuation}
 * tags, keyed by simple lower-case names.
 *
 * <p>Sources and assumptions:
 * <ul>
 * <li>glass, wood, concrete, brick, dirt (medium-dry ground), sand (very dry
 *     ground): ITU-R P.2040-1 Table 3 (&epsilon;r = a, &sigma; = c&middot;f<sup>d</sup>, f in GHz),
 *     &alpha; = 8.686 &middot; (&sigma;/2) &middot; 376.7/&radic;&epsilon;r dB/m, k = d (low-loss limit).</li>
 * <li>stone: dense rock, taken as P.2040 concrete; LF conduction as dry ground.</li>
 * <li>water: Debye relaxation of liquid water (&tau; &asymp; 9.3 ps, &epsilon;'' &asymp; 4.4 at 1 GHz)
 *     gives &asymp; 45 dB/m at 1 GHz rising as f&sup2; (&asymp; 260 dB/m at 2.4 GHz: opaque);
 *     LF conduction from fresh or sea water (skin depth).</li>
 * <li>leaves: dense in-leaf canopy, upper range of ITU-R P.833 specific attenuation.</li>
 * <li>iron, copper: physically opaque (skin depth of micrometres). Capped at
 *     80 dB per block, frequency-flat, so an enclosed metal room is a Faraday cage
 *     while the numbers stay finite. Metal is reflective, not absorptive; the
 *     reflection itself is modelled by {@link Ground#METAL} in {@link TwoRay}.</li>
 * <li>ice, snow: very low-loss dielectrics (&epsilon;'' &asymp; 10<sup>-3</sup> / 3&middot;10<sup>-4</sup>).</li>
 * <li>wool: dry textile, low loss.</li>
 * </ul>
 */
public final class Materials {
    public static final double REF_FREQ_HZ = 1e9;

    public static final MaterialAttenuation AIR = new MaterialAttenuation(0, REF_FREQ_HZ, 0);
    public static final MaterialAttenuation GLASS = new MaterialAttenuation(2.35, REF_FREQ_HZ, 1.34);
    public static final MaterialAttenuation WOOD = new MaterialAttenuation(5.45, REF_FREQ_HZ, 1.07);
    public static final MaterialAttenuation LEAVES = new MaterialAttenuation(1.0, REF_FREQ_HZ, 0.6);
    public static final MaterialAttenuation STONE = new MaterialAttenuation(33.0, REF_FREQ_HZ, 0.78, Ground.DRY_GROUND);
    public static final MaterialAttenuation CONCRETE = new MaterialAttenuation(33.0, REF_FREQ_HZ, 0.78, Ground.DRY_GROUND);
    public static final MaterialAttenuation BRICK = new MaterialAttenuation(19.7, REF_FREQ_HZ, 0.16, Ground.DRY_GROUND);
    public static final MaterialAttenuation DIRT = new MaterialAttenuation(14.8, REF_FREQ_HZ, 1.68, Ground.AVERAGE_GROUND);
    public static final MaterialAttenuation SAND = new MaterialAttenuation(0.142, REF_FREQ_HZ, 2.52, Ground.DRY_SAND);
    public static final MaterialAttenuation WATER = new MaterialAttenuation(45.0, REF_FREQ_HZ, 2.0, Ground.FRESH_WATER);
    public static final MaterialAttenuation SEA_WATER = new MaterialAttenuation(45.0, REF_FREQ_HZ, 2.0, Ground.SEA_WATER);
    public static final MaterialAttenuation IRON = new MaterialAttenuation(80.0, REF_FREQ_HZ, 0);
    public static final MaterialAttenuation COPPER = new MaterialAttenuation(80.0, REF_FREQ_HZ, 0);
    public static final MaterialAttenuation WOOL = new MaterialAttenuation(0.5, REF_FREQ_HZ, 1.0);
    public static final MaterialAttenuation ICE = new MaterialAttenuation(0.05, REF_FREQ_HZ, 1.0, Ground.ICE);
    public static final MaterialAttenuation SNOW = new MaterialAttenuation(0.02, REF_FREQ_HZ, 1.0, Ground.ICE);

    private static final Map<String, MaterialAttenuation> TABLE = new LinkedHashMap<>();

    static {
        TABLE.put("air", AIR);
        TABLE.put("glass", GLASS);
        TABLE.put("wood", WOOD);
        TABLE.put("leaves", LEAVES);
        TABLE.put("stone", STONE);
        TABLE.put("concrete", CONCRETE);
        TABLE.put("brick", BRICK);
        TABLE.put("dirt", DIRT);
        TABLE.put("sand", SAND);
        TABLE.put("water", WATER);
        TABLE.put("sea_water", SEA_WATER);
        TABLE.put("iron", IRON);
        TABLE.put("copper", COPPER);
        TABLE.put("wool", WOOL);
        TABLE.put("ice", ICE);
        TABLE.put("snow", SNOW);
    }

    /** Every default, in a stable order. */
    public static final Map<String, MaterialAttenuation> ALL = java.util.Collections.unmodifiableMap(TABLE);

    private Materials() {}

    /** The material named {@code name}, or null if unknown. */
    public static MaterialAttenuation get(String name) {
        return TABLE.get(name);
    }

    /** The material named {@code name}, or {@link #AIR} if unknown. */
    public static MaterialAttenuation getOrAir(String name) {
        return TABLE.getOrDefault(name, AIR);
    }
}
