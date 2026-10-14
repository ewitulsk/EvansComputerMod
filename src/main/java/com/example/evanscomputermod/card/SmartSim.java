package com.example.evanscomputermod.card;

import org.jetbrains.annotations.Nullable;

/**
 * A SIM card seen as a smart card: the API Card Reader/Writers, the phone's SIM tray
 * and (later) {@code celld} use. {@code SimCardItem.card(stack)} gives one for an item
 * stack (and changes that stack in place); the logic here is pure so it is unit-tested.
 *
 * <ul>
 *   <li>{@link #isBlank()}: a new card has no identity and no key.</li>
 *   <li>{@link #identity()}: carrier and IMSI (public, synced to clients).</li>
 *   <li>{@link #respond(byte[])}: the card's answer to a challenge and the session
 *       key material derived from it ({@link SimAuth}). The key itself can't be read.</li>
 *   <li>{@link #write(long, String, byte[])}: give a blank card its identity and key.
 *       Only once: a written card can't be rewritten.</li>
 * </ul>
 *
 * {@code respond} works only on the server (a client never has the key).
 */
public final class SmartSim {

    /** Why a card operation failed (the message is player-readable). */
    public static final class CardException extends Exception {
        public CardException(String message) {
            super(message);
        }
    }

    /** Where a card keeps its two components (an item stack, or a test double). */
    public interface Storage {
        @Nullable SimIdentity identity();

        @Nullable SimSecret secret();

        /** How many cards share this storage (an item stack's count). */
        int count();

        void store(SimIdentity identity, SimSecret secret);
    }

    private final Storage storage;

    public SmartSim(Storage storage) {
        this.storage = storage;
    }

    public boolean isBlank() {
        return storage.identity() == null;
    }

    /** Carrier and IMSI, or null for a blank card. */
    public @Nullable SimIdentity identity() {
        return storage.identity();
    }

    /** The card answers {@code challenge} (16 bytes): RES and the session key material. */
    public SimAuth.Response respond(byte[] challenge) throws CardException {
        if (isBlank()) throw new CardException("the card is blank");
        SimSecret s = storage.secret();
        if (s == null || !s.known()) throw new CardException("the card has no key");
        if (challenge == null || challenge.length != SimAuth.CHALLENGE_BYTES)
            throw new CardException("a challenge is " + SimAuth.CHALLENGE_BYTES + " bytes");
        return SimAuth.respond(s.key(), challenge);
    }

    /** Write a blank card: identity and key. Fails for a written card or a stack of several cards. */
    public SimIdentity write(long imsi, String carrier, byte[] key) throws CardException {
        if (!isBlank()) throw new CardException("the card is not blank: SIM cards can be written only once");
        if (storage.count() != 1) throw new CardException("one card at a time");
        SimIdentity id;
        SimSecret secret;
        try {
            id = new SimIdentity(carrier, imsi);
            secret = SimSecret.of(key);
        } catch (IllegalArgumentException e) {
            throw new CardException(e.getMessage());
        }
        storage.store(id, secret);
        return id;
    }
}
