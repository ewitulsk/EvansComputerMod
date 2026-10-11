package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RouterScenarios;
import net.minecraft.gametest.framework.*;
import net.neoforged.neoforge.gametest.*;

@GameTestHolder("ecm_router_scenarios")
@PrefixGameTestTemplate(false)
public final class RouterScenarioTests {
  // Real computers use a 55 s wall clock in Scenario; unthrottled GameTest ticks
  // cannot measure elapsed runtime. Each independent lab is its own batch.
  private static void run(GameTestHelper h, String name) {
    TestDriver.scenario(h, "ecm_router_scenarios", RouterScenarios.ALL.get(name));
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.home")
  public static void home(GameTestHelper h) {
    run(h, "router_home");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.forward")
  public static void port_forward(GameTestHelper h) {
    run(h, "router_port_forward");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.static")
  public static void static_routes(GameTestHelper h) {
    run(h, "router_static");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.wan")
  public static void wan_dhcp(GameTestHelper h) {
    run(h, "router_wan_dhcp");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.pair")
  public static void bgp_pair(GameTestHelper h) {
    run(h, "router_bgp_pair");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.ring")
  public static void bgp_ring(GameTestHelper h) {
    run(h, "router_bgp_ring");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.policy")
  public static void bgp_policy(GameTestHelper h) {
    run(h, "router_bgp_policy");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.headless")
  public static void headless(GameTestHelper h) {
    run(h, "router_headless");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.internet")
  public static void internet(GameTestHelper h) {
    run(h, "router_internet");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.fiber")
  public static void fiber(GameTestHelper h) {
    run(h, "router_fiber");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.village")
  public static void village(GameTestHelper h) {
    run(h, "router_village");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.playerfiber")
  public static void player_fiber(GameTestHelper h) {
    run(h, "router_player_fiber");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.chat")
  public static void chat(GameTestHelper h) {
    run(h, "router_chat");
  }

  @GameTest(
      template = TestDriver.STRUCTURE,
      timeoutTicks = TestDriver.BACKSTOP_TICKS,
      batch = "ecm_router_scenarios.fibertap")
  public static void fiber_tap(GameTestHelper h) {
    run(h, "router_fiber_tap");
  }
}
//?}
