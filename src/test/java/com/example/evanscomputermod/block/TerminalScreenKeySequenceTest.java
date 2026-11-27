package com.example.evanscomputermod.block;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;

/** Control-key sequences sent by the Terminal block screen (Vim Visual Block, tags, Esc, alternate buffer). */
class TerminalScreenKeySequenceTest {

    @Test
    void controlLettersAndPunctuationMatchXterm() {
        assertEquals("\u0016", TerminalScreen.keySequence(86, true, false, false), "Ctrl+V");
        assertEquals("\u0000", TerminalScreen.keySequence(50, true, false, false), "Ctrl+2 / Ctrl+@");
        assertEquals("\u001b", TerminalScreen.keySequence(91, true, false, false), "Ctrl+[");
        assertEquals("\u001c", TerminalScreen.keySequence(92, true, false, false), "Ctrl+\\");
        assertEquals("\u001d", TerminalScreen.keySequence(93, true, false, false), "Ctrl+]");
        assertEquals("\u001e", TerminalScreen.keySequence(54, true, false, false), "Ctrl+6 / Ctrl+^");
        assertEquals("\u001f", TerminalScreen.keySequence(45, true, false, false), "Ctrl+- / Ctrl+_");
    }
}
