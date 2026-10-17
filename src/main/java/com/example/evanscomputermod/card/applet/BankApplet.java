package com.example.evanscomputermod.card.applet;

import com.example.evanscomputermod.card.Hex;
import org.jetbrains.annotations.Nullable;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.Locale;

/**
 * The bank chip ("keyed ledger response", docs/cards/CARDS.md): the card's silicon, pure
 * over a {@link Storage}. It is the only code that sees {@link BankSecret}, and nothing it
 * returns lets K be recovered.
 *
 * <pre>
 * TAG = HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE(8) || CTR(8 BE) || AMOUNT(8 BE))[0..16)
 * pin_hash = SHA-256("ECM-BANK-PIN" || UID(8) || PIN ascii)
 * </pre>
 * {@link #sign} increments CTR before computing TAG. A PIN session (a verified PIN) lives
 * with the reader, not the card: the caller passes whether one is open. Three failed PINs
 * lock the chip for good; the stripe is untouched by that.
 */
public final class BankApplet {
    public static final int NONCE_BYTES = 8;
    private static final byte[] TAG_LABEL = "ECM-BANK-TAG".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] PIN_LABEL = "ECM-BANK-PIN".getBytes(StandardCharsets.US_ASCII);

    /** Where the chip keeps its components (an item stack, or a test double). */
    public interface Storage {
        long uid();

        /** Give the card a UID when its first applet is written (no-op if it has one). */
        void ensureUid();

        @Nullable BankPublic bankPublic();

        @Nullable BankSecret bankSecret();

        int count();

        void store(BankPublic pub, @Nullable BankSecret secret);
    }

    public record Signature(long ctr, byte[] tag) {
        public String tagHex() {
            return Hex.of(tag);
        }
    }

    private final Storage storage;

    public BankApplet(Storage storage) {
        this.storage = storage;
    }

    public boolean isBlank() {
        return storage.bankPublic() == null;
    }

    public @Nullable BankPublic info() {
        return storage.bankPublic();
    }

    /** Write a blank chip: issuer, PAN, 32-byte key, PIN (4-8 digits, or null for none). Once. */
    public BankPublic write(String issuer, String pan, byte[] key, @Nullable String pin, boolean pinRequired) throws CardException {
        if (!isBlank()) throw new CardException("the bank chip is already written");
        if (storage.count() != 1) throw new CardException("one card at a time");
        if (key == null || key.length != BankSecret.KEY_BYTES) throw new CardException("a bank key is " + BankSecret.KEY_BYTES + " bytes");
        if (pinRequired && pin == null) throw new CardException("pin_required needs a PIN");
        BankPublic pub;
        try {
            pub = new BankPublic(issuer == null ? null : issuer.toLowerCase(Locale.ROOT), pan, pinRequired, 0, 0, false);
            if (pin != null) checkPin(pin);
        } catch (IllegalArgumentException e) {
            throw new CardException(e.getMessage());
        }
        storage.ensureUid();
        byte[] pinHash = pin == null ? null : pinHash(storage.uid(), pin);
        storage.store(pub, BankSecret.of(key, pinHash));
        return pub;
    }

    /**
     * Check a PIN. True opens the caller's session; false counts a failure, and the third
     * locks the chip. Throws when the chip is blank, locked, has no PIN, or has no secret here.
     */
    public boolean verifyPin(String pin) throws CardException {
        BankPublic pub = written();
        BankSecret s = secret();
        byte[] want = s.pinHash();
        if (want == null) throw new CardException("the card has no PIN");
        try {
            checkPin(pin);
        } catch (IllegalArgumentException e) {
            throw new CardException(e.getMessage());
        }
        if (MessageDigest.isEqual(want, pinHash(storage.uid(), pin))) {
            if (pub.pinFails() != 0) storage.store(pub.withFails(0), s);
            return true;
        }
        storage.store(pub.withFails(pub.pinFails() + 1), s);
        return false;
    }

    /** Tries left before the chip locks (0 when locked). */
    public int triesLeft() {
        BankPublic p = storage.bankPublic();
        return p == null || p.locked() ? 0 : BankPublic.MAX_PIN_FAILS - p.pinFails();
    }

    /** Sign {@code nonce} (8 bytes) over {@code amount}: increments CTR, returns {ctr, tag}. */
    public Signature sign(byte[] nonce, long amount, boolean pinSessionOpen) throws CardException {
        BankPublic pub = written();
        BankSecret s = secret();
        if (nonce == null || nonce.length != NONCE_BYTES) throw new CardException("a nonce is " + NONCE_BYTES + " bytes");
        if (amount < 0) throw new CardException("a negative amount");
        if (pub.pinRequired() && !pinSessionOpen) throw new CardException("PIN required: verify_pin first");
        long ctr = pub.ctr() + 1;
        storage.store(pub.withCtr(ctr), s);
        return new Signature(ctr, tag(s.key(), nonce, ctr, amount));
    }

    private BankPublic written() throws CardException {
        BankPublic p = storage.bankPublic();
        if (p == null) throw new CardException("the bank chip is blank");
        if (p.locked()) throw new CardException("the bank chip is locked");
        return p;
    }

    private BankSecret secret() throws CardException {
        BankSecret s = storage.bankSecret();
        if (s == null || !s.known()) throw new CardException("the card has no key");
        return s;
    }

    // ------------------------------------------------------------ pure crypto (shared vectors)

    public static byte[] tag(byte[] key, byte[] nonce, long ctr, long amount) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            mac.update(TAG_LABEL);
            mac.update(nonce);
            mac.update(Hex.longBytes(ctr));
            mac.update(Hex.longBytes(amount));
            return Arrays.copyOf(mac.doFinal(), 16);
        } catch (java.security.GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    public static byte[] pinHash(long uid, String pin) {
        try {
            MessageDigest d = MessageDigest.getInstance("SHA-256");
            d.update(PIN_LABEL);
            d.update(Hex.longBytes(uid));
            d.update(pin.getBytes(StandardCharsets.US_ASCII));
            return d.digest();
        } catch (java.security.GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    public static void checkPin(String pin) {
        if (pin == null || pin.length() < 4 || pin.length() > 8 || !pin.chars().allMatch(Character::isDigit))
            throw new IllegalArgumentException("a PIN is 4-8 digits");
    }
}
