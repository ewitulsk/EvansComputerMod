package com.example.evanscomputermod.speaker;

/**
 * Client side: when one speaker's next audio block plays (DISPLAY_PIPELINE_SPEC.md
 * Part E2, and live audio). Plain Java, so tests drive it without a sound engine.
 *
 * <ul>
 *   <li>A <b>live</b> block ({@link SpeakerAudioPacket#FLAG_LIVE}: a call, a voice
 *       chat) plays at once, whatever the picture's playout delay; the first one
 *       after delayed audio drops that speaker's delayed blocks still queued, so
 *       a call never waits behind video sound.</li>
 *   <li>A <b>delayed</b> block (its computer streams timed frames) plays
 *       {@code lead} from now, with the picture. Right after live audio it is
 *       held at least {@link #RESUME_GAP_US}, a short silence between the two.</li>
 *   <li>Anything else plays at once, as before (beeps, terminal sounds).</li>
 * </ul>
 */
public final class SpeakerSchedule {
    /** The silence before delayed audio after live audio stopped (µs). */
    public static final long RESUME_GAP_US = 20_000;

    /** What to do with a block: drop the queued audio first, and play it {@code leadUs} from now (-1 = at once). */
    public record Plan(boolean dropQueued, long leadUs) {
        public boolean now() {
            return leadUs < 0;
        }
    }

    private boolean live, delayedQueued;

    /**
     * The next block: {@code liveBlock} its live flag, {@code delayUs} the
     * playout delay of its computer's timed stream (-1: none) and
     * {@code leadUs} how far from now its slot is under that delay.
     */
    public Plan next(boolean liveBlock, long delayUs, long leadUs) {
        if (liveBlock) {
            boolean drop = delayedQueued;
            live = true;
            delayedQueued = false;
            return new Plan(drop, -1);
        }
        if (delayUs < 0) {
            live = false;
            delayedQueued = false;
            return new Plan(false, -1);
        }
        long lead = leadUs;
        if (live) {
            live = false;
            lead = Math.max(lead, RESUME_GAP_US);
        }
        delayedQueued = true;
        return new Plan(false, lead);
    }

    /** Is live audio playing (the last block was live)? */
    public boolean live() {
        return live;
    }
}
