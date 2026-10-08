package com.example.evanscomputermod.radio.controller;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.RadioEndpoint;

/**
 * A computer module that hears Wireless Controllers: the dedicated Controller
 * Receiver, or the Wi-Fi module in controller mode. Controllers transmit on
 * {@link #channel()}; the module applies what it receives to the computer.
 */
public interface ControllerReceiver {

    RadioEndpoint endpoint();

    Channel channel();

    /** False while a multi-mode module (the Wi-Fi module in Wi-Fi mode) is not listening for controllers. */
    default boolean receivingControllers() {
        return true;
    }
}
