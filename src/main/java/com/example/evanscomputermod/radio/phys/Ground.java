package com.example.evanscomputermod.radio.phys;

/**
 * Electrical constants of a ground or water medium: relative permittivity
 * &epsilon;<sub>r</sub> and conductivity &sigma; (S/m). The complex relative
 * permittivity is &epsilon;<sub>c</sub> = &epsilon;<sub>r</sub> &minus; j&sigma;/(&omega;&epsilon;0)
 * = &epsilon;<sub>r</sub> &minus; j60&sigma;&lambda;. {@link #METAL} has infinite conductivity.
 *
 * <p>Preset values follow ITU-R P.527 / P.368 ground classes at LF-HF; sea water
 * uses the common textbook &sigma; = 4 S/m (P.527 gives 4-5 S/m depending on
 * temperature and salinity).
 */
public record Ground(double relativePermittivity, double conductivitySPerM) {
    public static final Ground SEA_WATER = new Ground(80, 4.0);
    public static final Ground FRESH_WATER = new Ground(80, 0.003);
    public static final Ground WET_GROUND = new Ground(30, 0.01);
    public static final Ground AVERAGE_GROUND = new Ground(15, 0.005);
    public static final Ground DRY_GROUND = new Ground(4, 0.001);
    public static final Ground DRY_SAND = new Ground(3, 0.0001);
    public static final Ground ICE = new Ground(3.2, 0.00001);
    /** Perfect conductor: reflection coefficient &minus;1 (horizontal) / +1 (vertical), no penetration. */
    public static final Ground METAL = new Ground(1, Double.POSITIVE_INFINITY);

    public Ground {
        if(!(relativePermittivity >= 1)) throw new IllegalArgumentException("relative permittivity must be >= 1");
        if(!(conductivitySPerM >= 0)) throw new IllegalArgumentException("conductivity must be >= 0");
    }

    public boolean isPerfectConductor() {
        return conductivitySPerM == Double.POSITIVE_INFINITY;
    }

    /** Imaginary part magnitude &sigma;/(&omega;&epsilon;0) = 60&sigma;&lambda; at {@code freqHz}. */
    public double lossFactor(double freqHz) {
        return conductivitySPerM / (Units.angularFrequency(freqHz) * Units.EPS0_F_PER_M);
    }

    /** Loss tangent &sigma;/(&omega;&epsilon;0&epsilon;r): &gt;&gt; 1 is a good conductor, &lt;&lt; 1 a dielectric. */
    public double lossTangent(double freqHz) {
        return lossFactor(freqHz) / relativePermittivity;
    }

    /** Complex relative permittivity &epsilon;r &minus; j&sigma;/(&omega;&epsilon;0). */
    public Complex complexPermittivity(double freqHz) {
        return new Complex(relativePermittivity, -lossFactor(freqHz));
    }
}
