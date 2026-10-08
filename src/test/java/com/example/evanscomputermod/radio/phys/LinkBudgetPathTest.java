package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

public class LinkBudgetPathTest {
    @Test
    void linkBudgetSums() {
        LinkBudget b = LinkBudget.builder().txPowerDbm(20).txGainDbi(2).rxGainDbi(2).pathLossDb(80).obstructionDb(10)
                .diffractionDb(0).polarizationDb(3).feedLossDb(1).fadeLossDb(0).build();
        assertEquals(-70, b.rxPowerDbm(), 1e-12);
        assertEquals(90, b.netLossDb(), 1e-12);
        assertEquals(30, b.sinrDb(-100, 0), 1e-12);
        // interference equal to the noise costs 3 dB
        assertEquals(30 - 3.0103, b.sinrDb(-100, Units.dbmToMw(-100)), 1e-3);
        assertEquals(30 - 4.7712, b.sinrDb(-100, Units.dbmToMw(-100), Units.dbmToMw(-100)), 1e-3);
        assertEquals(20, b.marginDb(-90), 1e-12);
        assertEquals(30, LinkBudget.builder().txPowerW(1).build().txPowerDbm(), 1e-12);
    }

    @Test
    void wifiLinkEndToEnd() {
        // 20 dBm AP, 2 dBi each side, 50 m, one glass block: SINR in 20 MHz, NF 6 dB -> top OFDM rate
        double f = Band.wifi24ChannelHz(6);
        var path = PathLossModel.evaluate(PathLossModel.Path.of(50, f).withObstructionDb(Materials.GLASS.dbPerBlock(f)));
        LinkBudget b = LinkBudget.builder().txPowerDbm(20).txGainDbi(2).rxGainDbi(2).path(path).build();
        double sinr = b.sinrDb(Noise.thermalDbm(20e6, 6), 0);
        assertEquals(WifiMcs.best(WifiMcs.OFDM_11AG, sinr).rateMbps(), 54);
        // three stone blocks kill it
        var walled = PathLossModel.evaluate(PathLossModel.Path.of(50, f).withObstructionDb(3 * Materials.STONE.dbPerBlock(f)));
        double walledSinr = LinkBudget.builder().txPowerDbm(20).txGainDbi(2).rxGainDbi(2).path(walled).build()
                .sinrDb(Noise.thermalDbm(20e6, 6), 0);
        assertNull(WifiMcs.best(WifiMcs.OFDM_11AG, walledSinr));
    }

    @Test
    void freeSpaceMode() {
        var r = PathLossModel.evaluate(PathLossModel.Path.of(1000, 2.4e9));
        assertEquals(PathLossModel.Mode.FREE_SPACE, r.mode());
        assertEquals(FreeSpace.lossDb(1000, 2.4e9), r.totalDb(), 1e-12);
        var o = PathLossModel.evaluate(PathLossModel.Path.of(1000, 2.4e9).withObstructionDb(12));
        assertEquals(r.totalDb() + 12, o.totalDb(), 1e-12);
        // heights count in the 3-D distance
        var h = PathLossModel.evaluate(PathLossModel.Path.of(30, 2.4e9).withHeights(40, 0));
        assertEquals(FreeSpace.lossDb(50, 2.4e9), h.totalDb(), 1e-9);
    }

    @Test
    void twoRayModePastCrossover() {
        var p = PathLossModel.Path.of(20_000, 900e6).withHeights(10, 2).withGround(Ground.AVERAGE_GROUND,
                Polarization.HORIZONTAL);
        var r = PathLossModel.evaluate(p);
        assertEquals(PathLossModel.Mode.TWO_RAY, r.mode());
        assertTrue(r.groundExcessDb() > 20);
        LinkBudget b = LinkBudget.builder().path(r).build();
        assertEquals(r.totalDb(), b.pathLossDb() + b.obstructionDb() + b.diffractionDb(), 1e-9);
    }

    @Test
    void groundWaveModeAtLf() {
        var p = PathLossModel.Path.of(20_000, 100e3).withHeights(1, 1).withGround(Ground.AVERAGE_GROUND,
                Polarization.VERTICAL);
        assertEquals(PathLossModel.Mode.GROUND_WAVE, PathLossModel.evaluate(p).mode());
    }

    @Test
    void diffractionMode() {
        double f = 2.4e9;
        TerrainProfile hill = TerrainProfile.uniform(1000, 64, 64, 80, 64, 64);
        var r = PathLossModel.evaluate(PathLossModel.Path.of(1000, f).withHeights(2, 2).withTerrain(hill));
        assertEquals(PathLossModel.Mode.DIFFRACTION, r.mode());
        assertEquals(Deygout.lossDb(hill, f, 2, 2), r.diffractionDb(), 1e-12);
        assertEquals(r.freeSpaceDb() + r.diffractionDb(), r.totalDb(), 1e-9);
        // HF around the same hill barely notices
        var hf = PathLossModel.evaluate(PathLossModel.Path.of(1000, 7e6).withHeights(2, 2).withTerrain(hill));
        assertTrue(hf.diffractionDb() < r.diffractionDb() - 10);
    }

    @Test
    void skywaveModeAtNightOnly() {
        var base = PathLossModel.Path.of(20_000, 3e6).withHeights(2, 2).withGround(Ground.DRY_GROUND,
                Polarization.VERTICAL);
        var night = PathLossModel.evaluate(base.withSkywave(Ionosphere.DEFAULT, 18000, true));
        assertEquals(PathLossModel.Mode.SKYWAVE, night.mode());
        assertEquals(night.skywaveDb(), night.totalDb(), 1e-9);
        var day = PathLossModel.evaluate(base.withSkywave(Ionosphere.DEFAULT, 6000, true));
        assertNotEquals(PathLossModel.Mode.SKYWAVE, day.mode());
        var nether = PathLossModel.evaluate(base.withSkywave(Ionosphere.DEFAULT, 18000, false));
        assertNotEquals(PathLossModel.Mode.SKYWAVE, nether.mode());
        assertTrue(nether.totalDb() > night.totalDb() + 10);
    }

    @Test
    void controlZeroDistanceAndPurity() {
        var p = PathLossModel.Path.of(0, 2.4e9);
        assertEquals(0, PathLossModel.lossDb(p), 1e-12);
        var q = PathLossModel.Path.of(1234, 433e6).withHeights(5, 3).withGround(Ground.WET_GROUND, Polarization.VERTICAL);
        assertEquals(PathLossModel.evaluate(q), PathLossModel.evaluate(q));
    }
}
