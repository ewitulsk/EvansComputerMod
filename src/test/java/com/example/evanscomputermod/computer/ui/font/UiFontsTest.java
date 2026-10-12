package com.example.evanscomputermod.computer.ui.font;

import com.example.evanscomputermod.computer.ui.proto.UiProto;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

/** The Java font tables match the generator's hash and the metrics the Rust toolkit lays out with. */
class UiFontsTest {
    @Test
    void tablesMatchTheirHash() {
        assertEquals(UiFonts.METRICS_HASH, UiFonts.computeMetricsHash());
    }

    @Test
    void sizesAndMetricsMatchTheRustTables() {
        // Same expectations as rust/crates/ecm-ui-proto/src/fonts.rs tests.
        assertEquals("ui-small", UiFonts.face(UiProto.Font.UI, 8).design().name);
        assertEquals(2, UiFonts.face(UiProto.Font.UI, 24).scale());
        assertEquals(4, UiFonts.face(UiProto.Font.UI, 32).scale());
        assertNull(UiFonts.face(UiProto.Font.UI, 10));
        assertNull(UiFonts.face(UiProto.Font.MONO, 12));
        UiFonts.Face r = UiFonts.face(UiProto.Font.UI, 12);
        assertEquals(8, r.textWidth("Hi"));
        assertEquals(12, r.lineHeight());
        assertEquals(9, r.ascent());
        assertEquals(48, UiFonts.face(UiProto.Font.MONO, 32).textWidth("abc"));
        assertEquals(8, UiFonts.face(UiProto.Font.MONO, 16).advance(0x4E2D)); // missing glyph keeps the cell
        for (int size : new int[] {8, 12}) {
            UiFonts.Design d = UiFonts.face(UiProto.Font.UI, size).design();
            for (int c = 0x20; c <= 0xFF; c++) {
                if (c < 0x7F || c >= 0xA0) assertTrue(d.has(c), d.name + " lacks U+" + Integer.toHexString(c));
            }
            assertFalse(d.has(0x4E2D));
        }
        // Astral code points count as one glyph.
        assertEquals(r.advance(0x1F600), r.textWidth("😀"));
    }
}
