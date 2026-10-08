package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
/**
 * Insulator: joins bare conductors mechanically (they show an arm into it)
 * but not electrically, so it ends an antenna element or separates two.
 * Rated for a peak voltage (data map {@code voltage_rating_kv}, default
 * 4 kV); the antenna's end voltage per √W against that rating sets the
 * "insulators" power limit.
 */
public class InsulatorBlock extends ConductorBlock {
    public InsulatorBlock(Properties properties) {
        super(properties, Role.INSULATOR, 6, true);
    }
}
//?}
