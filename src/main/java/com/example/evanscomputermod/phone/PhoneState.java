package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
/**
 * Power state of an ePhone's computer (docs/gui/DISPLAY_PIPELINE.md §7).
 * <ul>
 *   <li>{@link #AWAKE}: in a player's main hand or off hand. Runs.</li>
 *   <li>{@link #ASLEEP}: in a hotbar slot, not held. The whole computer sleeps
 *       ({@code ComputerInstance.sleep()}); the background scheduler may give it
 *       budgeted refresh wakes ({@link PhoneHost#refreshWake}). Radios stay on.</li>
 *   <li>{@link #OFF}: anywhere else (inventory, chest, ground, dock) or the
 *       battery is empty. Asleep in memory with its modules unloaded and the
 *       host detached; picking it up resumes it. A logout of its holder or a
 *       server stop ends the computer (cold boot next time).</li>
 * </ul>
 */
public enum PhoneState {
    OFF, ASLEEP, AWAKE;

    public String id() {
        return name().toLowerCase(java.util.Locale.ROOT);
    }
}
//?}
