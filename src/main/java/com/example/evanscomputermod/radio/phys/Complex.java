package com.example.evanscomputermod.radio.phys;

/** Immutable complex number (field phasors, reflection coefficients, permittivities). */
public record Complex(double re, double im) {
    public static final Complex ZERO = new Complex(0, 0);
    public static final Complex ONE = new Complex(1, 0);

    public static Complex polar(double magnitude, double phaseRad) {
        return new Complex(magnitude * Math.cos(phaseRad), magnitude * Math.sin(phaseRad));
    }

    public Complex plus(Complex o) {
        return new Complex(re + o.re, im + o.im);
    }

    public Complex plus(double r) {
        return new Complex(re + r, im);
    }

    public Complex minus(Complex o) {
        return new Complex(re - o.re, im - o.im);
    }

    public Complex times(Complex o) {
        return new Complex(re * o.re - im * o.im, re * o.im + im * o.re);
    }

    public Complex times(double s) {
        return new Complex(re * s, im * s);
    }

    public Complex div(Complex o) {
        double d = o.re * o.re + o.im * o.im;
        return new Complex((re * o.re + im * o.im) / d, (im * o.re - re * o.im) / d);
    }

    public Complex conj() {
        return new Complex(re, -im);
    }

    /** Magnitude |z|. */
    public double abs() {
        return Math.hypot(re, im);
    }

    /** Squared magnitude |z|&sup2; (power of a field phasor). */
    public double abs2() {
        return re * re + im * im;
    }

    public double arg() {
        return Math.atan2(im, re);
    }

    /** Principal square root (non-negative real part). */
    public Complex sqrt() {
        double r = abs();
        if(r == 0) return ZERO;
        double a = Math.sqrt((r + re) / 2);
        double b = Math.copySign(Math.sqrt(Math.max(0, (r - re) / 2)), im);
        return new Complex(a, b);
    }

    /** Power |z|&sup2; in dB. */
    public double powerDb() {
        return Units.linearToDb(abs2());
    }
}
