# DNS

Names resolve on the Tech Village ring the way they do on the internet. A house PC asks its
village's **recursive resolver**. The resolver walks the tree from the **root** servers to the
**TLD** servers to the zone's own **authoritative** server, caches what it learns, and
answers. `dig +trace` shows the same chain of referrals it would show at home.

```
   IANA (village B): root zone primary ──NOTIFY/AXFR──> root servers a-e (B+1, B+3, B+5, B+7, B+9)
                                  │
            ┌─────────────────────┴─────────────────────┐
     ring. registry (B+4)                         ecm. registry (B+2)
     + secondary (B+9)                            + secondary (B+7)
            │                                             │   ▲ EPP-lite (TCP 700)
     vN.ring on every village's web server        mybase.ecm  │
                                                  (hosted, or │ registrar (B+6):
                                                  your named) │ accounts, web, whois
```

Like the rest of the mod, DNS is software on computers: the `rust/crates/ecm-dns` crate and
the programs `named`, `dig`, `nslookup`, `whois`, `domain`, `registryd`, `registrard` and
`ianad`. They speak RFC 1035 DNS on UDP and TCP port 53. There is no DNS block.

## Using it

On a village computer (a house PC behind its home router, a computer in a Data Center
rack), DHCP hands out the village resolver and the search domain:

```text
/ > cat /etc/resolv.conf
# Written by the kernel from DHCP / resolvectl. Edit it and it is yours.
nameserver 100.65.0.53
search v1.ring
/ > curl http://web/                    # web.v1.ring, through the search domain
/ > curl http://web.v4.ring/            # another village's site
/ > dig +trace web.v4.ring              # root -> ring. -> v4.ring.
/ > nslookup -type=MX mybase.ecm
/ > whois mybase.ecm
```

| Name | What |
|---|---|
| `web.vN.ring`, `www.vN.ring` | village N's web server (`100.(64+N).0.10`) |
| `isp.vN.ring`, `dns.vN.ring`, `ns.vN.ring` | its ISP router, its resolver, its zone's server |
| `chat.ring`, `repo.ring`, `core.ring` | the ring's chat server, package repository, carrier core |
| `nic.ring`, `whois.nic.ring` | the registrar: web pages and API, whois |
| `iana.ring` | IANA: TLD applications |
| `a.root-servers.ring` ... `e.root-servers.ring` | the root servers |
| `NAME.ecm` | names players register |
| `example.com` and every other real name | forwarded to the real internet, with the internet add-on (see "Real names") |

### The stub resolver

Programs resolve names with `getaddrinfo` (`ecm_host_abi::socket::resolve`). The kernel
(`ecm-net` `resolv.rs`, `Stack::dns_query_with`, `net/ipc.rs`) works like a Linux libc:

* `/etc/hosts` first (`address name [aliases]`); `localhost` is always there.
* `/etc/resolv.conf`: up to three `nameserver` lines tried in order, `search` domains,
  `options ndots:N`. Without the file, the server from `network.cfg` (`dns` line) and the
  DHCP domain are used.
* A name with fewer dots than `ndots` (default 1) is tried with each search domain first,
  then as it is. A name ending in a dot is only tried as it is.
* **NXDOMAIN** for every candidate fails at once ("name not found"). A server that does not
  answer is retried, then the next server is tried, and the error is "no answer from
  100.x.0.53". A server that answers SERVFAIL or REFUSED is a third error. `curl` and `ping`
  print which one happened.
* CNAMEs in an answer are followed (a CNAME without its target's address is asked again).
  Answers are cached by their TTL (at most 5 minutes, 32 names).
* The stub asks with recursion desired. It never iterates itself.

The kernel writes `/etc/resolv.conf` when DHCP or `resolvectl dns` changes the server or the
domain. A file without the kernel's first line is the player's and is left alone. DHCP
carries the domain as option 15: `domain-name vN.ring` in a router pool, `domain NAME` in a
`dhcpd` pool.

## named

`named &` (or a `named &` line in `services.cfg`) is the DNS server, like BIND. It reads
`/etc/named.conf`:

```text
recursion yes                      # answer RD queries for clients in `allow`
allow 100.64.0.0/10 10.0.0.0/8     # who may use recursion (default: 127/8 and the private ranges)
root-hints /etc/named/root.hints   # where iteration starts
fallback 1.1.1.1                   # where names the in-game root does not know go
zone v3.ring primary /etc/named/v3.ring.zone
zone ecm primary /etc/named/ecm.zone notify 100.71.0.55
zone ring secondary 100.68.0.55            # kept in /etc/named/sec/ring.zone
zone . secondary 100.66.0.57 file /etc/named/sec/root.zone
allow-transfer 100.64.0.0/10       # who may AXFR (default: anyone)
listen 100.67.0.53                 # answer on one address only (default: all)
```

* **Authoritative**: data, CNAMEs inside the zone, referrals (NS records plus glue) at
  delegations, NXDOMAIN and NODATA with the SOA for negative caching. The most specific zone
  a server has wins.
* **Recursive** (`recursion yes`, for clients in `allow`): the iterative resolver
  (`ecm_dns::resolver`). It starts at the root hints and follows at most 16 referrals and 8
  CNAMEs. It looks up the addresses of nameservers that came without glue, moves to the next
  server after a 1 s timeout or a bad answer, and tries timed-out servers once more. It
  caches positive and negative answers by TTL. A delegation where nobody answers usefully is
  SERVFAIL ("lame delegation", logged).
* **Primary zones** are read from master files. On (re)load with a new serial, named sends a
  NOTIFY to the zone's NS addresses and to the `notify` list.
* **Secondary zones** check the primary's SOA serial at start, on a NOTIFY and every SOA
  refresh. A newer serial is fetched by AXFR over TCP 53 and saved.

Control commands talk to the running daemon on `127.0.0.1` port 953:

| Command | |
|---|---|
| `named reload` | re-read named.conf and the zones (NOTIFYs for changed serials) |
| `named status` | zones and serials, recursion, counters |
| `named cache` | what the resolver has cached, TTLs counted down |
| `named flush [NAME]` | forget cached answers |
| `named retransfer ZONE` | a secondary checks its primary now |
| `named check [FILE]` | check named.conf and every primary zone file |
| `named stop` | stop the daemon |

### Zone files

RFC 1035 master format, written with `edit`:

```text
$ORIGIN lab.test.
$TTL 300
@     IN SOA ns1 admin ( 1 60 30 600 30 )   ; serial refresh retry expire minimum
      IN NS  ns1
ns1      A   10.53.0.3
www  60  A   10.53.0.3
web      CNAME www
mail     MX  10 www
info     TXT "hello world"
```

`$ORIGIN`, `$TTL`, `@`, relative and absolute names, a blank owner (same as the line before),
TTL and class in any order, parentheses over several lines, `;` comments, TTL units (`1h`,
`2d`) are all supported. Record types: A, NS, CNAME, SOA, PTR, MX, TXT (others are passed
through as raw data in answers). A zone needs one SOA and an NS at its apex.

### dig and nslookup

```text
dig [@SERVER] NAME [TYPE] [+trace] [+norecurse] [+short] [+tcp] [-p PORT]
dig @SERVER ZONE AXFR          # the whole zone
nslookup [-type=TYPE] NAME [SERVER]
```

Without `@SERVER`, `dig` asks the first `nameserver` of `/etc/resolv.conf`. `+trace` asks
that resolver for the root servers, then walks the tree itself and prints each step and who
gave it:

```text
.                        86400 IN NS    a.root-servers.ring.
;; Received 109 bytes from 100.65.0.53#53(100.65.0.53) in 2 ms
ring.                    86400 IN NS    ns1.ring.ring.
;; Received 102 bytes from 100.71.0.54#53(a.root-servers.ring) in 10 ms
v2.ring.                  3600 IN NS    ns.v2.ring.
;; Received 62 bytes from 100.74.0.55#53(ns1.ring.ring) in 2 ms
web.v2.ring.               300 IN A     100.66.0.10
;; Received 45 bytes from 100.66.0.10#53(ns.v2.ring) in 2 ms
```

## Who runs what

Roles are dealt out from one base village B, chosen by the seed (`DnsPlan.baseVillage`, its
own salt like the chat village's), at fixed offsets. Every village takes part and no single
village is the whole of DNS. `/ecm dns roles` prints the map. `/ecm techvillage info N`
prints village N's part, and each village's web page lists it too.

| Role | Villages | Address | Runs |
|---|---|---|---|
| Recursive resolver | all ten | `100.(64+N).0.53` | `named` with recursion, root hints, fallback |
| Village zone `vN.ring` | all ten | `100.(64+N).0.10` | `named` beside `httpd` on the web server |
| IANA: root zone primary, TLD applications | B | `.57` | hidden primary for `.`, `ianad` |
| Root servers a-e | B+1, B+3, B+5, B+7, B+9 | `.54` | `named`, secondaries of IANA |
| `.ecm` registry | B+2 (primary), B+7 | `.55` | `named` for `ecm.`; `registryd` on the primary |
| `.ring` registry | B+4 (primary), B+9 | `.55` | `named` for `ring.` |
| Registrar | B+6 | `.56` | `registrard`: http://nic.ring/, whois |

That makes 21 new computers, at most three per Data Center (racks 5-7). The root servers sit
in alternate villages. Any two fiber cuts leave every piece of the ring with a root server,
and each registry's primary and secondary are five villages apart, so cutting the ring in
two halves leaves each half with one copy of `ring.`.

**`dns.resolverOnIspRouter`** (server config `evanscomputermod-dns-server.toml`, default
`false`): when true, each village's ISP router runs the recursive `named` itself on
`100.(64+N).0.1`. No `.53` computers are generated or booted, DHCP pools and the carrier
core hand out `.1`, and the count drops to 11 new computers. The provisioning marker records
the mode, so switching it rewrites the infrastructure configuration once at the next start.

## Real names

The resolvers treat the in-game root as the truth. When the root says a top-level domain does
not exist, the query goes to the `fallback` (real `1.1.1.1` through village 1's uplink, when
the host gateway is up). So `.ring` and `.ecm` resolve in game and `example.com` still
resolves for real. The uplink needs the optional server-side internet add-on
(`evanscomputermod-internet`, docs/ROUTER.md): without it the Internet Gateway is offline and
the fallback times out (SERVFAIL), while ring and `.ecm` names keep working. Names under the
special-use TLDs (`test`, `example`, `invalid`, `localhost`, `local`, `internal`, `arpa`)
and the ring's own TLDs are never forwarded.

## Registering a name

The registrar (B+6) keeps the customers: accounts, web pages, whois. The `.ecm` registry
(B+2) keeps the zone. They talk **EPP-lite**: one command per line on TCP 700, authenticated
with the registrar's credentials (`ecm_dns::registry`):

```text
LOGIN <registrar> <secret>            CHECK <domain>          CREATE <domain> <registrant>
INFO <domain>                         DELETE <domain>         LIST
UPDATE <domain> RR ADD <name> <ttl> <type> <data>       UPDATE <domain> RR DEL <name> <type> [data]
UPDATE <domain> NS SET <ns>[=<glue>]...                 UPDATE <domain> NS HOSTED
```

Every change rewrites `ecm.zone` with a new serial and has the registry's `named` reload, which
NOTIFYs the secondary in B+7. The registry accepts any registrar it has credentials for, so a
second registrar is a `registrar <id> <secret>` line in `/etc/registryd.conf`.

From any computer:

```text
domain account create alice pw1234                  # an account at nic.ring (saved in /etc/domain.auth)
domain register mybase.ecm
domain set mybase.ecm www A 100.70.0.120            # [TTL], at least 60
domain set mybase.ecm @ MX 10 mail.mybase.ecm.
domain ns mybase.ecm ns1.mybase.ecm=10.200.3.10     # your own named; glue for names inside the domain
domain ns mybase.ecm --hosted                       # back to records at the registry
domain show mybase.ecm | domain list | domain delete mybase.ecm
whois mybase.ecm
```

The same is on the web at http://nic.ring/ (made for the in-game Browser, docs/gui/BROWSER.md:
search a name, create an account or log in, which keeps a session cookie (`nic_session`, in
memory: a restart of `registrard` logs everyone out), then register names and add records on
the home page, which lists the account's domains; `ecm_dns::registrar_web`) and in `POST /api` (the plain-text API
`domain` uses: `op=account|login|check|register|set|del|ns|show|list|delete`). A domain is
either **hosted** (its records live in the `.ecm` zone itself, served by `ns1.ecm.ring` and
`ns2.ecm.ring`) or **delegated** to the registrant's nameservers.

Rules: labels are 3-63 letters, digits and hyphens, not at the start or end. Reserved names
are refused (`nic`, `www`, `root`, `whois`, `iana`, `registry`, `registrar`, `dns`, `ns1`,
`ns2`, `example`, `test`, `localhost`, `v1`-`v10`). An account can hold five domains. TTLs
must be at least 60 s. The account belongs to whoever knows the password. Registration is
free and does not expire.

## Adding a TLD

A new top-level domain exists when IANA puts its NS records in the root zone. Run the TLD's
nameservers yourself (a `named` with the TLD's zone, two in different networks is better),
then apply:

```text
domain tld apply base ns1.base=10.200.4.1 ns2.base=10.200.4.10
domain tld show base | domain tld list | domain tld revoke base
```

`ianad` checks the label: 2-63 LDH, not all digits, not reserved, not taken. It then asks every
nameserver for the TLD's SOA, and each must answer authoritatively (the technical
evaluation). If all pass, it adds `base. NS` with glue to the root zone, bumps the serial and
has `named` reload, which NOTIFYs the five root servers. Resolvers see the TLD once any cached
NXDOMAIN for it expires (the root's negative TTL is 5 minutes). There is one TLD per account.
An operator removes one with `/ecm dns tld remove base`, which types `ianad revoke base` at
the IANA console.

The other two ways real life has work too:

* **Alternate root**: run `named` with your own `.` zone and point your resolver's
  `root-hints` at it. Only your resolver's clients see it.
* **Local zone**: a resolver answers a private TLD itself (`zone home primary
  /etc/named/home.zone` on a home router or your own resolver), without asking the root.

## Provisioning

`PROVISION_VERSION` 6 (`WorldNetwork`) adds the computers above, with their `named.conf`,
zones (`root.zone` from the plan, `ring.zone` with every village's delegation and the
ring-wide names, an empty `ecm.zone`), root hints, and `named` on every web server with its
`vN.ring` zone. The ISP's DHCP pools hand out the village resolver and `vN.ring`. The
carrier core's phones get the resolver of the core's village. Infrastructure in existing
worlds is upgraded once. Data Center servers get `/etc/resolv.conf` and a `dns` line.

Home routers in **old worlds** keep handing out `1.1.1.1` (house computers are never
rewritten). Real names still work there. For ring names, change the home router's pool to the
village resolver:

```text
router
configure terminal
dhcp-server vrf default
pool lan
dns-server 100.(64+N).0.53
domain-name vN.ring
end
write memory
```

New worlds' home routers hand out the village resolver already.

## Scenarios and tests

| Scenario | What the player does | Controls |
|---|---|---|
| `dns_lab` | Four computers around a switch: root, the `test.` TLD, `lab.test.` (zone written in `edit`) and a client with its own resolver. `dig +trace www.lab.test`, the search domain, a CNAME | the zone's server stopped: SERVFAIL; the delegation removed from the TLD: NXDOMAIN |
| `dns_tech_village` | At a house PC: `/etc/resolv.conf` from DHCP, `dig +trace web.vN.ring` (root, `ring.`, `vN.ring.`), `curl http://web.vN.ring/`, `curl http://web/` | `dig nosuch.ring` NXDOMAIN at once; the second answer comes from the cache with a lower TTL |
| `dns_resilience` | `/ecm net cut` around a root server's village: names still resolve through the other letters; the ring cut in two: each half resolves its `.ring` names | a name in the other half: SERVFAIL |
| `dns_register` | A computer in a free Data Center rack makes an account, registers `mybase.ecm`, points `www` at itself; the house PC curls it, `whois` shows the registrant, the `.ecm` secondary has it (NOTIFY), with the primary cut off the secondary still answers | the same name again, a wrong password, a reserved name; a change while the registry is unreachable |
| `dns_own_nameserver` | The fiber-tap lab's PC runs `named` for `tapbase.ecm`, registers a delegation with glue; `dig +trace` shows the fourth hop; a changed record is served from the resolver's cache until its TTL runs out | the player's `named` stopped: SERVFAIL, everything else keeps resolving |
| `browser_dns` (docs/gui/BROWSER.md) | At a house PC, the Browser: `web.vN.ring`, `web` (search domain); at http://nic.ring/ a search, an account, a registration and a `www` record, then `http://www.NAME.ecm/` | `nosuch.ring`: Can't find the server; the same name twice |
| `dns_tld` | The fiber-tap lab runs `.base` on the tap router and the PC (which also serves `shop.base`), applies to IANA; `dig +trace www.shop.base` shows root, `base.`, `shop.base.`; another village's resolver finds it | `.test` refused; a lame nameserver refused; before the application the root says NXDOMAIN |

Spawn them with `/ecm scenario spawn <name>`. The ring scenarios start at the house PC
nearest to you. The tap scenarios start on the fiber span nearest to you, which must be
within 64 blocks. GameTests: `scripts\Test.ps1 -GameTests ecm_dns` (`testing/v1211/DnsTests`;
the ring ones place Tech Village 1 at its site and boot the ring). Simulator:
`45_dns_lab`, `46_dns_registry`. Rust: `cargo test -p ecm-dns` (unit tests, a multi-server
`hierarchy` test, `fuzz`), `-p ecm-net` (stub resolver), JUnit `DnsPlanTest`.

## Later

Reverse DNS for `100.64.0.0/10` (so `traceroute` prints names), SRV records (so `disco` can
also look up `_chat._tcp.ring`), `Host:`-based virtual hosting in `httpd`, an anycast resolver
address, DNSSEC with the keys `pkg` already uses, payment in emeralds and expiry.
