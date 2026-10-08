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
}
