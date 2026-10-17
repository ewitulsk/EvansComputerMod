package com.example.evanscomputermod.speaker;

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import org.junit.jupiter.api.Test;

import java.util.UUID;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Live speaker audio (calls) against the picture's playout delay: a live
 * block plays at once, the first one after delayed audio drops what is still
 * queued, and delayed scheduling comes back after a short silence. Also the
 * packet's flag on the wire and which stream roles are live.
 */
class SpeakerScheduleTest {

    @Test
    void liveBlocksPlayNowAndDropTheDelayedQueueOnTheSwitch() {
        SpeakerSchedule s = new SpeakerSchedule();
        // Video sound while the computer streams timed frames: held for the picture's delay.
        var p = s.next(false, 150_000, 180_000);
        assertFalse(p.now());
        assertEquals(180_000, p.leadUs());
        assertFalse(p.dropQueued());
        p = s.next(false, 150_000, 200_000);
        assertEquals(200_000, p.leadUs());
        // A call starts: at once, and the delayed audio behind it goes.
        p = s.next(true, 150_000, 300_000);
        assertTrue(p.now(), "live audio ignores the playout delay");
        assertTrue(p.dropQueued(), "the first live block drops the delayed blocks still queued");
        assertTrue(s.live());
        p = s.next(true, 150_000, 300_000);
        assertTrue(p.now());
        assertFalse(p.dropQueued(), "nothing delayed is left to drop");
    }

    @Test
    void delayedAudioResumesAfterAShortGapWhenTheCallEnds() {
        SpeakerSchedule s = new SpeakerSchedule();
        s.next(true, 150_000, 0);
        var p = s.next(false, 150_000, 5_000);
        assertFalse(p.now());
        assertEquals(SpeakerSchedule.RESUME_GAP_US, p.leadUs(), "a short silence between the call and the video sound");
        assertFalse(s.live());
        p = s.next(false, 150_000, 160_000);
        assertEquals(160_000, p.leadUs(), "then the picture's delay again");
        // Without a timed stream everything plays at once, as before (beeps, terminal sounds).
        p = s.next(false, -1, 0);
        assertTrue(p.now());
        assertFalse(p.dropQueued());
    }

    @Test
    void theLiveFlagTravelsAndPhoneAndVoiceAreLiveRoles() {
        SpeakerAudioPacket p = new SpeakerAudioPacket(BlockPos.ZERO, 3, 48000, 4, new byte[]{1, 2, 3, 4}, 1234, UUID.randomUUID(),
                SpeakerAudioPacket.FLAG_LIVE);
        ByteBuf buf = Unpooled.buffer();
        SpeakerAudioPacket.STREAM_CODEC.encode(buf, p);
        SpeakerAudioPacket q = SpeakerAudioPacket.STREAM_CODEC.decode(buf);
        buf.release();
        assertTrue(q.live());
        assertEquals(p.computer(), q.computer());
        assertEquals(1234, q.ptsUs());
        assertFalse(new SpeakerAudioPacket(BlockPos.ZERO, 1, 48000, 1, new byte[]{0}).live());
        assertTrue(SpeakerAudio.isLiveRole("phone"));
        assertTrue(SpeakerAudio.isLiveRole("voice"));
        assertFalse(SpeakerAudio.isLiveRole("media"));
        assertFalse(new SpeakerAudio().lastDrainLive(), "without a mixer nothing is live");
    }
}
