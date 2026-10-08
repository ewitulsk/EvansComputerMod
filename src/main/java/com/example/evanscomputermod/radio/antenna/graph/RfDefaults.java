package com.example.evanscomputermod.radio.antenna.graph;

import java.util.Map;

/**
 * Built-in electrical values, keyed by block (or item) registry path / ID.
 * They are the fallback when the {@code evanscomputermod:rf_conductor} data
 * map has no entry, and the shipped JSON
 * ({@code data/evanscomputermod/data_maps/block/rf_conductor.json}) carries
 * the same numbers so packs can see and override them.
 *
 * <p>Current ratings are chosen so a resonant half-wave dipole (≈ 73 Ω, where
 * P = I<sub>rms</sub>² · R at the feed) is limited to the spec's power per
 * tier: copper wire ≈ 50 W, antenna wire ≈ 200 W, heavy cable ≈ 2 kW, rod ≈
 * 10 kW, lattice mast ≈ 50 kW, fine wire ≈ 5 W.
 */
public final class RfDefaults {
    private RfDefaults() {}

    private static final double CU = 1.68e-8, AL = 2.65e-8, FE = 9.7e-8;

    public static final ConductorSpec COPPER_WIRE = new ConductorSpec("copper wire", 0.0010, CU, 0.85, 1_500, true);
    public static final ConductorSpec ANTENNA_WIRE = new ConductorSpec("antenna wire", 0.0016, CU, 1.65, 2_500, true);
    public static final ConductorSpec HEAVY_CABLE = new ConductorSpec("heavy cable", 0.0050, CU, 5.2, 5_000, true);
    public static final ConductorSpec ANTENNA_ROD = new ConductorSpec("antenna rod", 0.0125, AL, 11.7, 10_000, false);
    public static final ConductorSpec LATTICE_MAST = new ConductorSpec("lattice mast", 0.20, FE, 26, 40_000, false);
    /** The routed Fine Wire (sensor_wire entity): 0.5 mm copper, ~5 W. */
    public static final ConductorSpec FINE_WIRE = new ConductorSpec("fine wire", 0.00025, CU, 0.27, 800, false);
    /** The feed point's own gap conductor. */
    public static final ConductorSpec FEED_POINT = new ConductorSpec("feed point", 0.003, CU, 30, 3_000, false);
    public static final ConductorSpec LIGHTNING_ROD = new ConductorSpec("lightning rod", 0.004, CU, 8, 4_000, false);
    public static final ConductorSpec IRON_BARS = new ConductorSpec("iron bars", 0.03, FE, 20, 8_000, false);
    public static final ConductorSpec CHAIN = new ConductorSpec("chain", 0.01, FE, 10, 5_000, false);
    /** Any other block in {@code #evanscomputermod:rf_conductors} (iron, copper, gold blocks...). */
    public static final ConductorSpec METAL_BLOCK = new ConductorSpec("metal block", 0.20, FE, 100, 40_000, false);

    /** Peak voltage ratings of insulating parts, V. */
    public static final double INSULATOR_VOLTS = 4_000;
    public static final double FEED_POINT_VOLTS = 3_000;

    public static final CoaxSpec COAX_CABLE = new CoaxSpec("coax", 0.49, 6.6, 600);
    public static final CoaxSpec HARDLINE = new CoaxSpec("hardline", 0.02, 0.23, 20_000);
    /** Per 10 blocks, so one arrestor block adds 0.005 dB at HF. */
    public static final CoaxSpec LIGHTNING_ARRESTOR = new CoaxSpec("lightning arrestor", 0.05, 1.0, 5_000);

    public static final Map<String, ConductorSpec> CONDUCTORS = Map.ofEntries(
            Map.entry("evanscomputermod:copper_wire", COPPER_WIRE),
            Map.entry("evanscomputermod:antenna_wire", ANTENNA_WIRE),
            Map.entry("evanscomputermod:heavy_cable", HEAVY_CABLE),
            Map.entry("evanscomputermod:antenna_rod", ANTENNA_ROD),
            Map.entry("evanscomputermod:lattice_mast", LATTICE_MAST),
            Map.entry("evanscomputermod:feed_point", FEED_POINT),
            Map.entry("minecraft:lightning_rod", LIGHTNING_ROD),
            Map.entry("minecraft:iron_bars", IRON_BARS),
            Map.entry("minecraft:chain", CHAIN));

    public static final Map<String, CoaxSpec> COAX = Map.of(
            "evanscomputermod:coax_cable", COAX_CABLE,
            "evanscomputermod:hardline", HARDLINE,
            "evanscomputermod:lightning_arrestor", LIGHTNING_ARRESTOR);

    public static final Map<String, Double> INSULATORS = Map.of(
            "evanscomputermod:insulator", INSULATOR_VOLTS,
            "evanscomputermod:feed_point", FEED_POINT_VOLTS);
}
