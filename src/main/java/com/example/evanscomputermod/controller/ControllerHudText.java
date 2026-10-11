package com.example.evanscomputermod.controller;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Text the controller HUD and the terminal's controller button show. Pure, so it is
 * unit-tested. The server's status for a connected controller is
 * {@code "Connected (-54 dBm)"}; once connected the HUD shows the player number and keeps
 * that signal reading next to it instead of dropping it.
 */
public final class ControllerHudText {
    private static final Pattern DBM = Pattern.compile("(-?\\d+(?:\\.\\d+)?) ?dBm");

    private ControllerHudText() {}

    /** The signal reading in a status message ("-54 dBm"), or "" if it has none. */
    public static String signal(String status) {
        if (status == null) return "";
        Matcher m = DBM.matcher(status);
        return m.find() ? m.group(1) + " dBm" : "";
    }

    /** The HUD label: "P1 -54 dBm" while connected (just "P1" before any reading), else the status. */
    public static String hudLabel(int player, String status) {
        if (player <= 0) return status == null ? "" : status;
        String s = signal(status);
        return s.isEmpty() ? "P" + player : "P" + player + " " + s;
    }

    /** The suffix for the terminal's controller button while connected: " (-54 dBm)" or "". */
    public static String buttonSuffix(String status) {
        String s = signal(status);
        return s.isEmpty() ? "" : " (" + s + ")";
    }
}
