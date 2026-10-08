package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.phys.Modulation;
import com.example.evanscomputermod.radio.phys.Per;
import com.example.evanscomputermod.radio.phys.SpectralMask;
import com.example.evanscomputermod.radio.phys.WifiMcs;

import java.util.List;
import java.util.Locale;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Maps the CONTRACTS modulation names to packet error rate and spectral mask:
 * {@code DSSS-1/2}, {@code CCK-5.5/11} and {@code OFDM-6..54} and {@code HT-MCSn}
 * through {@link WifiMcs}; {@code CHIRP-SFn} (LoRa), {@code AFSK1200}, {@code FSK},
 * {@code BPSK}, {@code QPSK}, {@code OOK} through {@link Modulation} BER with
 * E<sub>b</sub>/N<sub>0</sub> = SINR · B / R; {@code CTRL} and anything unknown
 * through {@link BasicRadioMedium#packetErrorRate}. Parsed once per name.
 */
public final class PerModels {

    /** PER for one frame. */
    public interface Model {
        double per(double sinrDb, int bits, double bandwidthHz, double bitRate);

        SpectralMask mask();
    }

    private static final ConcurrentHashMap<String, Model> CACHE = new ConcurrentHashMap<>();
    private static final List<WifiMcs> HT20 = WifiMcs.ht(20, 4, false);

    private PerModels() {}

    public static Model of(String modulation) {
        String key = modulation == null ? "" : modulation;
        Model m = CACHE.get(key);
        if (m == null) {
            m = parse(key);
            CACHE.put(key, m);
        }
        return m;
    }

    public static double per(String modulation, double sinrDb, int bits, double bandwidthHz, double bitRate) {
        return of(modulation).per(sinrDb, bits, bandwidthHz, bitRate);
    }

    private static Model parse(String name) {
        String m = name.toUpperCase(Locale.ROOT);
        try {
            if (m.startsWith("DSSS-") || m.startsWith("CCK-")) {
                double rate = Double.parseDouble(m.substring(m.indexOf('-') + 1));
                for (WifiMcs w : WifiMcs.DSSS_11B) if (w.rateMbps() == rate) return wifi(w, SpectralMask.DSSS_22MHZ);
            }
            if (m.startsWith("OFDM-")) {
                double rate = Double.parseDouble(m.substring(5));
                for (WifiMcs w : WifiMcs.OFDM_11AG) if (w.rateMbps() == rate) return wifi(w, SpectralMask.OFDM_20MHZ);
            }
            if (m.startsWith("HT-MCS")) {
                int mcs = Integer.parseInt(m.substring(6).replaceAll("[^0-9].*", ""));
                if (mcs >= 0 && mcs < HT20.size()) return wifi(HT20.get(mcs), SpectralMask.OFDM_20MHZ);
            }
            if (m.startsWith("CHIRP-SF")) {
                int sf = Integer.parseInt(m.substring(8).replaceAll("[^0-9].*", ""));
                return model((s, bits, bw, r) -> Per.packetErrorRate(Modulation.loraBerAtSnr(s, sf), bits));
            }
            if (m.startsWith("AFSK")) {
                String digits = m.substring(4).replaceAll("[^0-9].*", "");
                double nominal = digits.isEmpty() ? 1200 : Double.parseDouble(digits);
                return digital(Modulation.AFSK, nominal);
            }
            if (m.startsWith("FSK")) return digital(Modulation.FSK, 0);
            if (m.startsWith("BPSK")) return digital(Modulation.BPSK, 0);
            if (m.startsWith("QPSK")) return digital(Modulation.QPSK, 0);
            if (m.startsWith("OOK")) return digital(Modulation.OOK, 0);
        } catch (RuntimeException ignored) {
            // malformed name: fall through to the logistic model
        }
        return model((s, bits, bw, r) -> BasicRadioMedium.packetErrorRate(name, s, bits));
    }

    private interface Fn {
        double per(double sinrDb, int bits, double bandwidthHz, double bitRate);
    }

    private static Model model(Fn fn) {
        return new Model() {
            @Override public double per(double s, int bits, double bw, double r) { return fn.per(s, bits, bw, r); }
            @Override public SpectralMask mask() { return SpectralMask.GENERIC; }
        };
    }

    private static Model wifi(WifiMcs w, SpectralMask mask) {
        return new Model() {
            @Override public double per(double s, int bits, double bw, double r) {
                return w.packetErrorRate(s, Math.max(1, (bits + 7) / 8));
            }
            @Override public SpectralMask mask() { return mask; }
        };
    }

    private static Model digital(Modulation mod, double nominalRate) {
        return model((s, bits, bw, r) -> {
            double rate = r > 0 ? r : nominalRate;
            double ebN0 = rate > 0 && bw > 0 ? s + 10 * Math.log10(bw / rate) : s - 10 * Math.log10(mod.bitsPerSymbol());
            return Per.packetErrorRate(mod.berAtEbN0(ebN0), Math.max(8, bits));
        });
    }
}
