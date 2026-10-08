package com.example.evanscomputermod.radio.phys;

import java.util.Arrays;

/**
 * Terrain heights sampled along a path: {@code distancesM[i]} from the
 * transmitter (non-decreasing; the first sample is the transmitter, the last
 * the receiver) and the absolute ground height {@code heightsM[i]} there
 * (metres = blocks). Arrays are not copied; callers must not mutate them later.
 */
public record TerrainProfile(double[] distancesM, double[] heightsM) {
    public TerrainProfile {
        if(distancesM == null || heightsM == null) throw new IllegalArgumentException("null profile");
        if(distancesM.length != heightsM.length) throw new IllegalArgumentException("length mismatch");
        if(distancesM.length < 2) throw new IllegalArgumentException("need at least 2 samples");
        for(int i = 1; i < distancesM.length; i++)
            if(distancesM[i] < distancesM[i - 1]) throw new IllegalArgumentException("distances must be non-decreasing");
    }

    /** Samples spaced evenly over {@code lengthM}. */
    public static TerrainProfile uniform(double lengthM, double... heightsM) {
        double[] d = new double[heightsM.length];
        for(int i = 0; i < d.length; i++) d[i] = d.length == 1 ? 0 : lengthM * i / (d.length - 1);
        return new TerrainProfile(d, heightsM);
    }

    /** Flat ground at one height. */
    public static TerrainProfile flat(double lengthM, double heightM, int samples) {
        double[] h = new double[Math.max(2, samples)];
        Arrays.fill(h, heightM);
        return uniform(lengthM, h);
    }

    public int size() {
        return distancesM.length;
    }

    public double lengthM() {
        return distancesM[distancesM.length - 1] - distancesM[0];
    }
}
