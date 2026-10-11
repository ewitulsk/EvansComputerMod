package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.phys.FreeSpace;
import com.example.evanscomputermod.radio.phys.Fresnel;
import com.example.evanscomputermod.radio.phys.Ground;
import com.example.evanscomputermod.radio.phys.Ionosphere;
import com.example.evanscomputermod.radio.phys.PathLossModel;
import com.example.evanscomputermod.radio.phys.Polarization;
import com.example.evanscomputermod.radio.phys.TerrainProfile;

import java.util.function.LongConsumer;

/**
 * Builds the path loss of one transmitter→receiver path from an {@link RfWorld}
 * (spec, Propagation physics, "Path hybrid"):
 *
 * <ul>
 *   <li><b>Voxel ends.</b> The {@value #VOXEL_END} blocks of the ray nearest each
 *       antenna are traced block by block (Amanatides–Woo); each block adds
 *       α(f) dB/block × the length of ray inside it. The blocks holding the
 *       antennas themselves are skipped. Short paths are traced whole.</li>
 *   <li><b>Other volumes.</b> Every Sable sub-level whose bounds the ray meets
 *       is traced whole in its own block space, so hulls shadow.</li>
 *   <li><b>Heightmap middle.</b> In between, the MOTION_BLOCKING surface gives a
 *       terrain profile for Deygout diffraction (≤ 3 edges). Terrain in the voxel
 *       ends is held a Fresnel radius under the ray so it is not counted twice.
 *       If both antennas are above the whole middle surface (airships), the
 *       middle diffraction is skipped.</li>
 *   <li><b>Ground.</b> Antenna heights are above the first solid block under
 *       each; the reflecting ground (two-ray, ground wave) is the block under the
 *       path (under the transmitter for short paths, which are usually indoors).</li>
 *   <li><b>Underground</b> (below 30 MHz). An antenna below the surface may instead reach it
 *       straight up through the column; the cheaper of that and the direct ray
 *       is used, which is what lets VLF/LF out of mines (skin-depth loss of the
 *       material via {@link com.example.evanscomputermod.radio.phys.MaterialAttenuation}).</li>
 *   <li><b>Skywave</b> (below 30 MHz) through {@link Ionosphere} when the
 *       dimension has a sky.</li>
 * </ul>
 * One instance per thread: it keeps scratch state.
 */
public final class PathTracer {

    public static final int VOXEL_END = 32;
    /** Most terrain samples in a middle profile. */
    public static final int MAX_SAMPLES = 257;
    private static final int GROUND_SCAN = 48;
    /**
     * The straight-up exit from an underground antenna is only a propagation mode
     * where the surface path then carries the signal on as a ground wave
     * (VLF..HF); above that a roof over an antenna is just another wall.
     */
    public static final double UNDERGROUND_MAX_HZ = 30e6;

    /** Everything the path model found, for the cache and the debug command. */
    public record Result(double distanceM, double freqHz, double freeSpaceDb, double obstructionDb,
                         double volumeDb, double diffractionDb, double groundExcessDb, double skywaveDb,
                         double totalDb, PathLossModel.Mode mode, boolean lineOfSight, boolean underground,
                         double txHeightM, double rxHeightM, String groundName, int cost) {
        /** Loss beyond free space at the traced distance. */
        public double excessDb() {
            return totalDb - freeSpaceDb;
        }
    }

    /** Skywave settings: ionosphere (null = none), the dimension's day time and sky. */
    public record Sky(Ionosphere ionosphere, long dayTime, boolean hasSky) {
        public static final Sky NONE = new Sky(null, 6000, false);
    }

    private int cells;
    private long lastDep;
    private LongConsumer deps;
    private final double[] la = new double[3], lb = new double[3];
    private final java.util.List<RfWorld.RfVolume> vols = new java.util.ArrayList<>();

    public Result trace(RfWorld w, double ax, double ay, double az, double bx, double by, double bz,
                        double freqHz, Polarization pol, Sky sky, LongConsumer depSink) {
        cells = 0;
        lastDep = Long.MIN_VALUE;
        deps = depSink == null ? k -> {} : depSink;
        double dx = bx - ax, dy = by - ay, dz = bz - az;
        double d3 = Math.max(1e-6, Math.sqrt(dx * dx + dy * dy + dz * dz));
        double dh = Math.sqrt(dx * dx + dz * dz);

        // 1. Voxel ends (or the whole ray when short).
        double direct;
        if (d3 <= 2 * VOXEL_END) {
            direct = walk(w, ax, ay, az, bx, by, bz, 0, 1, freqHz, true);
        } else {
            double t = VOXEL_END / d3;
            direct = walk(w, ax, ay, az, bx, by, bz, 0, t, freqHz, true)
                    + walk(w, ax, ay, az, bx, by, bz, 1 - t, 1, freqHz, true);
        }

        // 2. Other volumes (Sable sub-levels) the ray crosses, whole, in their own space.
        // (The box reaches down GROUND_SCAN so a deck under an antenna is found for short paths.)
        vols.clear();
        w.volumes(Math.min(ax, bx) - 1, Math.min(ay, by) - 1 - GROUND_SCAN, Math.min(az, bz) - 1,
                Math.max(ax, bx) + 1, Math.max(ay, by) + 1, Math.max(az, bz) + 1, vols::add);
        double volumeDb = 0;
        for (RfWorld.RfVolume v : vols) {
            v.toLocal(ax, ay, az, la);
            v.toLocal(bx, by, bz, lb);
            volumeDb += walk(v.blocks(), la[0], la[1], la[2], lb[0], lb[1], lb[2], 0, 1, freqHz, true);
        }

        // 3. Underground ends may leave straight up instead.
        int sa = w.surfaceY(floor(ax), floor(az)), sb = w.surfaceY(floor(bx), floor(bz));
        // The ground under each antenna sets its height and the reflecting ground: a change of the
        // surface column (or of the blocks scanned under the antenna, below) must retrace the pair.
        dep(Sections.columnOfBlock(floor(ax), floor(az)));
        dep(Sections.columnOfBlock(floor(bx), floor(bz)));
        boolean underA = sa != RfWorld.UNKNOWN && ay < sa - 1 && earthCover(w, ax, sa, az);
        boolean underB = sb != RfWorld.UNKNOWN && by < sb - 1 && earthCover(w, bx, sb, bz);
        boolean underground = false;
        double obstruction = direct;
        if ((underA || underB) && freqHz < UNDERGROUND_MAX_HZ) {
            double up = (underA ? column(w, ax, ay, az, sa, freqHz) : 0) + (underB ? column(w, bx, by, bz, sb, freqHz) : 0);
            if (up < direct) {
                obstruction = up;
                underground = true;
            }
        }
        obstruction += volumeDb;

        // 4. Ground under each antenna and heights above it.
        double ga = groundUnder(w, ax, ay, az, sa), gb = groundUnder(w, bx, by, bz, sb);
        if (underground && underA) ga = ay - 0.05;
        if (underground && underB) gb = by - 0.05;
        if (Double.isNaN(ga)) ga = Double.isNaN(gb) ? Math.min(ay, by) - 1 : Math.min(gb, ay - 0.05);
        if (Double.isNaN(gb)) gb = Math.min(ga, by - 0.05);
        double txH = Math.max(0.05, ay - ga), rxH = Math.max(0.05, by - gb);

        // 5. Terrain profile (heightmap middle).
        TerrainProfile profile;
        Ground ground;
        String groundName;
        if (dh <= 2 * VOXEL_END) {
            // Short paths (rooms, decks): a sub-level floor under an antenna is its ground (a metal
            // hull reflects like a counterpoise). Long paths reflect off the terrain, not the ship.
            RfBlock gblock = blockBelow(w, ax, ga, az);
            for (RfWorld.RfVolume v : vols) {
                double deck = deckUnder(v, ax, ay, az);
                if (!Double.isNaN(deck) && deck > ga) {
                    ga = deck;
                    gblock = deckBlock;
                }
                deck = deckUnder(v, bx, by, bz);
                if (!Double.isNaN(deck) && deck > gb) gb = deck;
            }
            txH = Math.max(0.05, ay - ga);
            rxH = Math.max(0.05, by - gb);
            profile = new TerrainProfile(new double[] {0, Math.max(dh, 1e-3)}, new double[] {ga, gb});
            ground = gblock == null ? null : gblock.groundOrDefault();
            groundName = gblock == null ? "none" : gblock.name();
        } else {
            int n = (int) Math.min(MAX_SAMPLES, Math.ceil(dh) + 1);
            double[] dist = new double[n], h = new double[n];
            double middleMax = Double.NEGATIVE_INFINITY;
            for (int i = 0; i < n; i++) {
                double t = i / (double) (n - 1);
                double x = ax + dx * t, z = az + dz * t;
                dist[i] = dh * t;
                if (i == 0) { h[i] = ga; continue; }
                if (i == n - 1) { h[i] = gb; continue; }
                int s = w.surfaceY(floor(x), floor(z));
                dep(Sections.columnOfBlock(floor(x), floor(z)));
                double surf = s == RfWorld.UNKNOWN ? ga + (gb - ga) * t : s;
                double fromEnd = Math.min(dist[i], dh - dist[i]);
                if (fromEnd < VOXEL_END) {
                    double line = ay + dy * t;
                    double f1 = Fresnel.radiusM(freqHz, Math.max(1e-3, dist[i]), Math.max(1e-3, dh - dist[i]), 1);
                    surf = Math.min(surf, line - f1);
                } else {
                    middleMax = Math.max(middleMax, surf);
                }
                h[i] = surf;
            }
            if (Math.min(ay, by) > middleMax + 1) {
                // Airborne: nothing in the middle can diffract (spec: skip middle-path diffraction).
                profile = new TerrainProfile(new double[] {0, dh}, new double[] {ga, gb});
            } else {
                profile = new TerrainProfile(dist, h);
            }
            int mx = floor(ax + dx / 2), mz = floor(az + dz / 2);
            int ms = w.surfaceY(mx, mz);
            dep(Sections.columnOfBlock(mx, mz));
            RfBlock gblock = ms == RfWorld.UNKNOWN ? blockBelow(w, ax, ga, az) : w.block(mx, ms - 1, mz);
            ground = gblock == null ? Ground.AVERAGE_GROUND : gblock.groundOrDefault();
            groundName = gblock == null ? "average" : gblock.name();
            cells += n / 4;
        }

        PathLossModel.Path path = new PathLossModel.Path(dh, freqHz, txH, rxH, ground, pol, profile, obstruction,
                sky.ionosphere() != null && freqHz < 30e6 ? sky.ionosphere() : null, sky.dayTime(), sky.hasSky());
        PathLossModel.Result r = PathLossModel.evaluate(path);
        double fs = FreeSpace.lossDb(d3, freqHz);
        double total = r.totalDb();
        boolean los = r.mode() != PathLossModel.Mode.SKYWAVE && r.diffractionDb() < 6 && obstruction < 20;
        return new Result(d3, freqHz, fs, obstruction, volumeDb, r.diffractionDb(), r.groundExcessDb(), r.skywaveDb(),
                total, r.mode(), los, underground, txH, rxH, groundName, 1 + cells / 32);
    }

    /**
     * World height of the top of the first solid block of a volume straight under a point
     * (within {@value #GROUND_SCAN} blocks), or NaN; the block is left in {@link #deckBlock}.
     */
    private double deckUnder(RfWorld.RfVolume v, double x, double y, double z) {
        v.toLocal(x, y, z, la);
        v.toLocal(x, y - GROUND_SCAN, z, lb);
        double t = firstSolid(v.blocks(), la[0], la[1], la[2], lb[0], lb[1], lb[2]);
        return Double.isNaN(t) ? Double.NaN : y - t * GROUND_SCAN;
    }

    private RfBlock deckBlock;

    /** Parameter (0..1) where segment A→B first enters a solid block (A's own block skipped), or NaN. */
    private double firstSolid(RfWorld w, double ax, double ay, double az, double bx, double by, double bz) {
        double dx = bx - ax, dy = by - ay, dz = bz - az;
        int x = floor(ax), y = floor(ay), z = floor(az);
        int ex = x, ey = y, ez = z;
        int stepX = dx > 0 ? 1 : dx < 0 ? -1 : 0, stepY = dy > 0 ? 1 : dy < 0 ? -1 : 0, stepZ = dz > 0 ? 1 : dz < 0 ? -1 : 0;
        double tDx = stepX == 0 ? Double.POSITIVE_INFINITY : Math.abs(1 / dx);
        double tDy = stepY == 0 ? Double.POSITIVE_INFINITY : Math.abs(1 / dy);
        double tDz = stepZ == 0 ? Double.POSITIVE_INFINITY : Math.abs(1 / dz);
        double tMx = stepX == 0 ? Double.POSITIVE_INFINITY : (stepX > 0 ? x + 1 - ax : ax - x) * tDx;
        double tMy = stepY == 0 ? Double.POSITIVE_INFINITY : (stepY > 0 ? y + 1 - ay : ay - y) * tDy;
        double tMz = stepZ == 0 ? Double.POSITIVE_INFINITY : (stepZ > 0 ? z + 1 - az : az - z) * tDz;
        double t = 0;
        for (int guard = 0; t <= 1 && guard < 4096; guard++) {
            if (x != ex || y != ey || z != ez) {
                RfBlock b = w.block(x, y, z);
                cells++;
                if (b != null && b.fraction() >= 0.5 && !b.isAir()) {
                    deckBlock = b;
                    return t;
                }
            }
            if (tMx <= tMy && tMx <= tMz) { t = tMx; x += stepX; tMx += tDx; }
            else if (tMy <= tMz) { t = tMy; y += stepY; tMy += tDy; }
            else { t = tMz; z += stepZ; tMz += tDz; }
        }
        return Double.NaN;
    }

    /** True if the top of a column is earth or water (a roof of glass, wood or a barrier is not "underground"). */
    private static boolean earthCover(RfWorld w, double x, int surface, double z) {
        RfBlock top = w.block(floor(x), surface - 1, floor(z));
        return top != null && top.attenuation().lowFrequencyMedium() != null;
    }

    /** Loss straight up from an underground antenna to the surface. */
    private double column(RfWorld w, double x, double y, double z, int surface, double f) {
        int bx = floor(x), bz = floor(z);
        double loss = 0;
        for (int yy = floor(y) + 1; yy < surface; yy++) {
            RfBlock b = w.block(bx, yy, bz);
            dep(Sections.ofBlock(bx, yy, bz));
            cells++;
            if (b != null) loss += b.lossDb(1, f);
        }
        return loss;
    }

    /** Top of the first solid block under a point, or NaN if none within reach (airborne). */
    private double groundUnder(RfWorld w, double x, double y, double z, int surface) {
        int bx = floor(x), bz = floor(z), top = floor(y);
        for (int yy = top - 1; yy >= Math.max(w.minY(), top - GROUND_SCAN); yy--) {
            RfBlock b = w.block(bx, yy, bz);
            cells++;
            dep(Sections.ofBlock(bx, yy, bz));
            if (b != null && b.fraction() >= 0.5 && !b.isAir()) {
                dep(Sections.ofBlock(bx, yy - 1, bz));   // the block under it is the reflecting ground
                return yy + 1;
            }
        }
        if (surface != RfWorld.UNKNOWN && surface <= y) return surface;
        return Double.NaN;
    }

    private static RfBlock blockBelow(RfWorld w, double x, double groundTop, double z) {
        return w.block(floor(x), (int) Math.floor(groundTop) - 1, floor(z));
    }

    /**
     * Ray-march from parameter t0 to t1 of segment A→B, summing α(f)·length per
     * block; the blocks containing A and B are skipped (the antennas' own blocks).
     */
    private double walk(RfWorld w, double ax, double ay, double az, double bx, double by, double bz,
                        double t0, double t1, double f, boolean skipEnds) {
        double dx = bx - ax, dy = by - ay, dz = bz - az;
        double len = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (len < 1e-9 || t1 <= t0) return 0;
        int ex0 = floor(ax), ey0 = floor(ay), ez0 = floor(az);
        int ex1 = floor(bx), ey1 = floor(by), ez1 = floor(bz);
        double sx = ax + dx * t0, sy = ay + dy * t0, sz = az + dz * t0;
        int x = floor(sx), y = floor(sy), z = floor(sz);
        int stepX = dx > 0 ? 1 : dx < 0 ? -1 : 0, stepY = dy > 0 ? 1 : dy < 0 ? -1 : 0, stepZ = dz > 0 ? 1 : dz < 0 ? -1 : 0;
        double tDx = stepX == 0 ? Double.POSITIVE_INFINITY : Math.abs(1 / dx);
        double tDy = stepY == 0 ? Double.POSITIVE_INFINITY : Math.abs(1 / dy);
        double tDz = stepZ == 0 ? Double.POSITIVE_INFINITY : Math.abs(1 / dz);
        double tMx = stepX == 0 ? Double.POSITIVE_INFINITY : ((stepX > 0 ? x + 1 - sx : sx - x) * tDx) + t0;
        double tMy = stepY == 0 ? Double.POSITIVE_INFINITY : ((stepY > 0 ? y + 1 - sy : sy - y) * tDy) + t0;
        double tMz = stepZ == 0 ? Double.POSITIVE_INFINITY : ((stepZ > 0 ? z + 1 - sz : sz - z) * tDz) + t0;
        double t = t0, loss = 0;
        int guard = 0;
        while (t < t1 && guard++ < 100_000) {
            double next = Math.min(t1, Math.min(tMx, Math.min(tMy, tMz)));
            boolean own = skipEnds && (x == ex0 && y == ey0 && z == ez0 || x == ex1 && y == ey1 && z == ez1);
            if (!own && next > t) {
                RfBlock b = w.block(x, y, z);
                cells++;
                dep(Sections.ofBlock(x, y, z));
                if (b != null && b.fraction() > 0) loss += b.lossDb((next - t) * len, f);
            }
            t = next;
            if (t >= t1) break;
            if (tMx <= tMy && tMx <= tMz) { x += stepX; tMx += tDx; }
            else if (tMy <= tMz) { y += stepY; tMy += tDy; }
            else { z += stepZ; tMz += tDz; }
        }
        return loss;
    }

    private void dep(long key) {
        if (key != lastDep) {
            lastDep = key;
            deps.accept(key);
        }
    }

    static int floor(double v) {
        return (int) Math.floor(v);
    }
}
