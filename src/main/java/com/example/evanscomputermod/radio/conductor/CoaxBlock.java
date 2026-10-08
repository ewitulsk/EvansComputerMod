package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
/**
 * Feedline: coax, hardline, and the lightning arrestor (an inline coax block
 * bonded to ground). Connects only to other feedline blocks, the feed
 * point's coax port and blocks in {@code #evanscomputermod:rf_coax_ports}
 * (amplifiers, tuners, radios). Loss per tier and frequency comes from the
 * {@code rf_conductor} data map ({@code coax_loss_10mhz_db},
 * {@code coax_loss_1ghz_db}); see {@link Feedline}.
 */
public class CoaxBlock extends ConductorBlock {
    private final boolean grounded;

    public CoaxBlock(Properties properties, float thicknessPx, boolean grounded) {
        super(properties, Role.COAX, thicknessPx, true);
        this.grounded = grounded;
    }

    /** True for the lightning arrestor: lightning down the feedline goes to ground here. */
    public boolean grounded() {
        return grounded;
    }
}
//?}
