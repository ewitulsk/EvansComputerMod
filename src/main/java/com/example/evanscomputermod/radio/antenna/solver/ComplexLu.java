package com.example.evanscomputermod.radio.antenna.solver;

/**
 * Dense complex LU factorization with partial pivoting (Doolittle, row-major,
 * split real/imaginary arrays). Sized for the antenna solver's N ≤ a few
 * hundred. Instances are not thread-safe; create one per solve.
 */
public final class ComplexLu {
    private final int n;
    private final double[] re, im;
    private final int[] pivot;

    private ComplexLu(int n, double[] re, double[] im, int[] pivot) {
        this.n = n;
        this.re = re;
        this.im = im;
        this.pivot = pivot;
    }

    public int size() { return n; }

    /**
     * Factors the n×n row-major matrix (re, im) in place; the arrays become
     * the packed L and U factors and must not be reused by the caller.
     *
     * @throws ArithmeticException if the matrix is singular to working precision
     */
    public static ComplexLu factorInPlace(int n, double[] re, double[] im) {
        if (n <= 0 || re.length < n * n || im.length < n * n) throw new IllegalArgumentException("matrix size");
        int[] pivot = new int[n];
        double scale = 0;
        for (int i = 0; i < n * n; i++) scale = Math.max(scale, Math.hypot(re[i], im[i]));
        double tiny = scale * 1e-14;
        for (int k = 0; k < n; k++) {
            int p = k;
            double best = -1;
            for (int i = k; i < n; i++) {
                double m = re[i * n + k] * re[i * n + k] + im[i * n + k] * im[i * n + k];
                if (m > best) { best = m; p = i; }
            }
            if (!(Math.sqrt(best) > tiny)) throw new ArithmeticException("singular impedance matrix at column " + k);
            pivot[k] = p;
            if (p != k) {
                for (int j = 0; j < n; j++) {
                    int a = k * n + j, b = p * n + j;
                    double t = re[a]; re[a] = re[b]; re[b] = t;
                    t = im[a]; im[a] = im[b]; im[b] = t;
                }
            }
            double pr = re[k * n + k], pi = im[k * n + k];
            double d = pr * pr + pi * pi;
            double ir = pr / d, ii = -pi / d;
            for (int i = k + 1; i < n; i++) {
                int ik = i * n + k;
                double lr = re[ik] * ir - im[ik] * ii, li = re[ik] * ii + im[ik] * ir;
                re[ik] = lr;
                im[ik] = li;
                if (lr == 0 && li == 0) continue;
                int rowI = i * n, rowK = k * n;
                for (int j = k + 1; j < n; j++) {
                    double ur = re[rowK + j], ui = im[rowK + j];
                    re[rowI + j] -= lr * ur - li * ui;
                    im[rowI + j] -= lr * ui + li * ur;
                }
            }
        }
        return new ComplexLu(n, re, im, pivot);
    }

    /** Solves A·x = b in place: (bRe, bIm) is overwritten with x. */
    public void solveInPlace(double[] bRe, double[] bIm) {
        for (int k = 0; k < n; k++) {
            int p = pivot[k];
            if (p != k) {
                double t = bRe[k]; bRe[k] = bRe[p]; bRe[p] = t;
                t = bIm[k]; bIm[k] = bIm[p]; bIm[p] = t;
            }
        }
        for (int i = 0; i < n; i++) {
            double sr = bRe[i], si = bIm[i];
            int row = i * n;
            for (int j = 0; j < i; j++) {
                double lr = re[row + j], li = im[row + j];
                sr -= lr * bRe[j] - li * bIm[j];
                si -= lr * bIm[j] + li * bRe[j];
            }
            bRe[i] = sr;
            bIm[i] = si;
        }
        for (int i = n - 1; i >= 0; i--) {
            double sr = bRe[i], si = bIm[i];
            int row = i * n;
            for (int j = i + 1; j < n; j++) {
                double ur = re[row + j], ui = im[row + j];
                sr -= ur * bRe[j] - ui * bIm[j];
                si -= ur * bIm[j] + ui * bRe[j];
            }
            double dr = re[row + i], di = im[row + i], d = dr * dr + di * di;
            bRe[i] = (sr * dr + si * di) / d;
            bIm[i] = (si * dr - sr * di) / d;
        }
    }
}
