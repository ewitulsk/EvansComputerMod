package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;

/**
 * A compositor's display (§4): device pixels, the scale (device pixels per
 * lu), the logical size apps lay out in, its size class, panel tier and kind.
 */
public record UiDisplay(int deviceW, int deviceH, int scale, int tier, UiProto.Device device) {

    /** A Terminal at today's resolution: 640x400 device pixels, scale 1 (Tier 2, §4 table). */
    public static UiDisplay terminal() {
        return new UiDisplay(640, 400, 1, 2, UiProto.Device.TERMINAL);
    }

    public int logicalW() {
        return deviceW / scale;
    }

    public int logicalH() {
        return deviceH / scale;
    }

    public UiProto.SizeClass sizeClass() {
        int w = logicalW();
        return w < 400 ? UiProto.SizeClass.COMPACT : w < 800 ? UiProto.SizeClass.MEDIUM : UiProto.SizeClass.WIDE;
    }

    public UiProto.DisplayInfo info() {
        return new UiProto.DisplayInfo(logicalW(), logicalH(), scale, sizeClass(), tier, device);
    }
}
