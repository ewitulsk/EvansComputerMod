package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.DnsScenarios;
import net.minecraft.gametest.framework.*;
import net.neoforged.neoforge.gametest.*;

/**
 * DNS (docs/DNS.md): the {@code /ecm scenario} definitions of {@link DnsScenarios} run as
 * tests, one batch each.
 */
@GameTestHolder("ecm_dns")
@PrefixGameTestTemplate(false)
public final class DnsTests {
  private static void run(GameTestHelper h, String name) {
    TestDriver.scenario(h, "ecm_dns", DnsScenarios.ALL.get(name));
  }

  @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = "ecm_dns.lab")
  public static void dns_lab(GameTestHelper h) {
    run(h, "dns_lab");
  }
}
//?}
