# Service discovery

Computers find chat servers and app repositories on the network by asking for them, the
way Bonjour/mDNS finds printers: nobody types an address. The **App Store** lists the apps of
every repository it finds, **Chat** lists the chat servers it finds, and `disco` shows
everything from a terminal.

Like the rest of the mod, it is software on computers (`rust/crates/ecm-disco`, the `disco`
program), built on UDP sockets; there is no discovery block.

## How it works

A computer that offers services runs the responder, `disco serve &` (like avahi-daemon). It
answers for:

* what its servers **announce**: `chatd` and `pkg serve` write a line to `/run/disco/` when
  they start (`chatd` under its room's name, `pkg serve` under `--name NAME`);
* what `/etc/disco.conf` lists:

```text
service <type> <port> [name]           a service on this computer
record <type> <host> <port> [name]     a service elsewhere this responder answers for
relay gateway | relay <address>        pass searches on to the next router
```

A search (`disco`, the App Store, Chat, `chat` with no server, `pkg discover`) sends one UDP
query to port **5354** of:

1. its own computer (`127.0.0.1`);
2. every connected segment (each interface's subnet broadcast, e.g. `10.0.8.255`);
3. its default gateway;

and collects the answers for 1.5 s. One answer per type, address and port is kept; a named
one wins over an unnamed one.

Broadcasts never cross a router, so **routers relay**: a router running `disco serve` with
`relay gateway` answers a search from its LAN with its own entries and passes it on to its
own default gateway, returning what comes back for 1 s. A relayed search goes at most three
routers up, and never back where it came from.

```
house PC ──search──> home router (relay gateway) ──> village ISP router (records)
   <──── chat 100.(64+K).0.20:7777 Tech Village chat, pkg 100.(64+K).0.21:8080 Tech-Village-Apps
phone (LTE) ──> carrier core (relay gateway) ──> its village's ISP router (records)
```

Types used by the mod: `chat` (a `chatd`) and `pkg` (a `pkg serve` repository,
`http://<host>:<port>/`). Any other lower-case name works for your own services
(`service web 80 My site`).

## Wire format

One datagram, text lines:

```text
ECMDISCO/1 QUERY <id> <type|*> <hops>
ECMDISCO/1 ANSWER <id>
<type> <host|-> <port> <name...>        one line per service, at most 16; '-' is the sender
```

A relay rewrites `-` to the address it got the answer from. Anything else is ignored.

## Commands

```text
/ > disco
chat  10.0.8.1:7777          Tech Village chat
pkg   10.0.8.1:8080          Lab-Apps
/ > disco chat                       only one type (--wait MS to wait longer)
/ > disco serve &                    the responder (-v logs every query)
disco: answering on UDP port 5354 for 2 services, no relay. Ctrl+T to stop.
/ > pkg serve /srv/repo 8080 --name Lab-Apps &
/ > pkg discover
Lab-Apps  http://10.0.8.1:8080/  (pkg repo add Lab-Apps http://10.0.8.1:8080/)
/ > chat                             no server given or in /etc/chat.conf
chat: no server given; looking on the network ...
chat: found Tech Village chat at 10.0.8.1:7777
```

`chat` connects when it finds exactly one server and lists them when it finds several.

## In the apps

* **App Store**: opening it and Refresh search the network. A repository it finds that isn't
  configured is used as if it were (its apps are in Discover, installable), shown as "found on
  the network" in the status line, the wide sidebar and the Repositories page, whose **Add**
  keeps it in `/etc/pkg/repos`. Nothing is written until then. A key from the network is never
  pinned, so these apps are "unverified" (or "signed, key not pinned") unless you add the
  repository with `pkg repo add ... --key`. A repository already configured (same URL) is not
  listed twice.
* **Chat**: with no saved server, it searches as it opens (so the network permission is asked
  then) and lists the servers under **On the network**; a tap connects. With exactly one found
  and an empty form, the form is filled in with it (Connect is still yours). **Search again**
  repeats it. A saved server connects at once, without searching.

## In the Tech Villages

Generated in new worlds and in existing worlds' infrastructure (provisioning version 5,
docs/TECH_VILLAGE_NETWORK.md "Finding services"):

| Computer | services.cfg | /etc/disco.conf |
|---|---|---|
| ISP router | `disco serve &` | `record chat ...` and `record pkg ...`: the ring's chat server and repository |
| Home router | `disco serve &` | `relay gateway` (to the ISP) |
| Carrier core | `disco serve &` | `relay gateway` (phones' searches to its village's ISP) |
| Chat server, package server | `disco serve &` | none: `chatd` and `pkg serve --name Tech-Village-Apps` announce themselves |

So a house PC, a computer you cable into a village or a Data Center rack, and a phone on LTE
all find the ring's chat server and repository. Houses in worlds generated before this
version keep their routers' old `services.cfg` (house computers are never rewritten): add
`disco serve &` to the home router's `services.cfg` (its `/etc/disco.conf` with `relay gateway`
is written when it is missing).

## Limits

* No multicast in the network stack, so discovery is subnet broadcast plus relays; a router
  without `disco serve` is the end of the line for searches behind it.
* Announcements are files: a server that was killed stays listed by its computer's responder
  until the file in `/run/disco/` is removed or the server starts again (connecting then fails
  with the usual message).
* Answers are not authenticated, which is why discovered repositories are never pinned.

## Tests

| What | Where | Run |
|---|---|---|
| Wire format, disco.conf, the responder and relay (hop limit, dedupe, '-' rewritten), a search through a home router to the ISP | `rust/crates/ecm-disco/src/tests.rs` | `-Rust ecm-disco` |
| The store lists found repositories without adding them, installs from them, Add keeps one; configured URLs are skipped, names never collide, keys never pinned | `rust/crates/ecm-apps-ui/src/store/tests.rs` | `-Rust ecm-apps-ui` |
| Chat searches as it opens, lists servers, a tap connects; one server fills the form; Search again; a saved server doesn't search | `rust/wasm-programs/chatapp/src/app/tests.rs` | `-Rust chatapp` |
| The ISP's records and the routers' relay file (where they go: the in-game checks below) | `VillageDiscoveryTest` | `-JUnit VillageDiscoveryTest -McVersion 26.1` |
| `service_discovery`: nothing before `disco serve`, then `disco`, `pkg discover`, the App Store and Chat over a real cable | `AppScenarios.serviceDiscovery` | `-GameTests ecm_apps -GameTestOnly service_discovery` |
| A Data Center computer finds the package server with `disco pkg` | `ecm_cellular` `tech_village_repo` | `-GameTests ecm_cellular -GameTestOnly tech_village_repo` |
| A house PC finds the ring's services through its home router; a phone's Chat finds the chat server over LTE through the core | `house_network`, `cell_tech_village` | `-ClientChecks -ClientSuite tech` |
