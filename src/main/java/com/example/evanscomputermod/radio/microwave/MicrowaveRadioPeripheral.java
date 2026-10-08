package com.example.evanscomputermod.radio.microwave;

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;

import java.util.Map;

/**
 * A microwave radio as a peripheral (type {@code microwave_radio}). Two radios
 * link when their dishes see each other and they share band, channel width
 * and channel number.
 */
public final class MicrowaveRadioPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "microwave_radio";

    private final MicrowaveLink link;
    private final Runnable onChange;

    public MicrowaveRadioPeripheral(MicrowaveLink link, Runnable onChange) {
        this.link = link;
        this.onChange = onChange;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    private void run(Runnable r) throws PeripheralException {
        try {
            r.run();
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
        onChange.run();
    }

    @PeripheralMethod(description = "Band in GHz: 10, 24 or 60")
    public void set_band(int ghz) throws PeripheralException {
        run(() -> link.setBand(MwBand.of(ghz)));
    }

    @PeripheralMethod(description = "Channel number within the band (0 to count-1 for the width)")
    public void set_channel(int channel) throws PeripheralException {
        run(() -> link.setChannelNumber(channel));
    }

    @PeripheralMethod(description = "Channel width, MHz (10 GHz: 28/56, 24 GHz: 28/56/112, 60 GHz: 250/500/1000/2000)")
    public void set_bandwidth(int mhz) throws PeripheralException {
        run(() -> link.setWidthMhz(mhz));
    }

    @PeripheralMethod(description = "Transmit power, dBm (-40 to 30)")
    public void set_tx_power(double dbm) throws PeripheralException {
        run(() -> link.setTxPowerDbm(dbm));
    }

    @PeripheralMethod(mainThread = false, description = "Status: band, channel, frequency, dish gain, linked, rssi_dbm, sinr_db, modulation, rate_mbps, atmosphere_db, counters")
    public Map<String, Object> info() {
        return link.status();
    }
}
