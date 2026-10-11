# Rebuild the packet-level simulator fixtures from the village address plan.
$ErrorActionPreference='Stop'
$scenarioDir=Join-Path $PSScriptRoot '../rust/simulator/scenarios'
function Router-Node($name,$config) {
    "[[node]]`nname = `"$name`"`nifaces = 5`n[node.files]`n`"services.cfg`" = `"router on\n`"`n`"router.cfg`" = '''`n$config`n'''`n"
}
function Host-Node($name,$network) {
    "[[node]]`nname = `"$name`"`nifaces = 1`n[node.files]`n`"network.cfg`" = '''`n$network`n'''`n"
}
$lan=@'
configure terminal
ip routing
interface eth0
ip address 192.168.1.1/24
ip nat inside
exit
interface eth1
ip dhcp
ip nat outside
exit
dhcp-server vrf default
pool lan
range 192.168.1.10 192.168.1.100
default-router 192.168.1.1
dns-server 10.0.0.1
lease 86400
enable
end
'@
$isp=@'
configure terminal
interface eth0
ip address 10.0.0.1/24
exit
dhcp-server vrf default
pool wan
range 10.0.0.10 10.0.0.20
default-router 10.0.0.1
dns-server 10.0.0.1
lease 86400
enable
end
'@
$s=(Router-Node 'home' $lan)+(Router-Node 'isp' $isp)+(Host-Node 'pc' 'iface eth0 dhcp')+(Host-Node 'server' 'iface eth0 10.0.0.2/24')
$s+=@'
[[link]]
a = "pc:eth0"
b = "home:eth0"
[[segment]]
members = ["home:eth1", "isp:eth0", "server:eth0"]
[scenario]
name = "router-nat-dhcp"
steps = '''
wait 5s
send pc "ifconfig"
expect pc /192\.168\.1\.10/
wait_prompt pc
send pc "ping 10.0.0.2 -n 2"
expect pc /2 packets sent, 2 received/ within 10s
wait_prompt pc
send home "router"
send home "show ip nat translations"
expect home /192\.168\.1\.10/
send home "exit"
wait_prompt home
send home "router off"
send pc "ping 10.0.0.2 -n 1"
expect pc /1 packets sent, 0 received/ within 10s
'''
'@
[IO.File]::WriteAllText((Join-Path $scenarioDir '15_router_nat_dhcp.toml'),$s)
foreach ($count in @(2,10)) {
    $s=''
    for ($i=1;$i -le $count;$i++) {
        $prev=if ($i -eq 1) {$count} else {$i-1}
        $next=if ($i -eq $count) {1} else {$i+1}
        $config="configure terminal`nip routing`ninterface eth0`nip address 172.31.$prev.2/30`nexit`ninterface eth1`nip address 172.31.$i.1/30`nexit`ninterface eth2`nip address 100.$(64+$i).0.1/16`nexit`nrouter bgp $(65000+$i)`nbgp router-id 100.$(64+$i).0.1`ntimers bgp 1 3`nneighbor 172.31.$prev.1 remote-as $(65000+$prev)`nneighbor 172.31.$i.2 remote-as $(65000+$next)`naddress-family ipv4 unicast`nneighbor 172.31.$prev.1 activate`nneighbor 172.31.$i.2 activate`nnetwork 100.$(64+$i).0.0/16`nend"
        $s+=Router-Node "r$i" $config
    }
    $target=if ($count -eq 2) {2} else {3}
    $s+=Host-Node 'server' "iface eth0 100.$(64+$target).0.10/24`nroute default via 100.$(64+$target).0.1 dev eth0"
    $s+=Host-Node 'client' "iface eth0 100.65.0.10/24`nroute default via 100.65.0.1 dev eth0"
    $s+="[[link]]`na = `"r1:eth2`"`nb = `"client:eth0`"`n"
    for ($i=1;$i -le $count;$i++) {
        $next=if ($i -eq $count) {1} else {$i+1}
        $s+="[[link]]`na = `"r${i}:eth1`"`nb = `"r${next}:eth0`"`n"
    }
    $s+="[[link]]`na = `"r${target}:eth2`"`nb = `"server:eth0`"`n[scenario]`nname = `"bgp-ring$count`"`nsteps = '''`nwait 15s`nsend r1 `"router`"`nsend r1 `"show bgp ipv4 unicast summary`"`nexpect r1 /Established/`nsend r1 `"exit`"`nsend r1 `"ping 100.$(64+$target).0.10 -n 2`"`nexpect r1 /2 packets sent, 2 received/ within 10s`nlink down r1:eth1`nwait 10s`nsend r1 `"ping 100.$(64+$target).0.10 -n 2`"`nexpect r1 /2 packets sent, 2 received/ within 10s`nlink down r1:eth0`nwait 5s`nsend r1 `"ping 100.$(64+$target).0.10 -n 1`"`nexpect r1 /1 packets sent, 0 received/ within 10s`n'''`n"
    $filename=if ($count -eq 2) {'16_bgp_pair.toml'} else {'17_bgp_ring10.toml'}
    $s=$s.Replace('send r1 "ping','send client "ping').Replace('expect r1 /2 packets','expect client /2 packets').Replace('expect r1 /1 packets','expect client /1 packets')
    $s=$s.Replace('within 10s',"within 10s`nwait_prompt client")
    [IO.File]::WriteAllText((Join-Path $scenarioDir $filename),$s)
}
