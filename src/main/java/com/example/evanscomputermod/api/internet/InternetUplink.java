package com.example.evanscomputermod.api.internet;

/**
 * An open link from the Internet Gateway to the real network. Frames leave
 * the game through {@link #sendFrame}; frames from outside come back through
 * the injector the {@link InternetUplinkProvider} was opened with.
 */
public interface InternetUplink extends AutoCloseable {
    /** Offer one Ethernet frame from a segment that reaches an Internet Gateway. Must not block. */
    void sendFrame(byte[] frame);

    /** Release the link (sockets, helper processes). Called when the server stops. */
    @Override
    void close();
}
