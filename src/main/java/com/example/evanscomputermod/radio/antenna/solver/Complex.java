package com.example.evanscomputermod.radio.antenna.solver;

/**
 * Immutable complex number. Hot loops in the solver work on primitive re/im
 * arrays instead; this type is for the public API and small calculations.
 */
public record Complex(double re, double im) {
    public static final Complex ZERO = new Complex(0, 0);
    public static final Complex ONE = new Complex(1, 0);
    public static final Complex J = new Complex(0, 1);

    public static Complex of(double re, double im) { return new Complex(re, im); }

    public static Complex real(double re) { return new Complex(re, 0); }

    /** e^(j·phase). */
    public static Complex expj(double phase) { return new Complex(Math.cos(phase), Math.sin(phase)); }

    public Complex add(Complex o) { return new Complex(re + o.re, im + o.im); }

    public Complex sub(Complex o) { return new Complex(re - o.re, im - o.im); }

    public Complex mul(Complex o) { return new Complex(re * o.re - im * o.im, re * o.im + im * o.re); }

    public Complex scale(double s) { return new Complex(re * s, im * s); }

    public Complex div(Complex o) {
        // Smith's algorithm: avoids overflow for large or small denominators.
        if (Math.abs(o.re) >= Math.abs(o.im)) {
            double r = o.im / o.re, d = o.re + o.im * r;
            return new Complex((re + im * r) / d, (im - re * r) / d);
        }
        double r = o.re / o.im, d = o.re * r + o.im;
        return new Complex((re * r + im) / d, (im * r - re) / d);
    }

    public Complex conj() { return new Complex(re, -im); }

    public Complex negate() { return new Complex(-re, -im); }

    public Complex reciprocal() { return ONE.div(this); }

    public double abs() { return Math.hypot(re, im); }

    public double abs2() { return re * re + im * im; }

    public double arg() { return Math.atan2(im, re); }

    public Complex sqrt() {
        double m = abs();
        if (m == 0) return ZERO;
        double r = Math.sqrt((m + Math.abs(re)) / 2);
        if (re >= 0) return new Complex(r, im / (2 * r));
        return new Complex(Math.abs(im) / (2 * r), Math.copySign(r, im));
    }

    public Complex exp() {
        double e = Math.exp(re);
        return new Complex(e * Math.cos(im), e * Math.sin(im));
    }

    public boolean isFinite() { return Double.isFinite(re) && Double.isFinite(im); }

    @Override
    public String toString() {
        return String.format(java.util.Locale.ROOT, "%.4f%sj%.4f", re, im < 0 ? "-" : "+", Math.abs(im));
    }
}
