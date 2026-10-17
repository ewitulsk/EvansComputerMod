package com.example.evanscomputermod.command;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.*;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.IntegerArgumentType;
import net.minecraft.commands.*;
import net.minecraft.network.chat.Component;

public final class TechNetworkCommand {
  public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
    var root = Commands.literal("ecm").requires(s -> s.hasPermission(2));
    var village = Commands.literal("techvillage");
    village.then(
        Commands.literal("list")
            .executes(
                c -> {
                  var d = data(c.getSource());
                  for (var v : d.villages)
                    say(
                        c.getSource(),
                        "Village "
                            + v.number()
                            + " AS "
                            + (65000 + v.number())
                            + " ("
                            + v.style()
                            + ") at "
                            + v.x()
                            + ", "
                            + v.z()
                            + "; fiber endpoint y "
                            + v.endY());
                  return d.villages.size();
                }));
    for (String action : new String[] {"tp", "info"})
      village.then(
          Commands.literal(action)
              .then(
                  Commands.argument("number", IntegerArgumentType.integer(1, 10))
                      .executes(
                          c -> {
                            var s = c.getSource();
                            int i = IntegerArgumentType.getInteger(c, "number");
                            var v = data(s).villages.get(i - 1);
                            if (action.equals("tp")) {
                              var p = s.getPlayerOrException();
                              var l = s.getServer().overworld();
                              com.example.evanscomputermod.worldgen.TechVillageLocator.Visit visit;
                              try {
                                visit =
                                    com.example.evanscomputermod.worldgen.TechVillageLocator.visit(
                                        l, i);
                              } catch (IllegalStateException e) {
                                s.sendFailure(Component.literal(e.getMessage()));
                                return 0;
                              }
                              p.teleportTo(
                                  l,
                                  visit.arrival().getX() + 0.5,
                                  visit.arrival().getY(),
                                  visit.arrival().getZ() + 0.5,
                                  visit.yaw(),
                                  0);
                            } else {
                              var d = data(s);
                              var l = s.getServer().overworld();
                              int prev = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1;
                              say(s, "Village " + i + " (" + v.style() + "): AS " + (65000 + i) + ", ISP at "
                                  + v.x() + ", " + v.z());
                              say(s, "  village cable (ISP eth0): 100." + (64 + i)
                                  + ".1.0/24, gateway .1, DHCP .10-.200: plug any computer into it");
                              say(s, "  data center LAN (ISP eth1): 100." + (64 + i) + ".0.0/24; web server 100."
                                  + (64 + i) + ".0.10" + (i == d.chatVillage() ? ", chat server "
                                  + WorldNetwork.chatAddress(i) + ":" + WorldNetwork.CHAT_PORT : "")
                                  + "; free racks DHCP .100-.199");
                              say(s, "  fiber eth2 -> village " + prev + " (172.31." + prev + ".2/26, open peering): panel "
                                  + v.panel(false).toShortString() + " (" + v.side(false).getSerializedName()
                                  + " of the mast), " + d.endState(l, i, false));
                              say(s, "  fiber eth3 -> village " + next + " (172.31." + i + ".1/26, open peering): panel "
                                  + v.panel(true).toShortString() + " (" + v.side(true).getSerializedName()
                                  + " of the mast), " + d.endState(l, i, true));
                              say(s, "  taps: put a Fiber Patch Panel against a span, use a free .3-.14 in that link (/26: cell sites use .16 up),"
                                  + " peer with 172.31." + prev + ".2 / 172.31." + i + ".1 (AS " + (65000 + i) + ")");
                              say(s, "  chat server of the ring: village " + d.chatVillage() + ", "
                                  + WorldNetwork.chatAddress(d.chatVillage()) + " port " + WorldNetwork.CHAT_PORT);
                              var repo = d.repo(l);
                              say(s, "  app repository " + com.example.evanscomputermod.computer.VillageRepo.NAME + ": village " + repo.village()
                                  + ", " + repo.url() + " (key " + repo.publicKey().substring(0, 16) + "..., pinned on village computers and store phones)");
                              var cells = d.cells(l);
                              say(s, "  cellular: " + cells.displayName() + " (carrier " + cells.carrier + ", LTE channel " + cells.channel
                                  + "), core in village " + cells.coreVillage + " at " + cells.coreAddress() + ", phones get "
                                  + com.example.evanscomputermod.computer.CellPlan.POOL);
                              say(s, "  Phone Store: cell site " + com.example.evanscomputermod.computer.CellPlan.storeSiteId(i) + " (cell " + i
                                  + ", " + com.example.evanscomputermod.computer.CellPlan.villageLan(i, com.example.evanscomputermod.computer.CellPlan.STORE_HOST)
                                  + "), " + com.example.evanscomputermod.computer.CellPlan.PHONES + " phones on display and a chest of "
                                  + com.example.evanscomputermod.computer.CellPlan.SIMS + " SIM cards");
                              var spans = cells.spans(i - 1);
                              StringBuilder sb = new StringBuilder();
                              for (var sp : spans)
                                sb.append(sb.length() == 0 ? "" : ", ").append(sp.id()).append(" ").append(sp.x()).append(" ").append(sp.z());
                              say(s, "  cell sites toward village " + next + " (" + spans.size() + ", 172.31." + i + ".16 up): " + sb);
                              var dns = d.dns(l);
                              say(s, "  DNS: v" + i + ".ring (search domain), http://web.v" + i + ".ring/; this village runs:");
                              for (String r : dns.describe(i)) say(s, "    " + r);
                              say(s, "  router " + d.identity(l, i, WorldNetwork.ISP_ROUTER) + ", web "
                                  + d.identity(l, i, WorldNetwork.WEB));
                              for (String r : dns.roles(i))
                                say(s, "  " + r + " " + d.identity(l, i, r));
                            }
                            return 1;
                          })));
    root.then(village);
    var network = Commands.literal("net");
    network.then(
        Commands.literal("links")
            .executes(
                c -> {
                  var d = data(c.getSource());
                  var level = c.getSource().getServer().overworld();
                  for (int i = 1; i <= 10; i++)
                    for (String line : d.describeChord(level, i)) say(c.getSource(), line);
                  var mgr = CableNetworkManager.getInstance();
                  if (mgr != null) say(c.getSource(), "topology: " + mgr.stats());
                  return 10;
                }));
    for (String action : new String[] {"cut", "repair"})
      network.then(
          Commands.literal(action)
              .then(
                  Commands.argument("a", IntegerArgumentType.integer(1, 10))
                      .then(
                          Commands.argument("b", IntegerArgumentType.integer(1, 10))
                              .executes(
                                  c -> {
                                    int a = IntegerArgumentType.getInteger(c, "a"),
                                        b = IntegerArgumentType.getInteger(c, "b");
                                    if (Math.abs(a - b) != 1 && Math.abs(a - b) != 9) {
                                      say(
                                          c.getSource(),
                                          "Those villages" + " are not" + " adjacent.");
                                      return 0;
                                    }
                                    data(c.getSource())
                                        .link(
                                            c.getSource().getServer().overworld(),
                                            a,
                                            b,
                                            action.equals("repair"));
                                    say(
                                        c.getSource(),
                                        WorldNetwork.linkName(a, b)
                                            + " "
                                            + action
                                            + "; physical"
                                            + " breaks"
                                            + " must"
                                            + " also be"
                                            + " repaired"
                                            + " (an admin cut is a break at the chord's midpoint).");
                                    return 1;
                                  }))));
    root.then(network);
    root.then(
        Commands.literal("headless")
            .then(
                Commands.literal("list")
                    .executes(
                        c -> {
                          int count = 0;
                          for (var h : ComputerHost.list(c.getSource().getServer()))
                            if (h.isHeadless()) {
                              say(
                                  c.getSource(),
                                  h.getComputerId()
                                      + (h.isInfrastructure() ? " infrastructure" : " always-on"));
                              count++;
                            }
                          return count;
                        })));
    // /ecm dns roles: the whole DNS map (docs/DNS.md).
    root.then(
        Commands.literal("dns")
            .then(
                Commands.literal("roles")
                    .executes(
                        ctx -> {
                          var s = ctx.getSource();
                          var dns = data(s).dns(s.getServer().overworld());
                          say(s, "Tech Village DNS (docs/DNS.md):");
                          for (String line : dns.map()) say(s, "  " + line);
                          for (int v = 1; v <= 10; v++) {
                            java.util.List<String> roles = new java.util.ArrayList<>();
                            for (String r : dns.roles(v)) roles.add(r.replace("datacenter.", ""));
                            say(s, "  village " + v + ": v" + v + ".ring, " + String.join(", ", roles));
                          }
                          return 1;
                        }))
            // /ecm dns tld remove TLD: the operator at IANA's console (types `ianad revoke TLD` there).
            .then(
                Commands.literal("tld")
                    .then(
                        Commands.literal("remove")
                            .then(
                                Commands.argument("tld", com.mojang.brigadier.arguments.StringArgumentType.word())
                                    .executes(
                                        ctx -> {
                                          var s = ctx.getSource();
                                          var l = s.getServer().overworld();
                                          var d = data(s);
                                          var dns = d.dns(l);
                                          String tld = com.mojang.brigadier.arguments.StringArgumentType.getString(ctx, "tld");
                                          if (!tld.matches("[A-Za-z0-9-]{1,63}")) {
                                            s.sendFailure(Component.literal("not a TLD: " + tld));
                                            return 0;
                                          }
                                          var host = ComputerHost.get(s.getServer(), d.identity(l, dns.iana(), DnsPlan.IANA));
                                          if (host.instance() == null) {
                                            s.sendFailure(Component.literal("IANA's computer (village " + dns.iana() + ") is not running"));
                                            return 0;
                                          }
                                          host.instance().sendInput("ianad revoke " + tld.toLowerCase() + "\n");
                                          say(s, "typed 'ianad revoke " + tld.toLowerCase() + "' at IANA's console (village " + dns.iana()
                                              + ", " + dns.ianaAddress() + "); the root servers drop ." + tld.toLowerCase() + " on the NOTIFY");
                                          return 1;
                                        })))));
    dispatcher.register(root);
  }

  private static WorldNetwork data(CommandSourceStack s) {
    var d = WorldNetwork.get(s.getServer().overworld());
    d.plan(s.getServer().overworld());
    return d;
  }

  private static void say(CommandSourceStack s, String text) {
    s.sendSuccess(() -> Component.literal(text), false);
  }
}
//?}
