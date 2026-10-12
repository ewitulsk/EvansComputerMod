package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;

/** The GUI's display follows the installed panel (UI_PROTOCOL §4 table). */
class UiDisplayTest {

    @Test
    void terminalDisplayFollowsTheTier() {
        UiDisplay t1 = UiDisplay.forTier(PanelTier.PHOSPHOR, UiProto.Device.TERMINAL);
        assertEquals(320, t1.logicalW());
        assertEquals(200, t1.logicalH());
        assertEquals(UiProto.SizeClass.COMPACT, t1.sizeClass());
        assertEquals(1, t1.tier());

        UiDisplay t2 = UiDisplay.forTier(PanelTier.COLOR_LCD, UiProto.Device.TERMINAL);
        assertEquals(UiDisplay.terminal(), t2, "Tier 2 is the pre-panel Terminal display");

        UiDisplay t3 = UiDisplay.forTier(PanelTier.HD, UiProto.Device.TERMINAL);
        assertEquals(1280, t3.deviceW());
        assertEquals(800, t3.deviceH());
        assertEquals(2, t3.scale());
        assertEquals(640, t3.logicalW(), "Tier 3 doubles pixels, not space");
        assertEquals(UiProto.SizeClass.MEDIUM, t3.sizeClass());
    }

    @Test
    void phoneDisplayIsPortrait() {
        UiDisplay p2 = UiDisplay.forTier(PanelTier.COLOR_LCD, UiProto.Device.EPHONE);
        assertEquals(240, p2.logicalW());
        assertEquals(400, p2.logicalH());
        assertEquals(UiProto.SizeClass.COMPACT, p2.sizeClass());
        UiDisplay p3 = UiDisplay.forTier(PanelTier.HD, UiProto.Device.EPHONE);
        assertEquals(480, p3.deviceW());
        assertEquals(240, p3.logicalW());
    }
}
