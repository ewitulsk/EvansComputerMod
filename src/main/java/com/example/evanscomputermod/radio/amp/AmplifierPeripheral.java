package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.List;
import java.util.Map;

/**
 * An amplifier as a peripheral (type {@code amplifier}): read-only status for
 * station monitoring programs.
 */
public final class AmplifierPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "amplifier";

    private final AmplifierBlockEntity amp;

    public AmplifierPeripheral(AmplifierBlockEntity amp) {
        this.amp = amp;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @PeripheralMethod(description = "Status: tier_w, transmitting, output_w, drive_w, reflected_w, swr, foldback, temperature_c, fe_per_tick, energy, capacity, supply, antenna_w, bursts")
    public Map<String, Object> status() {
        return amp.status();
    }

    @PeripheralMethod(description = "Warnings for the chain behind this amplifier (hot wire, corona, open feedline, missing arrestor)")
    public List<String> warnings() {
        ExciterLink link = amp.getLevel() == null ? null : ExciterLink.through(amp.getLevel(), amp.getBlockPos());
        return link == null ? List.of("no exciter attached") : link.warnings();
    }
}
//?}
