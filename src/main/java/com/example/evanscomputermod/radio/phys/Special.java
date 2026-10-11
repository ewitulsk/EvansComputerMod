package com.example.evanscomputermod.radio.phys;

/** Special functions for error-rate formulas. */
public final class Special {
    private Special() {}

    /**
     * Complementary error function. Chebyshev fit from Numerical Recipes
     * ({@code erfcc}); fractional error below 1.2&times;10<sup>-7</sup> everywhere,
     * so tails like Q(4.27) &asymp; 10<sup>-5</sup> are accurate.
     */
    public static double erfc(double x) {
        double z = Math.abs(x);
        double t = 1.0 / (1.0 + 0.5 * z);
        double r = t * Math.exp(-z * z - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418
                + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587
                + t * (-0.82215223 + t * 0.17087277)))))))));
        return x >= 0 ? r : 2.0 - r;
    }

    public static double erf(double x) {
        return 1.0 - erfc(x);
    }

    /** Gaussian tail Q(x) = &frac12; erfc(x/&radic;2). */
    public static double q(double x) {
        return 0.5 * erfc(x / Math.sqrt(2.0));
    }
}
