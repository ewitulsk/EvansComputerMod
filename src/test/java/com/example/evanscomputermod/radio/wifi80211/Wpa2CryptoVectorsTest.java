package com.example.evanscomputermod.radio.wifi80211;

import com.example.evanscomputermod.radio.wifi80211.crypto.AesCcm;
import com.example.evanscomputermod.radio.wifi80211.crypto.AesKeyWrap;
import com.example.evanscomputermod.radio.wifi80211.crypto.Ccmp;
import com.example.evanscomputermod.radio.wifi80211.crypto.PnReplayWindow;
import com.example.evanscomputermod.radio.wifi80211.crypto.Ptk;
import com.example.evanscomputermod.radio.wifi80211.crypto.Wpa2Crypto;
import com.example.evanscomputermod.radio.wifi80211.frame.EapolKey;
import com.example.evanscomputermod.radio.wifi80211.frame.Fcs;
import com.example.evanscomputermod.radio.wifi80211.frame.MacHeader;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.*;

/**
 * WPA2 crypto against IEEE 802.11-2016 Annex J and RFC 3394. The same vectors
 * live in docs/radio/vectors/*.json for the Rust ecm-wifi crate; this test runs
 * both the inline copies and the JSON files so the two cannot drift apart.
 */
class Wpa2CryptoVectorsTest {

    @Test
    void pbkdf2AnnexJ4() {
        assertEquals("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e",
                Hex.of(Wpa2Crypto.pmk("password", "IEEE")));
        assertEquals("0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af",
                Hex.of(Wpa2Crypto.pmk("ThisIsAPassword", "ThisIsASSID")));
        assertEquals("becb93866bb8c3832cb777c2f559807c8c59afcb6eae734885001300a981cc62",
                Hex.of(Wpa2Crypto.pmk("a".repeat(32), "Z".repeat(32))));
    }

    @Test
    void passphraseRules() {
        assertFalse(Wpa2Crypto.isValidPassphrase("short"));
        assertFalse(Wpa2Crypto.isValidPassphrase("x".repeat(64)));
        assertFalse(Wpa2Crypto.isValidPassphrase("tab\there!"));
        assertTrue(Wpa2Crypto.isValidPassphrase("12345678"));
        assertTrue(Wpa2Crypto.isValidPassphrase("x".repeat(63)));
        // 64 hex digits is the raw PSK, used directly as the PMK.
        String raw = "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e";
        assertArrayEquals(Hex.bytes(raw), Wpa2Crypto.pmk(raw, "anything"));
        assertThrows(IllegalArgumentException.class, () -> Wpa2Crypto.pmk("short", "IEEE"));
    }

    @Test
    void prfAnnexJ3() {
        assertEquals("bcd4c650b30b9684951829e0d75f9d54b862175ed9f00606",
                Hex.of(Wpa2Crypto.prf(Hex.bytes("0b".repeat(20)), "prefix", ascii("Hi There"), 192)));
        assertEquals("47c4908e30c947521ad20be9053450ecbea23d3aa604b77326d8b3825ff7475c",
                Hex.of(Wpa2Crypto.prf(ascii("Jefe"), "prefix-2", ascii("what do ya want for nothing?"), 256)));
        assertEquals("0ab6c33ccf70d0d736f4b04c8a7373255511abc5073713163bd0b8c9eeb7e1956fa066820a73ddee3f6d3bd407e0682a",
                Hex.of(Wpa2Crypto.prf(Hex.bytes("aa".repeat(80)), "prefix-3",
                        ascii("Test Using Larger Than Block-Size Key - Hash Key First"), 384)));
        assertEquals("248cfbc532ab38ffa483c8a2e40bf170eb542a2e0916d7bf6d97da2c4c5ca877"
                        + "736c53a65b03fa4b3745ce7613f6ad68e0e4a798b7cf691c96176fd634a59a49",
                Hex.of(Wpa2Crypto.prf(Hex.bytes("0b".repeat(20)), "prefix-4", ascii("Hi There Again"), 512)));
    }

    @Test
    void ptkUsesMinMaxOrderingOfAddressesAndNonces() {
        byte[] pmk = Wpa2Crypto.pmk("password", "IEEE");
        MacAddress aa = MacAddress.parse("02:00:00:00:00:01");
        MacAddress spa = MacAddress.parse("02:00:00:00:00:99");
        byte[] an = Hex.bytes("ff".repeat(32));
        byte[] sn = Hex.bytes("01".repeat(32));
        Ptk p = Wpa2Crypto.derivePtk(pmk, aa, spa, an, sn);
        // Swapping roles must give the same key: both sides sort before hashing.
        assertEquals(p, Wpa2Crypto.derivePtk(pmk, spa, aa, sn, an));
        byte[] data = new byte[76];
        System.arraycopy(aa.bytes(), 0, data, 0, 6);
        System.arraycopy(spa.bytes(), 0, data, 6, 6);
        System.arraycopy(sn, 0, data, 12, 32);
        System.arraycopy(an, 0, data, 44, 32);
        byte[] prf = Wpa2Crypto.prf(pmk, "Pairwise key expansion", data, 384);
        assertArrayEquals(Arrays.copyOfRange(prf, 0, 16), p.kck());
        assertArrayEquals(Arrays.copyOfRange(prf, 16, 32), p.kek());
        assertArrayEquals(Arrays.copyOfRange(prf, 32, 48), p.tk());
    }

    @Test
    void aesKeyWrapRfc3394() {
        String kek128 = "000102030405060708090a0b0c0d0e0f";
        String kek192 = kek128 + "1011121314151617";
        String kek256 = kek128 + "101112131415161718191a1b1c1d1e1f";
        String d128 = "00112233445566778899aabbccddeeff";
        kw(kek128, d128, "1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5");
        kw(kek192, d128, "96778b25ae6ca435f92b5b97c050aed2468ab8a17ad84e5d");
        kw(kek256, d128, "64e8c3f9ce0f5ba263e9777905818a2a93c8191e7d6e8ae7");
        kw(kek192, d128 + "0001020304050607", "031d33264e15d33268f24ec260743edce1c6c7ddee725a936ba814915c6762d2");
        kw(kek256, d128 + "0001020304050607", "a8f9bc1612c68b3ff6e6f4fbe30e71e4769c8b80a32cb8958cd5d17d6b254da1");
        kw(kek256, d128 + "000102030405060708090a0b0c0d0e0f",
                "28c9f404c4b810f4cbccb35cfb87f8263f5786e2d80ed326cbc7f0e71a99f43bfb988b9b7a02dd21");
        // Tampering breaks the integrity check.
        byte[] w = AesKeyWrap.wrap(Hex.bytes(kek128), Hex.bytes(d128));
        w[10] ^= 1;
        assertNull(AesKeyWrap.unwrap(Hex.bytes(kek128), w));
    }

    private static void kw(String kek, String plain, String wrapped) {
        assertEquals(wrapped, Hex.of(AesKeyWrap.wrap(Hex.bytes(kek), Hex.bytes(plain))));
        assertEquals(plain, Hex.of(AesKeyWrap.unwrap(Hex.bytes(kek), Hex.bytes(wrapped))));
    }

    static final String CCMP_TK = "c97c1f67ce371185514a8a19f2bdd52f";
    static final String CCMP_PLAIN = "0848c32c0fd2e128a57c5030f1844408abaea5b8fcba8033"
            + "f8ba1a55d02f85ae967bb62fb6cda8eb7e78a050";
    static final String CCMP_ENC = "0848c32c0fd2e128a57c5030f1844408abaea5b8fcba8033"
            + "0ce70020769703b5"
            + "f3d0a2fe9a3dbf2342a643e43246e80c3c04d019" + "7845ce0b16f97623";

    @Test
    void ccmpAnnexJ64() {
        MacHeader h = MacHeader.parse(Hex.bytes(CCMP_PLAIN));
        assertEquals("08400fd2e128a57c5030f1844408abaea5b8fcba0000", Hex.of(Ccmp.aad(h)));
        assertEquals("005030f1844408b5039776e70c", Hex.of(Ccmp.nonce(h, 0xB5039776E70CL)));
        byte[] enc = Ccmp.encrypt(Hex.bytes(CCMP_TK), Hex.bytes(CCMP_PLAIN), 0xB5039776E70CL, 0);
        assertEquals(CCMP_ENC, Hex.of(enc));
        assertEquals(CCMP_ENC + "1d99f066", Hex.of(Fcs.append(enc)));
        Ccmp.Decrypted d = Ccmp.decrypt(Hex.bytes(CCMP_TK), enc);
        assertNotNull(d);
        assertEquals(0xB5039776E70CL, d.pn());
        assertEquals(0, d.keyId());
        // Decryption clears Protected; everything else of the header is preserved.
        byte[] expectPlain = Hex.bytes(CCMP_PLAIN);
        expectPlain[1] &= ~0x40;
        assertArrayEquals(expectPlain, d.mpdu());
    }

    @Test
    void ccmpRejectsTamperingAndWrongKey() {
        byte[] enc = Hex.bytes(CCMP_ENC);
        assertNull(Ccmp.decrypt(Hex.bytes("00".repeat(16)), enc));
        for (int idx : new int[]{4, 22, 24, 40, enc.length - 1}) { // A1, SC fragment number, PN0, ciphertext, MIC
            byte[] t = enc.clone();
            t[idx] ^= 0x01;
            assertNull(Ccmp.decrypt(Hex.bytes(CCMP_TK), t), "byte " + idx);
        }
        // Retry and sequence number are masked out of the AAD: a retransmission still decrypts.
        byte[] retry = enc.clone();
        retry[1] ^= 0x08;
        retry[23] = 0x44;
        assertNotNull(Ccmp.decrypt(Hex.bytes(CCMP_TK), retry));
    }

    @Test
    void ccmpQosAndFourAddressRoundTrip() {
        MacAddress a = MacAddress.parse("02:00:00:00:00:01"), b = MacAddress.parse("02:00:00:00:00:02");
        MacHeader q = MacHeader.qosData(0x0100, a, b, a, 77, 5);
        MacHeader four = new MacHeader(q.frameControl() | 0x0200, 0, a, b, a, 0x123, b, 6, -1);
        byte[] tk = Hex.bytes("000102030405060708090a0b0c0d0e0f");
        for (MacHeader h : new MacHeader[]{q, four}) {
            byte[] mpdu = Arrays.copyOf(h.encode(), h.length() + 100);
            for (int i = h.length(); i < mpdu.length; i++) mpdu[i] = (byte) i;
            byte[] enc = Ccmp.encrypt(tk, mpdu, 42, 2);
            assertEquals(2, Ccmp.peekKeyId(enc));
            Ccmp.Decrypted d = Ccmp.decrypt(tk, enc);
            assertNotNull(d);
            assertArrayEquals(mpdu, d.mpdu());
            assertEquals(h.tid(), Ccmp.nonce(h, 42)[0]);
        }
        assertEquals(30, Ccmp.aad(four).length);
        assertEquals(24, Ccmp.aad(q).length);
    }

    @Test
    void ccmGenericRfc3610PacketVector1() {
        // RFC 3610 Packet Vector #1 (M = 8, L = 2), checks CCM independent of 802.11.
        byte[] key = Hex.bytes("c0c1c2c3c4c5c6c7c8c9cacbcccdcecf");
        byte[] nonce = Hex.bytes("00000003020100a0a1a2a3a4a5");
        byte[] aad = Hex.bytes("0001020304050607");
        byte[] msg = Hex.bytes("08090a0b0c0d0e0f101112131415161718191a1b1c1d1e");
        byte[] out = AesCcm.encrypt(key, nonce, aad, msg, 8);
        assertEquals("588c979a61c663d2f066d0c2c0f989806d5f6b61dac384" + "17e8d12cfdf926e0", Hex.of(out));
        assertArrayEquals(msg, AesCcm.decrypt(key, nonce, aad, out, 8));
    }

    @Test
    void pnReplayWindowIsStrictlyIncreasingPerTid() {
        PnReplayWindow w = new PnReplayWindow();
        assertTrue(w.accept(0, 1));
        assertFalse(w.accept(0, 1));
        assertTrue(w.accept(3, 1)); // separate TID counter
        assertTrue(w.accept(0, 10));
        assertFalse(w.accept(0, 9));
        w.reset(100);
        assertFalse(w.accept(PnReplayWindow.NON_QOS, 100));
        assertTrue(w.accept(PnReplayWindow.NON_QOS, 101));
    }

    @Test
    void ccmpPnHeaderCodec() {
        byte[] hdr = new byte[8];
        Ccmp.writeHeader(hdr, 0, 0xB5039776E70CL, 0);
        assertEquals("0ce70020769703b5", Hex.of(hdr));
        assertEquals(0xB5039776E70CL, Ccmp.readPn(hdr, 0));
        Ccmp.writeHeader(hdr, 0, Ccmp.PN_MAX, 3);
        assertEquals((byte) 0xE0, hdr[3]);
        assertEquals(Ccmp.PN_MAX, Ccmp.readPn(hdr, 0));
    }

    // ------------------------------------------------------------------ shared JSON fixtures

    @Test
    void sharedJsonVectors() throws IOException {
        int n = 0;
        for (JsonElement e : vectors("pbkdf2.json")) {
            JsonObject v = e.getAsJsonObject();
            assertEquals(v.get("pmk").getAsString(),
                    Hex.of(Wpa2Crypto.pmk(v.get("passphrase").getAsString(), Hex.bytes(v.get("ssid_hex").getAsString()))));
            n++;
        }
        for (JsonElement e : vectors("prf.json")) {
            JsonObject v = e.getAsJsonObject();
            byte[] out = Wpa2Crypto.prf(Hex.bytes(v.get("key").getAsString()), v.get("label").getAsString(),
                    ascii(v.get("data_ascii").getAsString()), v.get("bits").getAsInt());
            assertEquals(v.get("output").getAsString(), Hex.of(out));
            n++;
        }
        for (JsonElement e : vectors("aes_kw.json")) {
            JsonObject v = e.getAsJsonObject();
            kw(v.get("kek").getAsString(), v.get("plaintext").getAsString(), v.get("wrapped").getAsString());
            n++;
        }
        for (JsonElement e : vectors("ccmp.json")) {
            JsonObject v = e.getAsJsonObject();
            byte[] tk = Hex.bytes(v.get("tk").getAsString());
            long pn = Long.parseLong(v.get("pn").getAsString(), 16);
            byte[] plain = Hex.bytes(v.get("plaintext_mpdu").getAsString());
            MacHeader h = MacHeader.parse(plain);
            assertEquals(v.get("aad").getAsString(), Hex.of(Ccmp.aad(h)));
            assertEquals(v.get("nonce").getAsString(), Hex.of(Ccmp.nonce(h, pn)));
            byte[] enc = Ccmp.encrypt(tk, plain, pn, v.get("key_id").getAsInt());
            assertEquals(v.get("encrypted_mpdu").getAsString(), Hex.of(enc));
            assertEquals(v.get("encrypted_mpdu_fcs").getAsString(), Hex.of(Fcs.append(enc)));
            n++;
        }
        for (JsonElement e : vectors("ptk_mic.json")) {
            JsonObject v = e.getAsJsonObject();
            byte[] pmk = Wpa2Crypto.pmk(v.get("passphrase").getAsString(), v.get("ssid").getAsString());
            assertEquals(v.get("pmk").getAsString(), Hex.of(pmk));
            Ptk p = Wpa2Crypto.derivePtk(pmk, MacAddress.of(Hex.bytes(v.get("aa").getAsString())),
                    MacAddress.of(Hex.bytes(v.get("spa").getAsString())), Hex.bytes(v.get("anonce").getAsString()),
                    Hex.bytes(v.get("snonce").getAsString()));
            assertEquals(v.get("kck").getAsString(), Hex.of(p.kck()));
            assertEquals(v.get("kek").getAsString(), Hex.of(p.kek()));
            assertEquals(v.get("tk").getAsString(), Hex.of(p.tk()));
            byte[] unsigned = Hex.bytes(v.get("m2_unsigned").getAsString());
            assertEquals(v.get("m2_mic").getAsString(), Hex.of(Wpa2Crypto.eapolMic(p.kck(), unsigned)));
            // The codec rebuilds the PDU byte for byte, and its MIC input is the unsigned PDU.
            EapolKey k = EapolKey.parse(Hex.bytes(v.get("m2_signed").getAsString()));
            assertEquals(v.get("m2_signed").getAsString(), Hex.of(k.encode()));
            assertArrayEquals(unsigned, k.encodeForMic());
            n++;
        }
        assertEquals(3 + 4 + 6 + 1 + 1, n, "every shared vector must run");
    }

    private static JsonArray vectors(String file) throws IOException {
        String json = Files.readString(Hex.vectorsDir().resolve(file));
        return JsonParser.parseString(json).getAsJsonObject().getAsJsonArray("vectors");
    }

    private static byte[] ascii(String s) {
        return s.getBytes(StandardCharsets.US_ASCII);
    }
}
