package com.example.evanscomputermod.card.nfc;

import com.example.evanscomputermod.card.CardSession;
import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.applet.CardException;
import com.example.evanscomputermod.card.applet.NfcApplet;
import org.jetbrains.annotations.Nullable;

import java.util.Random;

/**
 * A card's side of the air interface: what the silicon answers to each command, over the
 * card's {@link NfcApplet} and a per-reader {@link CardSession}. Pure (the applet's storage
 * is the only state it changes); {@link CardEndpoint} runs it on the medium and the tests
 * drive it directly. Only a card with an NFC applet answers at all.
 */
public final class CardResponder {
    private NfcCard card;
    private final CardSession session = new CardSession();
    private boolean selected;
    private final Random rnd;

    public CardResponder(NfcCard card, Random rnd) {
        this.card = card;
        this.rnd = rnd;
    }

    public NfcCard card() {
        return card;
    }

    /** The same card through a fresh reference (the container reloaded). */
    public void rebind(NfcCard c) {
        if (!c.sameAs(card)) reset();
        card = c;
    }

    /** The field went away or another reader took over: forget sessions. */
    public void reset() {
        session.reset();
        selected = false;
    }

    public boolean selected() {
        return selected;
    }

    /** The reply to {@code f}, or null for silence (not addressed, not an NFC card). */
    public @Nullable NfcAir.Frame answer(NfcAir.Frame f) {
        NfcApplet n = card.nfc();
        if (n == null) return null;
        if (card.uid() == 0) {
            try {
                n.read(0);          // a fresh card materialises its UID and factory sectors on first contact
            } catch (CardException ignored) {
                return null;
            }
        }
        byte[] uid = Hex.longBytes(card.uid());
        switch (f.cmd()) {
            case NfcAir.POLL:
                return NfcAir.frame(NfcAir.POLL | NfcAir.REPLY, uid, new byte[] {(byte) card.kinds()});
            case NfcAir.SELECT:
                if (f.body().length != 8) return null;
                boolean me = java.util.Arrays.equals(f.body(), uid);
                reset();
                if (!me) return null;
                selected = true;
                return NfcAir.frame(NfcAir.SELECT | NfcAir.REPLY, uid);
            default:
                break;
        }
        if (!selected) return null;   // only the selected card answers addressed commands
        try {
            switch (f.cmd()) {
                case NfcAir.AUTH: {
                    if (f.body().length != 9) return NfcAir.error(NfcAir.ERR_MALFORMED);
                    int sector = f.body()[0] & 0xff;
                    if (sector >= 8) return NfcAir.error(NfcAir.ERR_BAD_SECTOR);
                    byte[] r1 = f.slice(1, 8);
                    NfcApplet.Challenge ch = n.challenge(sector, r1, rnd);
                    session.pendingSector = sector;
                    session.pendingR1 = r1;
                    session.pendingR2 = ch.r2();
                    return NfcAir.frame(NfcAir.AUTH | NfcAir.REPLY, ch.r2(), ch.m1());
                }
                case NfcAir.AUTH2: {
                    if (f.body().length != 16 || session.pendingSector < 0) return NfcAir.error(NfcAir.ERR_BAD_AUTH);
                    int sector = session.pendingSector;
                    boolean ok = n.checkM2(sector, session.pendingR1, session.pendingR2, f.body());
                    session.pendingSector = -1;
                    session.pendingR1 = session.pendingR2 = null;
                    if (!ok) return NfcAir.error(NfcAir.ERR_BAD_AUTH);
                    session.open(sector);
                    return NfcAir.frame(NfcAir.AUTH2 | NfcAir.REPLY);
                }
                case NfcAir.READ: {
                    if (f.body().length != 1) return NfcAir.error(NfcAir.ERR_MALFORMED);
                    int sector = f.body()[0] & 0xff;
                    if (sector >= 8) return NfcAir.error(NfcAir.ERR_BAD_SECTOR);
                    if (!session.sectorOpen(sector)) return NfcAir.error(NfcAir.ERR_NO_SESSION);
                    return NfcAir.frame(NfcAir.READ | NfcAir.REPLY, n.read(sector));
                }
                case NfcAir.WRITE: {
                    if (f.body().length != 65) return NfcAir.error(NfcAir.ERR_MALFORMED);
                    int sector = f.body()[0] & 0xff;
                    if (sector >= 8) return NfcAir.error(NfcAir.ERR_BAD_SECTOR);
                    if (!session.sectorOpen(sector)) return NfcAir.error(NfcAir.ERR_NO_SESSION);
                    n.write(sector, f.slice(1, 64));
                    return NfcAir.frame(NfcAir.WRITE | NfcAir.REPLY);
                }
                case NfcAir.SETKEY: {
                    if (f.body().length != 17) return NfcAir.error(NfcAir.ERR_MALFORMED);
                    int sector = f.body()[0] & 0xff;
                    if (sector >= 8) return NfcAir.error(NfcAir.ERR_BAD_SECTOR);
                    if (!session.sectorOpen(sector)) return NfcAir.error(NfcAir.ERR_NO_SESSION);
                    n.setKey(sector, f.slice(1, 16));
                    return NfcAir.frame(NfcAir.SETKEY | NfcAir.REPLY);
                }
                default:
                    return NfcAir.error(NfcAir.ERR_MALFORMED);
            }
        } catch (CardException e) {
            return NfcAir.error(NfcAir.ERR_MALFORMED);
        }
    }
}
