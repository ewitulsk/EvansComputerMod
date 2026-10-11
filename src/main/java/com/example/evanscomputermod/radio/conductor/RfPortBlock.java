package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.state.BlockState;

/**
 * A device in {@code #evanscomputermod:rf_coax_ports} whose coax jacks sit on
 * particular faces (relative to its facing). Coax only draws an arm to, and
 * the transmit chain only passes through, a face with a jack: what the model
 * shows is what connects.
 *
 * <p>Signal direction: the chain runs exciter to antenna, entering a device
 * through an {@linkplain #rfInput input} jack and leaving through an
 * {@linkplain #rfOutput output} jack (an amplifier's RF IN / RF OUT, a
 * tuner's TX / ANT). An exciter's jacks are all outputs.
 */
public interface RfPortBlock {
    /** True if {@code state} has a jack that signal enters through on {@code face}. */
    boolean rfInput(BlockState state, Direction face);

    /** True if {@code state} has a jack that signal leaves through on {@code face}. */
    boolean rfOutput(BlockState state, Direction face);

    /** True if {@code face} has any coax jack (what a coax arm may plug into). */
    default boolean hasRfPort(BlockState state, Direction face) {
        return rfInput(state, face) || rfOutput(state, face);
    }
}
//?}
