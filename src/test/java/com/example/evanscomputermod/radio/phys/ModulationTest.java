package com.example.evanscomputermod.radio.phys;

import static org.junit.jupiter.api.Assertions.*;

import java.util.List;

import org.junit.jupiter.api.Test;

public class ModulationTest {
    @Test
    void erfcReferenceValues() {
        assertEquals(1.0, Special.erfc(0), 1e-7);
        assertEquals(0.157299207, Special.erfc(1), 1e-7);
        assertEquals(1.842700793, Special.erfc(-1), 1e-7);
        assertEquals(4.677734981e-3, Special.erfc(2), 1e-9);
        assertEquals(0.025, Special.q(1.959964), 1e-6);
        assertEquals(3.167124183e-5, Special.q(4.0), 1e-9);
    }

    @Test
    void bpskAt9_6dBIs1e5() {
        double ber = Modulation.BPSK.berAtEbN0(9.6);
        assertEquals(1e-5, ber, 0.2e-5);
        assertEquals(ber, Modulation.QPSK.berAtEbN0(9.6), 1e-15);
        // BPSK SINR equals Eb/N0; QPSK carries 2 bits per symbol
        assertEquals(ber, Modulation.BPSK.berAt(9.6), 1e-15);
        assertEquals(ber, Modulation.QPSK.berAt(9.6 + 3.0103), 1e-8);
    }

    @Test
    void textbookThresholds() {
        // 16-QAM: ~1e-5 near 13.4 dB Eb/N0; DBPSK 1e-5 near 10.3 dB; non-coherent FSK ~13.4 dB
        assertEquals(1e-5, Modulation.QAM16.berAtEbN0(13.4), 0.5e-5);
        assertEquals(1e-5, Modulation.DBPSK.berAtEbN0(10.3), 0.3e-5);
        assertEquals(1e-5, Modulation.FSK.berAtEbN0(13.4), 0.3e-5);
        // denser constellations need more SINR
        Modulation[] order = {Modulation.BPSK, Modulation.QPSK, Modulation.QAM16, Modulation.QAM64, Modulation.QAM256};
        for(int i = 1; i < order.length; i++)
            assertTrue(order[i].requiredSinrDb(1e-5) > order[i - 1].requiredSinrDb(1e-5) + 2, order[i].name());
        assertEquals(9.6, Modulation.BPSK.requiredSinrDb(1e-5), 0.05);
    }

    @Test
    void loraDecodesAtMinus20dB() {
        assertTrue(Modulation.CHIRP_LORA.berAt(-20) < 1e-4, "SF12 at -20 dB: " + Modulation.CHIRP_LORA.berAt(-20));
        assertTrue(Modulation.CHIRP_LORA.berAt(-30) > 0.1);
        assertTrue(Modulation.BPSK.berAt(-20) > 0.3);
        // lower spreading factors need more SNR (SF7 ~ -7.5 dB)
        assertTrue(Modulation.loraBerAtSnr(-20, 7) > 0.1);
        assertTrue(Modulation.loraBerAtSnr(-5, 7) < 1e-3);
        assertEquals(12 * 125e3 / 4096, Modulation.CHIRP_LORA.bitRateBps(125e3), 1e-9);
    }

    @Test
    void berBoundedAndMonotonic() {
        for(Modulation m : Modulation.values()) {
            double prev = 0.5;
            for(double s = -30; s <= 40; s += 0.5) {
                double b = m.berAt(s);
                assertTrue(b >= 0 && b <= 0.5, m + " at " + s);
                assertTrue(b <= prev + 1e-15, m + " not monotonic at " + s);
                prev = b;
            }
        }
    }

    @Test
    void analogAudio() {
        assertEquals(30 + Modulation.FM_IMPROVEMENT_DB, Modulation.FM_ANALOG.audioSnrDb(30), 1e-9);
        assertTrue(Modulation.FM_ANALOG.audioSnrDb(5) < Modulation.FM_ANALOG.audioSnrDb(10) - 10);
        assertEquals(20 - 4.77, Modulation.AM_ANALOG.audioSnrDb(20), 0.01);
        assertTrue(Modulation.FM_ANALOG.isAnalog() && !Modulation.QPSK.isAnalog());
    }

    @Test
    void packetErrorRate() {
        assertEquals(1 - Math.pow(1 - 1e-5, 8000), Per.packetErrorRate(1e-5, 8000), 1e-12);
        assertEquals(0.0769, Per.packetErrorRateBytes(1e-5, 1000), 1e-4);
        assertEquals(1e-12 * 100, Per.packetErrorRate(1e-12, 100), 1e-16);
        assertEquals(0, Per.packetErrorRate(0, 1000));
        assertEquals(0, Per.packetErrorRate(0.1, 0));
        assertEquals(1, Per.packetErrorRate(1, 10));
        assertEquals(Per.lost(0.5, 42), Per.lost(0.5, 42));
        assertFalse(Per.lost(0, 42));
        assertTrue(Per.lost(1, 42));
        int lost = 0;
        for(long s = 0; s < 100_000; s++) if(Per.lost(0.3, s)) lost++;
        assertEquals(0.3, lost / 100_000.0, 0.01);
    }

    @Test
    void wifiRates() {
        List<WifiMcs> ht20 = WifiMcs.ht(20, 1, false);
        assertEquals(6.5, ht20.get(0).rateMbps(), 1e-9);
        assertEquals(65, ht20.get(7).rateMbps(), 1e-9);
        List<WifiMcs> ht40 = WifiMcs.ht(40, 2, true);
        assertEquals(150, ht40.get(7).rateMbps(), 1e-9);
        assertEquals(300, ht40.get(15).rateMbps(), 1e-9);
        assertEquals(16, ht40.size());
        assertEquals(54, WifiMcs.best(WifiMcs.OFDM_11AG, 22).rateMbps());
        assertEquals(6, WifiMcs.best(WifiMcs.OFDM_11AG, 4.5).rateMbps());
        assertNull(WifiMcs.best(WifiMcs.OFDM_11AG, 3));
        assertEquals(1, WifiMcs.best(WifiMcs.DSSS_11B, 0.5).rateMbps());
        assertEquals(11, WifiMcs.best(WifiMcs.DSSS_11B, 30).rateMbps());
        // rate falls monotonically as SINR drops ("iw link" bitrate dropping with distance)
        double prev = Double.MAX_VALUE;
        for(double s = 40; s >= 0; s -= 1) {
            WifiMcs m = WifiMcs.best(ht40, s);
            double r = m == null ? 0 : m.rateMbps();
            assertTrue(r <= prev);
            prev = r;
        }
        assertThrows(IllegalArgumentException.class, () -> WifiMcs.ht(80, 1, false));
    }

    @Test
    void wifiPerWaterfall() {
        WifiMcs m = WifiMcs.OFDM_11AG.get(7);
        assertEquals(0.1, m.packetErrorRate(m.minSinrDb(), 1000), 1e-9);
        assertTrue(m.packetErrorRate(m.minSinrDb() + 5, 1000) < 0.001);
        assertTrue(m.packetErrorRate(m.minSinrDb() - 5, 1000) > 0.9);
        assertTrue(m.packetErrorRate(m.minSinrDb(), 1500) > m.packetErrorRate(m.minSinrDb(), 100));
        assertEquals(0, m.packetErrorRate(10, 0));
        assertEquals(1500 * 8 / 54.0, m.payloadAirtimeUs(1500), 1e-9);
    }
}
