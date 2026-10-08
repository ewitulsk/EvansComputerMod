package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.phys.FreeSpace;
import com.example.evanscomputermod.radio.phys.Ionosphere;
import com.example.evanscomputermod.radio.phys.Materials;
import com.example.evanscomputermod.radio.phys.PathLossModel;
import com.example.evanscomputermod.radio.phys.Polarization;

import org.junit.jupiter.api.Test;

import java.util.HashSet;
import java.util.Set;

/** Path model assembly over a synthetic grid (no Level). */
public class PathTracerTest {

    static final double WIFI = 2.437e9, HF = 7.1e6;
    final PathTracer tracer = new PathTracer();

    PathTracer.Result trace(GridWorld w, double ax, double ay, double az, double bx, double by, double bz, double f) {
        return tracer.trace(w, ax, ay, az, bx, by, bz, f, Polarization.VERTICAL, PathTracer.Sky.NONE, null);
    }

    static GridWorld flat() {
        return new GridWorld(64, RfBlock.DIRT);
    }

    @Test
    void openAirIsFreeSpacePlusGround() {
        PathTracer.Result r = trace(flat(), 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, WIFI);
        assertEquals(0, r.obstructionDb(), 1e-9);
        assertEquals(FreeSpace.lossDb(10, WIFI), r.freeSpaceDb(), 1e-6);
        assertEquals(1.5, r.txHeightM(), 1e-9);
        assertTrue(r.lineOfSight());
        assertEquals("dirt", r.groundName());
        assertTrue(Math.abs(r.excessDb()) < 12, "two-ray excess " + r.excessDb());
    }

    @Test
    void stoneWallCostsTheTableValueAt24GHz() {
        GridWorld open = flat(), wall = flat().fill(5, 64, -3, 5, 68, 3, RfBlock.STONE);
        PathTracer.Result a = trace(open, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, WIFI);
        PathTracer.Result b = trace(wall, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, WIFI);
        double table = Materials.STONE.dbPerBlock(WIFI);
        assertEquals(66.1, table, 0.3);   // 33 dB at 1 GHz x (2.437)^0.78
        assertEquals(table, b.obstructionDb(), 1e-6);
        assertEquals(table, b.totalDb() - a.totalDb(), 1e-6);
        assertFalse(b.lineOfSight());
    }

    @Test
    void diagonalRayPaysForItsLengthInTheBlock() {
        GridWorld wall = flat().fill(5, 64, -10, 5, 70, 10, RfBlock.STONE);
        PathTracer.Result r = trace(wall, 0.5, 65.5, 0.5, 10.5, 65.5, 10.5, WIFI);
        // Crosses the slab x = 5..6 on a 45° run: √2 m of stone, split over the cells it crosses.
        assertEquals(Math.sqrt(2) * Materials.STONE.dbPerBlock(WIFI), r.obstructionDb(), 0.01);
    }

    @Test
    void antennaBlocksThemselvesDoNotCount() {
        GridWorld w = flat().set(0, 65, 0, RfBlock.IRON).set(10, 65, 0, RfBlock.IRON);
        assertEquals(0, trace(w, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, WIFI).obstructionDb(), 1e-9);
    }

    @Test
    void ironRoomIsAFaradayCageGlassBarelyMatters() {
        GridWorld iron = flat().shell(-3, 64, -3, 3, 68, 3, RfBlock.IRON);
        GridWorld glass = flat().shell(-3, 64, -3, 3, 68, 3, RfBlock.GLASS);
        double i = trace(iron, 0.5, 65.5, 0.5, 20.5, 65.5, 0.5, WIFI).obstructionDb();
        double g = trace(glass, 0.5, 65.5, 0.5, 20.5, 65.5, 0.5, WIFI).obstructionDb();
        assertTrue(i >= 79.9, "iron wall " + i);
        assertEquals(Materials.GLASS.dbPerBlock(WIFI), g, 1e-6);
        assertTrue(g < 10, "glass " + g);
        GridWorld two = flat().shell(-3, 64, -3, 3, 68, 3, RfBlock.IRON).shell(17, 64, -3, 23, 68, 3, RfBlock.IRON);
        assertTrue(trace(two, 0.5, 65.5, 0.5, 20.5, 65.5, 0.5, WIFI).obstructionDb() >= 159.9);
    }

    @Test
    void waterBlocks24GHzButNotHF() {
        GridWorld w = flat().fill(4, 64, -3, 6, 68, 3, RfBlock.WATER);
        assertTrue(trace(w, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, WIFI).obstructionDb() > 600);
        assertTrue(trace(w, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, HF).obstructionDb() < 3);
    }

    @Test
    void hfPassesAWallThatBlocks24GHz() {
        GridWorld wall = flat().fill(5, 64, -3, 5, 68, 3, RfBlock.STONE);
        double hf = trace(wall, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, HF).obstructionDb();
        double wifi = trace(wall, 0.5, 65.5, 0.5, 10.5, 65.5, 0.5, WIFI).obstructionDb();
        assertTrue(hf < 2, "HF through stone " + hf);
        assertTrue(wifi - hf > 60);
    }

    @Test
    void middleOfLongPathUsesHeightmapDiffractionNotVoxels() {
        // A 30-high stone ridge in the middle of a 400-block path.
        GridWorld ridge = flat().fill(200, 64, -50, 202, 94, 50, RfBlock.STONE);
        PathTracer.Result wifi = trace(ridge, 0.5, 66, 0.5, 400.5, 66, 0.5, WIFI);
        PathTracer.Result hf = trace(ridge, 0.5, 66, 0.5, 400.5, 66, 0.5, HF);
        assertEquals(0, wifi.obstructionDb(), 1e-9, "voxels only near the ends");
        assertEquals(PathLossModel.Mode.DIFFRACTION, wifi.mode());
        assertTrue(wifi.diffractionDb() > 30, "2.4 GHz is blocked by hills: " + wifi.diffractionDb());
        assertTrue(hf.diffractionDb() < wifi.diffractionDb() - 15, "HF bends round: " + hf.diffractionDb());
        // A wall near an end is voxel-traced exactly.
        GridWorld nearWall = flat().fill(10, 64, -3, 10, 68, 3, RfBlock.STONE);
        assertEquals(Materials.STONE.dbPerBlock(WIFI), trace(nearWall, 0.5, 66, 0.5, 400.5, 66, 0.5, WIFI).obstructionDb(), 1e-6);
    }

    @Test
    void airborneEndsSkipMiddleDiffraction() {
        GridWorld ridge = flat().fill(200, 64, -50, 202, 94, 50, RfBlock.STONE);
        PathTracer.Result r = trace(ridge, 0.5, 200, 0.5, 400.5, 200, 0.5, WIFI);
        assertEquals(0, r.diffractionDb(), 1e-9);
        assertTrue(r.lineOfSight());
    }

    @Test
    void unknownChunksAreTreatedAsOpenTerrain() {
        GridWorld w = flat();
        w.unknownFromX = 100;
        PathTracer.Result r = trace(w, 0.5, 66, 0.5, 300.5, 66, 0.5, WIFI);
        assertTrue(Double.isFinite(r.totalDb()));
        assertEquals(0, r.obstructionDb(), 1e-9);
    }

    @Test
    void vlfLeavesAMineStraightUp() {
        // Antenna 20 blocks down in solid stone, the other on the surface 300 blocks away.
        GridWorld w = new GridWorld(64, RfBlock.STONE);
        double f = 20e3;
        PathTracer.Result r = trace(w, 0.5, 44.5, 0.5, 300.5, 66, 0.5, f);
        assertTrue(r.underground());
        double perBlock = Materials.STONE.dbPerBlock(f);
        assertEquals(19 * perBlock, r.obstructionDb(), 0.5, "vertical exit through 19 blocks");
        assertTrue(r.obstructionDb() < 20, "VLF gets out of the mine: " + r.obstructionDb());
        PathTracer.Result wifi = trace(w, 0.5, 44.5, 0.5, 300.5, 66, 0.5, WIFI);
        assertTrue(wifi.obstructionDb() > 1000, "Wi-Fi does not: " + wifi.obstructionDb());
    }

    /** World position of a hull-local point for {@link GridWorld#volume} placement. */
    static double[] world(double ox, double oy, double oz, double yaw, double lx, double ly, double lz) {
        double c = Math.cos(yaw), s = Math.sin(yaw);
        return new double[] {ox + c * lx - s * lz, oy + ly, oz + s * lx + c * lz};
    }

    @Test
    void subLevelHullShadowsAndIsTracedInItsOwnFrame() {
        double yaw = Math.toRadians(30);
        GridWorld hull = new GridWorld(-1000, RfBlock.AIR).shell(0, 0, 0, 4, 4, 4, RfBlock.IRON);
        GridWorld w = flat();
        PathTracer.Result open = trace(w, 0.5, 66.5, 0.5, 40.5, 66.5, 0.5, WIFI);
        // The hull spans roughly x 16..22 around z = 0 after the rotation.
        w.volumes.add(GridWorld.volume(hull, 18, 64.5, -3, yaw));
        PathTracer.Result shadowed = trace(w, 0.5, 66.5, 0.5, 40.5, 66.5, 0.5, WIFI);
        assertTrue(shadowed.volumeDb() >= 159.9, "two hull walls: " + shadowed.volumeDb());
        assertTrue(shadowed.totalDb() - open.totalDb() >= 159.9);
        // An antenna inside the hull is shadowed by it: its own block is skipped, the wall is not.
        double[] in = world(18, 64.5, -3, yaw, 2.5, 2.5, 2.5);
        PathTracer.Result inside = trace(w, in[0], in[1], in[2], 0.5, 66.5, 0.5, WIFI);
        assertTrue(inside.volumeDb() >= 79.9 && inside.volumeDb() < 160, "one hull wall: " + inside.volumeDb());
    }

    @Test
    void skywaveCarriesHfAtNightOverTheHorizon() {
        GridWorld ridge = flat().fill(1500, 64, -50, 1510, 140, 50, RfBlock.STONE);
        Ionosphere iono = Ionosphere.DEFAULT.withMaxHopBlocks(Ionosphere.DEFAULT.maxHopRealM() / 500);
        PathTracer.Result night = tracer.trace(ridge, 0.5, 70, 0.5, 3000.5, 70, 0.5, HF, Polarization.VERTICAL,
                new PathTracer.Sky(iono, 18000, true), null);
        PathTracer.Result noSky = tracer.trace(ridge, 0.5, 70, 0.5, 3000.5, 70, 0.5, HF, Polarization.VERTICAL,
                new PathTracer.Sky(iono, 18000, false), null);
        assertEquals(PathLossModel.Mode.SKYWAVE, night.mode(), "night F-layer skywave: " + night);
        assertNotEquals(PathLossModel.Mode.SKYWAVE, noSky.mode(), "no sky (Nether/End), no skywave");
        assertTrue(night.totalDb() < noSky.totalDb());
    }

    @Test
    void dependenciesCoverTracedSections() {
        Set<Long> deps = new HashSet<>();
        tracer.trace(flat(), 0.5, 65.5, 0.5, 40.5, 65.5, 0.5, WIFI, Polarization.VERTICAL, PathTracer.Sky.NONE, deps::add);
        assertTrue(deps.contains(Sections.ofBlock(20, 65, 0)));
        assertTrue(deps.contains(Sections.ofBlock(39, 65, 0)));
        long k = Sections.key(-3, -4, 1000);
        assertEquals(-3, Sections.x(k));
        assertEquals(-4, Sections.y(k));
        assertEquals(1000, Sections.z(k));
    }
}
