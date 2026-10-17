package com.example.evanscomputermod.card.applet;

import org.jetbrains.annotations.Nullable;

import java.security.MessageDigest;
import java.util.Random;

/**
 * The NFC applet ("sector lattice", docs/cards/NFC_MIX.md): 8 sectors × 64 bytes, each
 * behind its own 16-byte key, authenticated with a 3-pass {@link NfcMix} handshake:
 * <pre>
 * reader → card: AUTH i, R1          card → reader: R2, M1 = Mix(S_i, R1 || R2)
 * reader → card: M2 = Mix(S_i, R2 || R1)   card: session for sector i open iff M2 matches
 * </pre>
 * Pure over a {@link Storage}; the only code that sees {@link NfcSecret}. A session is the
 * caller's (the reader's, over the air or the contact slot): this class checks and
 * answers, and the caller remembers which sector is open. A blank card has the factory
 * key in every sector; the same silicon answers through a contact reader and over the
 * near-field air.
 */
public final class NfcApplet {
    public interface Storage {
        long uid();

        void ensureUid();

        @Nullable NfcPublic nfcPublic();

        @Nullable NfcSecret nfcSecret();

        void store(NfcPublic pub, NfcSecret secret);
    }

    /** The card's answer to AUTH: R2 and M1. */
    public record Challenge(byte[] r2, byte[] m1) {}

    private final Storage storage;

    public NfcApplet(Storage storage) {
        this.storage = storage;
    }

    public NfcPublic info() {
        NfcPublic p = storage.nfcPublic();
        return p == null ? NfcPublic.FRESH : p;
    }

    /** The secret, materialising the factory state the first time the card is touched. */
    private NfcSecret secret() throws CardException {
        NfcSecret s = storage.nfcSecret();
        if (s == null) {
            storage.ensureUid();
            s = NfcSecret.factory();
            storage.store(info(), s);
        }
        if (!s.known()) throw new CardException("the card's sectors are not known here");
        return s;
    }

    public static void checkSector(int sector) throws CardException {
        if (sector < 0 || sector >= NfcSecret.SECTORS) throw new CardException("sectors are 0-" + (NfcSecret.SECTORS - 1));
    }

    /** Pass 1: the card draws R2 and answers M1 for {@code sector}. */
    public Challenge challenge(int sector, byte[] r1, Random rnd) throws CardException {
        checkSector(sector);
        if (r1 == null || r1.length != 8) throw new CardException("R1 is 8 bytes");
        byte[] r2 = new byte[8];
        rnd.nextBytes(r2);
        byte[] m1 = NfcMix.mix(secret().key(sector), NfcMix.concat(r1, r2));
        return new Challenge(r2, m1);
    }

    /** Pass 3: does the reader's M2 prove it has the sector key? */
    public boolean checkM2(int sector, byte[] r1, byte[] r2, byte[] m2) throws CardException {
        checkSector(sector);
        if (m2 == null || m2.length != 16) return false;
        byte[] want = NfcMix.mix(secret().key(sector), NfcMix.concat(r2, r1));
        return MessageDigest.isEqual(want, m2);
    }

    /** What a reader with the key computes for pass 3 (also the reader side's reference). */
    public static byte[] m2(byte[] key, byte[] r1, byte[] r2) {
        return NfcMix.mix(key, NfcMix.concat(r2, r1));
    }

    public static byte[] m1(byte[] key, byte[] r1, byte[] r2) {
        return NfcMix.mix(key, NfcMix.concat(r1, r2));
    }

    /** Sector data (the caller has an open session for it). */
    public byte[] read(int sector) throws CardException {
        checkSector(sector);
        return secret().data(sector);
    }

    public void write(int sector, byte[] data) throws CardException {
        checkSector(sector);
        if (data == null || data.length != NfcSecret.DATA_BYTES) throw new CardException("a sector is " + NfcSecret.DATA_BYTES + " bytes");
        storage.store(info().mark(sector), secret().withData(sector, data));
    }

    public void setKey(int sector, byte[] key) throws CardException {
        checkSector(sector);
        if (key == null || key.length != NfcSecret.KEY_BYTES) throw new CardException("a sector key is " + NfcSecret.KEY_BYTES + " bytes");
        storage.store(info().mark(sector), secret().withKey(sector, key));
    }
}
