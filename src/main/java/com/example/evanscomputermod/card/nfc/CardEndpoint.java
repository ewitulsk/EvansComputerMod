package com.example.evanscomputermod.card.nfc;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.PassiveEndpoint;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;

import java.util.Random;
import java.util.UUID;
import java.util.function.LongSupplier;

/**
 * A card on the near-field band: a {@link PassiveEndpoint} with no transmitter of its own
 * that, while a reader's field energises it, answers the reader's frames through its
 * {@link CardResponder}. One exists per card within a live field
 * ({@code CardEndpointTracker}); the card reference is refreshed every scan so the
 * applet's changes land on the real item.
 *
 * <p>Wake threshold −45 dBm, backscatter −20 dB: with {@code NearField}'s loss a 0 dBm
 * reader wakes a card to about 1.6 blocks and reads its reply to about 1.1.
 */
public final class CardEndpoint implements PassiveEndpoint {
    public static final double WAKE_DBM = -45;

    private final UUID id = UUID.randomUUID();
    private final LongSupplier clock;
    private volatile Pose pose;
    private volatile double speed;
    private volatile Runnable dirty = () -> {};
    private volatile double fieldDbm = Double.NEGATIVE_INFINITY;
    private volatile long energisedUntil;
    private volatile RadioMedium medium;
    private volatile UUID lastReader;
    private final CardResponder responder;
    /** For tests and the tracker: frames received (any) and answered, and the last frame's RSSI. */
    public volatile long received, answered;
    public volatile double lastRxDbm = Double.NaN;

    public CardEndpoint(NfcCard card, Pose pose, LongSupplier clock, Random rnd) {
        this.pose = pose;
        this.clock = clock;
        this.responder = new CardResponder(card, rnd);
    }

    public NfcCard card() {
        return responder.card();
    }

    /** The tracker found the card again: the live card, where it is now, and how to save it. */
    public void refresh(NfcCard card, Pose pose, double speedMps, Runnable markDirty) {
        responder.rebind(card);
        this.pose = pose;
        this.speed = speedMps;
        this.dirty = markDirty;
    }

    public void attach(RadioMedium m) {
        medium = m;
    }

    @Override
    public UUID id() {
        return id;
    }

    @Override
    public Pose pose() {
        return pose;
    }

    @Override
    public AntennaPattern antenna() {
        return AntennaPattern.ISOTROPIC;
    }

    @Override
    public Channel tunedChannel() {
        return NfcAir.CHANNEL;
    }

    @Override
    public double sensitivityDbm() {
        return WAKE_DBM;
    }

    @Override
    public double speedMps() {
        return speed;
    }

    @Override
    public double maxTxPowerDbm() {
        return energised() ? fieldDbm - backscatterLossDb() : Double.NEGATIVE_INFINITY;
    }

    @Override
    public double wakeThresholdDbm() {
        return WAKE_DBM;
    }

    @Override
    public boolean energised() {
        return clock.getAsLong() < energisedUntil;
    }

    @Override
    public void energise(double fieldPowerDbm, long untilMicros) {
        fieldDbm = fieldPowerDbm;
        energisedUntil = untilMicros;
    }

    @Override
    public void onReceive(Reception r) {
        if (r.emission().kind() != Emission.Kind.FRAME || !energised()) return;
        received++;
        lastRxDbm = r.rssiDbm();
        RadioMedium m = medium;
        if (m == null) return;
        NfcAir.Frame f = NfcAir.decode(r.payload());
        if (f == null) return;
        if (lastReader != null && !lastReader.equals(r.from()) && f.cmd() != NfcAir.POLL) responder.reset();
        lastReader = r.from();
        NfcAir.Frame reply;
        synchronized (responder) {
            reply = responder.answer(f);
        }
        if (reply == null) return;
        dirty.run();
        answered++;
        long start = clock.getAsLong();
        if (f.cmd() == NfcAir.POLL) start += NfcAir.slotOf(card().uid()) * NfcAir.SLOT_MICROS;
        byte[] bytes = reply.encode();
        m.transmit(this, Emission.frame(NfcAir.CHANNEL, maxTxPowerDbm(), start, NfcAir.durationMicros(bytes.length),
                NfcAir.MODULATION, NfcAir.BIT_RATE, bytes));
    }
}
