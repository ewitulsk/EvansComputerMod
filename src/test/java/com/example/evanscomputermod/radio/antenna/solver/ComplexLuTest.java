package com.example.evanscomputermod.radio.antenna.solver;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

import java.util.Random;

public class ComplexLuTest {
    @Test
    void solvesRandomSystemsAgainstKnownSolution() {
        Random rnd = new Random(42);
        for (int n : new int[] {1, 2, 7, 60, 200}) {
            double[] re = new double[n * n], im = new double[n * n];
            for (int i = 0; i < n * n; i++) { re[i] = rnd.nextGaussian(); im[i] = rnd.nextGaussian(); }
            double[] xr = new double[n], xi = new double[n];
            for (int i = 0; i < n; i++) { xr[i] = rnd.nextGaussian(); xi[i] = rnd.nextGaussian(); }
            double[] br = new double[n], bi = new double[n];
            for (int i = 0; i < n; i++)
                for (int j = 0; j < n; j++) {
                    br[i] += re[i * n + j] * xr[j] - im[i * n + j] * xi[j];
                    bi[i] += re[i * n + j] * xi[j] + im[i * n + j] * xr[j];
                }
            ComplexLu.factorInPlace(n, re, im).solveInPlace(br, bi);
            for (int i = 0; i < n; i++) {
                assertEquals(xr[i], br[i], 1e-8, "re x[" + i + "] n=" + n);
                assertEquals(xi[i], bi[i], 1e-8, "im x[" + i + "] n=" + n);
            }
        }
    }

    @Test
    void needsPivotingAndRejectsSingular() {
        // Zero on the diagonal: only solvable with row exchange.
        double[] re = {0, 1, 1, 0}, im = {0, 0, 0, 0};
        double[] br = {2, 3}, bi = {0, 0};
        ComplexLu.factorInPlace(2, re, im).solveInPlace(br, bi);
        assertEquals(3, br[0], 1e-12);
        assertEquals(2, br[1], 1e-12);
        assertThrows(ArithmeticException.class, () -> ComplexLu.factorInPlace(2, new double[] {1, 2, 2, 4}, new double[] {1, 2, 2, 4}));
    }

    @Test
    void complexArithmetic() {
        Complex a = Complex.of(3, 4), b = Complex.of(1, -2);
        assertEquals(5, a.abs(), 1e-12);
        assertEquals(Complex.of(11, -2), a.mul(b));
        Complex q = a.div(b);
        assertEquals(-1, q.re(), 1e-12);
        assertEquals(2, q.im(), 1e-12);
        Complex r = Complex.of(-4, 0).sqrt();
        assertEquals(0, r.re(), 1e-12);
        assertEquals(2, r.im(), 1e-12);
        assertEquals(-1, Complex.expj(Math.PI).re(), 1e-12);
    }
}
