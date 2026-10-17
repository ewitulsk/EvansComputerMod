package com.example.evanscomputermod.card.applet;

/** Why a card applet refused an operation (the message is player-readable). */
public class CardException extends Exception {
    public CardException(String message) {
        super(message);
    }
}
