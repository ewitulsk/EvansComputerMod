package com.example.evanscomputermod.controller;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;

/** The controller HUD keeps the signal reading once connected (it used to show only "P1"). */
public class ControllerHudTextTest {

    @Test
    void connectedLabelShowsPlayerAndSignal() {
        assertEquals("P1 -54 dBm", ControllerHudText.hudLabel(1, "Connected (-54 dBm)"));
        assertEquals("P2", ControllerHudText.hudLabel(2, "Connected"));
        assertEquals("No signal", ControllerHudText.hudLabel(0, "No signal"));
        assertEquals(" (-71 dBm)", ControllerHudText.buttonSuffix("Connected (-71 dBm)"));
        assertEquals("", ControllerHudText.buttonSuffix("Connecting..."));
        assertEquals("", ControllerHudText.signal(null));
    }
}
