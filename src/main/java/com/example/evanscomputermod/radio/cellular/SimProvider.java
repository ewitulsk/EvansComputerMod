package com.example.evanscomputermod.radio.cellular;

/**
 * Where a modem module gets its SIM card. In a Terminal the modem keeps the SIM
 * in its own slot (right-click the installed modem with a SIM card). A host
 * with a dedicated SIM tray (the ePhone) implements this on its module host
 * ({@code IModuleHost}), or hands one to {@code FramedModemModule.setSimProvider};
 * the modem's {@code sim} access then reads the tray on every call, so pulling the
 * SIM out of the phone ends celld's session at once.
 */
@FunctionalInterface
public interface SimProvider {

    /** The SIM currently inserted, or null when the tray is empty. */
    SimCard sim();
}
