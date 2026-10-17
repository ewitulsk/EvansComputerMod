package com.example.evanscomputermod.bluetooth.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.speaker.client.EarbudsAudioClient;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.common.NeoForge;

/** Client side of Bluetooth: stops the AirPods' audio when the phone stops sending. */
public final class BluetoothClient {
    private BluetoothClient() {}

    public static void register() {
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> EarbudsAudioClient.tick());
    }
}
//?}
