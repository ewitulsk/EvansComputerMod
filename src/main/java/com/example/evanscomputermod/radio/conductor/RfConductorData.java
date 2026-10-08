package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.antenna.graph.CoaxSpec;
import com.example.evanscomputermod.radio.antenna.graph.ConductorSpec;
import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.block.Block;
import net.neoforged.neoforge.registries.datamaps.DataMapType;

/**
 * One entry of the {@code evanscomputermod:rf_conductor} block data map
 * ({@code data/<ns>/data_maps/block/rf_conductor.json}): the electrical side
 * of a conductor, insulator or feedline block. Visual thickness is the
 * block's model; these are the electrical values. Every field is optional,
 * missing ones keep the built-in default for that block.
 *
 * <pre>{@code
 * { "values": {
 *     "evanscomputermod:copper_wire": { "name": "copper wire", "radius_mm": 1.0, "resistivity": 1.68e-8,
 *                                       "current_rating_a": 0.85, "corona_kv": 1.5, "oxidizes": true },
 *     "evanscomputermod:insulator":   { "voltage_rating_kv": 4.0 },
 *     "evanscomputermod:coax_cable":  { "coax_loss_10mhz_db": 0.49, "coax_loss_1ghz_db": 6.6, "max_power_w": 600 } } }
 * }</pre>
 */
public record RfConductorData(String name, double radiusMm, double resistivity, double currentRatingA, double coronaKv,
        boolean oxidizes, double voltageRatingKv, double coaxLoss10MHzDb, double coaxLoss1GHzDb, double maxPowerW) {

    public static final Codec<RfConductorData> CODEC = RecordCodecBuilder.create(i -> i.group(
            Codec.STRING.optionalFieldOf("name", "").forGetter(RfConductorData::name),
            Codec.DOUBLE.optionalFieldOf("radius_mm", -1.0).forGetter(RfConductorData::radiusMm),
            Codec.DOUBLE.optionalFieldOf("resistivity", -1.0).forGetter(RfConductorData::resistivity),
            Codec.DOUBLE.optionalFieldOf("current_rating_a", -1.0).forGetter(RfConductorData::currentRatingA),
            Codec.DOUBLE.optionalFieldOf("corona_kv", -1.0).forGetter(RfConductorData::coronaKv),
            Codec.BOOL.optionalFieldOf("oxidizes", false).forGetter(RfConductorData::oxidizes),
            Codec.DOUBLE.optionalFieldOf("voltage_rating_kv", -1.0).forGetter(RfConductorData::voltageRatingKv),
            Codec.DOUBLE.optionalFieldOf("coax_loss_10mhz_db", -1.0).forGetter(RfConductorData::coaxLoss10MHzDb),
            Codec.DOUBLE.optionalFieldOf("coax_loss_1ghz_db", -1.0).forGetter(RfConductorData::coaxLoss1GHzDb),
            Codec.DOUBLE.optionalFieldOf("max_power_w", -1.0).forGetter(RfConductorData::maxPowerW)
    ).apply(i, RfConductorData::new));

    public static final DataMapType<Block, RfConductorData> TYPE = DataMapType
            .builder(EvansComputerMod.id("rf_conductor"), Registries.BLOCK, CODEC)
            .synced(CODEC, false)
            .build();

    /** This entry laid over a default conductor spec (fields left at -1 keep the default). */
    public ConductorSpec over(ConductorSpec base) {
        try {
            return new ConductorSpec(name.isEmpty() ? base.name() : name,
                    radiusMm > 0 ? radiusMm / 1000 : base.radius(),
                    resistivity >= 0 ? resistivity : base.resistivity(),
                    currentRatingA > 0 ? currentRatingA : base.currentRatingA(),
                    coronaKv > 0 ? coronaKv * 1000 : base.coronaVoltage(),
                    oxidizes || base.oxidizes());
        } catch (IllegalArgumentException e) {
            return base;
        }
    }

    public CoaxSpec over(CoaxSpec base) {
        try {
            return new CoaxSpec(name.isEmpty() ? base.name() : name,
                    coaxLoss10MHzDb > 0 ? coaxLoss10MHzDb : base.lossAt10MHz(),
                    coaxLoss1GHzDb > 0 ? coaxLoss1GHzDb : base.lossAt1GHz(),
                    maxPowerW > 0 ? maxPowerW : base.maxPowerW());
        } catch (IllegalArgumentException e) {
            return base;
        }
    }

    public double voltageOver(double base) {
        return voltageRatingKv > 0 ? voltageRatingKv * 1000 : base;
    }
}
//?}
