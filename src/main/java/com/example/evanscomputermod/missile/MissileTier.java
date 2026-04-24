package com.example.evanscomputermod.missile;

/**
 * Defines the physical capabilities of each missile class.
 * All speeds are in blocks/tick, angles in degrees.
 */
public enum MissileTier {

    /** Medium Range Ballistic Missile — forgiving guidance, short range, fast to deploy. */
    MRBM("mrbm", "Medium Range Missile",
            500,   // maxRange
            40,    // thrustTicks
            2.5,   // topSpeed (blocks/tick)
            4.0,   // turnRate (degrees/tick)
            0.4f), // defaultYield

    /** Long Range Ballistic Missile — moderate arc, solid range, limited guidance authority. */
    LRBM("lrbm", "Long Range Missile",
            2000,
            80,
            4.0,
            2.0,
            0.6f),

    /** Intercontinental Ballistic Missile — massive range, high loft arc, nearly inflexible trajectory. */
    ICBM("icbm", "ICBM",
            8000,
            160,
            8.0,
            0.8,
            0.9f);

    public final String id;
    public final String displayName;
    /** Maximum guided range in blocks. */
    public final int maxRange;
    /** Motor burn duration in ticks. */
    public final int thrustTicks;
    /** Velocity at end of thrust phase (blocks/tick). */
    public final double topSpeed;
    /** Max guidance correction per tick (degrees). */
    public final double turnRate;
    /** Default warhead yield scale (0.0–1.0). */
    public final float defaultYield;

    MissileTier(String id, String displayName, int maxRange, int thrustTicks,
                double topSpeed, double turnRate, float defaultYield) {
        this.id = id;
        this.displayName = displayName;
        this.maxRange = maxRange;
        this.thrustTicks = thrustTicks;
        this.topSpeed = topSpeed;
        this.turnRate = turnRate;
        this.defaultYield = defaultYield;
    }

    public static MissileTier fromId(String id) {
        for (MissileTier t : values()) {
            if (t.id.equals(id)) return t;
        }
        return MRBM;
    }
}
