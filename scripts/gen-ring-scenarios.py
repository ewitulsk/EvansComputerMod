"""Generate the Tech Village ring simulator scenarios from the village address plan.

17_bgp_ring10.toml: the ten ISP routers exactly as WorldNetwork provisions them
(eth0 village cable, eth1 data center, eth2/eth3 the ring /28s, open peering for taps).
21_bgp_ring_tap.toml: the same ring with a player router tapped into the 3-4 fiber
segment, peering dynamically with both villages.

Keep isp_config() in step with WorldNetwork.ispRouterConfig (Java).
Run: py -3 scripts/gen-ring-scenarios.py
"""
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "rust" / "simulator" / "scenarios"
COUNT = 10


def prev_of(n):
    return COUNT if n == 1 else n - 1


def next_of(n):
    return 1 if n == COUNT else n + 1


def isp_config(n):
    """Router config of village n's ISP (without DHCP/NAT, which the simulator ring omits)."""
    p, x = prev_of(n), next_of(n)
    o = 64 + n
    return f"""configure terminal
ip routing
interface eth0
ip address 100.{o}.1.1/24
exit
interface eth1
ip address 100.{o}.0.1/24
exit
interface eth2
ip address 172.31.{p}.2/28
exit
interface eth3
ip address 172.31.{n}.1/28
exit
ip prefix-list TAP-IN seq 10 deny 100.64.0.0/10 le 32
ip prefix-list TAP-IN seq 20 deny 172.31.0.0/16 le 32
ip prefix-list TAP-IN seq 30 deny 0.0.0.0/0
ip prefix-list TAP-IN seq 40 permit 0.0.0.0/0 le 24
route-map TAP-IN permit 10
match ip address prefix-list TAP-IN
exit
router bgp {65000 + n}
bgp router-id 100.{o}.0.1
timers bgp 10 30
neighbor 172.31.{p}.1 remote-as {65000 + p}
neighbor 172.31.{n}.2 remote-as {65000 + x}
neighbor TAPS peer-group
neighbor TAPS remote-as external
neighbor TAPS listen ip-range 172.31.{p}.0/28 limit 8
neighbor TAPS listen ip-range 172.31.{n}.0/28 limit 8
address-family ipv4 unicast
neighbor 172.31.{p}.1 activate
neighbor 172.31.{n}.2 activate
neighbor TAPS activate
neighbor TAPS route-map TAP-IN in
neighbor TAPS maximum-prefix 20
network 100.{o}.0.0/24
network 100.{o}.1.0/24
end"""


def router_node(name, config, ifaces=5):
    return f'[[node]]\nname = "{name}"\nifaces = {ifaces}\n[node.files]\n"services.cfg" = "router on\\n"\n"router.cfg" = \'\'\'\n{config}\n\'\'\'\n'


def host_node(name, network, ifaces=1):
    return f'[[node]]\nname = "{name}"\nifaces = {ifaces}\n[node.files]\n"network.cfg" = \'\'\'\n{network}\n\'\'\'\n'


def link(a, b):
    return f'[[link]]\na = "{a}"\nb = "{b}"\n'


def ring(tap_segment):
    s = "".join(router_node(f"r{n}", isp_config(n)) for n in range(1, COUNT + 1))
    s += host_node("server", "iface eth1 100.67.0.10/24\nroute default via 100.67.0.1 dev eth1", 2)
    s += host_node("client", "iface eth0 100.65.1.10/24\nroute default via 100.65.1.1 dev eth0")
    s += link("r1:eth0", "client:eth0")
    for n in range(1, COUNT + 1):
        if tap_segment and n == 3:
            continue
        s += link(f"r{n}:eth3", f"r{next_of(n)}:eth2")
    s += link("r3:eth1", "server:eth1")
    return s


RING10_STEPS = """wait 15s
send r1 "router"
send r1 "show bgp ipv4 unicast summary"
expect r1 /Established/
send r1 "exit"
send client "ping 100.67.0.10 -n 2"
expect client /2 packets sent, 2 received/ within 10s
wait_prompt client
link down r1:eth3
wait 10s
send client "ping 100.67.0.10 -n 2"
expect client /2 packets sent, 2 received/ within 10s
wait_prompt client
link down r1:eth2
wait 5s
send client "ping 100.67.0.10 -n 1"
expect client /1 packets sent, 0 received/ within 10s
wait_prompt client
"""

TAP_CONFIG = """configure terminal
ip routing
interface eth0
ip address 10.200.0.1/24
exit
interface eth1
ip address 172.31.3.5/28
exit
ip route 100.71.0.0/24 10.200.0.99
router bgp 65200
bgp router-id 10.200.0.1
timers bgp 3 9
neighbor 172.31.3.1 remote-as 65003
neighbor 172.31.3.2 remote-as 65004
address-family ipv4 unicast
neighbor 172.31.3.1 activate
neighbor 172.31.3.2 activate
neighbor 172.31.3.1 default-originate
neighbor 172.31.3.2 default-originate
network 10.200.0.0/24
network 100.71.0.0/24
end"""

TAP_STEPS = r"""wait 20s
log "tap peers dynamically with both villages on the 3-4 fiber segment"
send tap "router"
send tap "show bgp ipv4 unicast summary"
expect tap /^ 172\.31\.3\.1 +65003 .*Established/
expect tap /^ 172\.31\.3\.2 +65004 .*Established/
send tap "show bgp ipv4 unicast"
expect tap /^100\.65\.0\.0\/24 via 172\.31\.3\.1/
expect tap /^100\.68\.1\.0\/24 via 172\.31\.3\.2/
expect tap /^100\.74\.1\.0\/24 via /
send tap "exit"
wait_prompt tap
send r3 "router"
send r3 "show bgp ipv4 unicast summary"
expect r3 /^\*172\.31\.3\.5 +65200 .*Established +Up +1$/
expect r3 /^ 172\.31\.3\.2 +65004 .*Established/
log "the tap's /24 is accepted; its hijack of 100.71.0.0/24 and its default are filtered"
send r3 "show bgp ipv4 unicast"
expect r3 /^10\.200\.0\.0\/24 via 172\.31\.3\.5 AS_PATH \[65200\]/
expect r3 /^100\.71\.0\.0\/24 via 172\.31\.3\.2 AS_PATH \[65004, 65005, 65006, 65007\]/
expect_not r3 /^0\.0\.0\.0\/0|^100\.71\.0\.0\/24 via 172\.31\.3\.5/ for 1s
send r3 "show bgp ipv4 unicast neighbors 172.31.3.5"
expect r3 /dynamic, peer-group TAPS, listen range 172\.31\.3\.0\/28/
expect r3 /Route Map In +: TAP-IN/
send r3 "exit"
wait_prompt r3
send r8 "router"
send r8 "show ip route"
expect r8 /^10\.200\.0\.0\/24 via 172\.31\.7\.1 dev eth2 Bgp/
send r8 "exit"
wait_prompt r8
send tapc "ping 100.67.0.10 -n 2"
expect tapc /2 packets sent, 2 received/ within 10s
wait_prompt tapc
send client "ping 10.200.0.10 -n 2"
expect client /2 packets sent, 2 received/ within 10s
wait_prompt client
log "cut the fiber between village 3 and the tap: village 4's side keeps working"
link down r3:eth3
wait 35s
send tapc "ping 100.68.0.10 -n 2"
expect tapc /2 packets sent, 2 received/ within 10s
wait_prompt tapc
send tapc "ping 100.67.0.10 -n 2"
expect tapc /2 packets sent, 2 received/ within 10s
wait_prompt tapc
send tap "router"
send tap "show bgp ipv4 unicast summary"
expect tap /^ 172\.31\.3\.2 +65004 .*Established/
expect tap /^ 172\.31\.3\.1 +65003 .*(Active|Connect|Idle)/
send tap "exit"
wait_prompt tap
send tapc "traceroute 100.67.0.10"
expect tapc /^2  172\.31\.3\.2$/ within 20s
expect tapc /^\d+  100\.67\.0\.10$/ within 30s
wait_prompt tapc within 20s
dump tapc
log "repair: the tap re-peers with village 3"
link up r3:eth3
wait 20s
send r3 "router"
send r3 "show bgp ipv4 unicast summary"
expect r3 /^\*172\.31\.3\.5 +65200 .*Established/
send r3 "exit"
wait_prompt r3
"""


def main():
    s = ring(False)
    s += '[scenario]\nname = "bgp-ring10"\n'
    s += "# Mirrors the Tech Village ISP routers (WorldNetwork.ispRouterConfig): eth0 village\n"
    s += "# access LAN, eth1 server LAN, eth2/eth3 the ring /28s with open peering for taps.\n"
    s += "# A fiber cut with no tap on it drops carrier, so the session falls over at once.\n"
    s += "steps = '''\n" + RING10_STEPS + "'''\n"
    (OUT / "17_bgp_ring10.toml").write_text(s, newline="\n")

    s = ring(True)
    s += router_node("tap", TAP_CONFIG, 2)
    s += host_node("tapc", "iface eth0 10.200.0.10/24\nroute default via 10.200.0.1 dev eth0")
    s += host_node("server4", "iface eth1 100.68.0.10/24\nroute default via 100.68.0.1 dev eth1", 2)
    s += link("tap:eth0", "tapc:eth0")
    s += link("r4:eth1", "server4:eth1")
    s += '[[segment]]\nname = "fiber34"\nmembers = ["r3:eth3", "r4:eth2", "tap:eth1"]\n'
    s += '[scenario]\nname = "bgp-ring-tap"\n'
    s += "# A player router (AS 65200, LAN 10.200.0.0/24) tapped into the 3-4 fiber at 172.31.3.5/28.\n"
    s += "# Both villages accept it as a dynamic neighbor; their TAP-IN filter keeps its /24 and\n"
    s += "# drops its hijack of village 7's 100.71.0.0/24 and its default route.\n"
    s += "steps = '''\n" + TAP_STEPS + "'''\n"
    (OUT / "21_bgp_ring_tap.toml").write_text(s, newline="\n")


if __name__ == "__main__":
    main()
