package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
/**
 * Where an ePhone's in-hand hotkeys go (UI_PROTOCOL §7: {@code Hotkey} slots 1-6
 * for the open app, plus the shell's "switch app" key). Called on the server
 * thread, in the order the holder pressed them.
 *
 * <p>The compositor installs its own sink with {@link PhoneHotkeys#setFactory}
 * (and turns these into {@code Hotkey}/{@code Chord} events for the open app and
 * the shell). Until then {@link PhoneHotkeys#peripheralEvents} delivers them to
 * programs as peripheral events from the {@code phone} peripheral:
 * {@code phone_hotkey(slot, down)} and {@code phone_switch_app()}.
 */
public interface PhoneHotkeySink {

    /** Hotkey slot {@code slot} (1..6) went down or up. */
    void slot(PhoneHost phone, int slot, boolean down);

    /** The OS-level "switch app" key was pressed. */
    void switchApp(PhoneHost phone);
}
//?}
