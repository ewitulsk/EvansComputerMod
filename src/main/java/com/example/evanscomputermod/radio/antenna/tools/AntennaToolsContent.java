package com.example.evanscomputermod.radio.antenna.tools;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;

/**
 * Antenna tools (lane 4E): the feed point's {@code antenna} peripheral (a
 * computer next to a feed point attaches it) and the analyzer screen packet.
 * Jade/WTHIT tooltips are not wired: neither is a dependency of this build.
 */
public final class AntennaToolsContent {
    private AntennaToolsContent() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterCapabilitiesEvent e) -> e.registerBlock(PeripheralCapability.PERIPHERAL,
                (level, pos, state, be, side) -> new AntennaPeripheral(level, pos), RadioAntennaContent.FEED_POINT.get()));
        modBus.addListener((RegisterPayloadHandlersEvent e) -> AnalyzerPackets.register(e));
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(AntennaPeripheral.TYPE,
                "Antenna on a feed point: SWR, impedance, sweeps, patterns, power limit", AntennaPeripheral.class)));
    }
}
//?}
