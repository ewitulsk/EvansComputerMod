package com.example.evanscomputermod.computer;

import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * DNS settings of the Tech Village ring ({@code <world>/serverconfig/evanscomputermod-dns-server.toml},
 * docs/DNS.md). Static getters fall back to the defaults before the file is loaded (unit
 * tests, early startup), like {@code RadioConfig}.
 */
public final class DnsConfig {

    public static final ModConfigSpec SPEC;

    private static final ModConfigSpec.BooleanValue RESOLVER_ON_ISP_ROUTER;

    /** Test override (GameTests switch modes without a config file); null = the config. */
    public static volatile Boolean resolverOnIspOverride;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        b.comment("The ring's DNS (docs/DNS.md)").push("dns");
        RESOLVER_ON_ISP_ROUTER = b.comment(
                        "Where each Tech Village's recursive resolver runs.",
                        "false: a dedicated computer in the village data center, 100.(64+N).0.53.",
                        "true: the village ISP router runs named itself on 100.(64+N).0.1 and no .53 computers are generated or booted.",
                        "DHCP pools and the carrier core hand out whichever it is. Changing it rewrites the DNS parts of the",
                        "infrastructure configuration at the next start (existing house routers keep what they hand out).")
                .define("resolverOnIspRouter", false);
        b.pop();
        SPEC = b.build();
    }

    private DnsConfig() {}

    public static boolean resolverOnIspRouter() {
        Boolean o = resolverOnIspOverride;
        if (o != null) return o;
        try {
            return SPEC.isLoaded() ? RESOLVER_ON_ISP_ROUTER.get() : false;
        } catch (RuntimeException e) {
            return false;
        }
    }
}
