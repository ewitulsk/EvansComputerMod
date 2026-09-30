use crate::{console::Term, fs, net::Net, services};
use ecm_router::{
    cli::{self, Session},
    Config, Router,
};
pub const CONFIG_PATH: &str = "router.cfg";
pub struct RouterService {
    session: Option<Session>,
}
impl RouterService {
    pub fn new() -> Self {
        Self { session: None }
    }
    pub fn prompt(&self) -> String {
        self.session
            .as_ref()
            .map(Session::prompt)
            .unwrap_or_default()
    }
    pub fn start(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) -> bool {
        if net.router.is_some() {
            return true;
        }
        let config = match fs::read_to_string(CONFIG_PATH) {
            Some(text) => match cli::load(&text) {
                Ok(c) => c,
                Err(e) => {
                    con.print(&e);
                    return false;
                }
            },
            None => Config::default(),
        };
        config.apply(&mut net.stack, now);
        let mut router = Router::new(config);
        if let Some(leases) = fs::read_to_string("router.leases") {
            router.dhcp.restore(&leases, now);
        }
        let bgp = router.config.bgp.clone();
        let policy = router.config.policy.clone();
        if bgp.asn != 0 && !bgp.router_id.is_unspecified() {
            net.bgp = Some(crate::bgp_svc::BgpService::new(bgp, policy, now));
        }
        net.router = Some(router);
        con.println("Router started.");
        true
    }
    pub fn start_detached(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) {
        if self.start(net, con, now) {
            services::enable("router", true);
        }
    }
    pub fn stop_cmd(&mut self, net: &mut Net, con: &mut dyn Term) {
        if let Some(mut b) = net.bgp.take() {
            b.stop(&mut net.stack);
        }
        net.router = None;
        net.stack.set_forwarding(false);
        self.session = None;
        services::enable("router", false);
        con.println("Router stopped.");
    }
    pub fn enter_cli(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) -> bool {
        if !self.start(net, con, now) {
            return false;
        }
        self.session = Some(Session::new());
        true
    }
    pub fn leave_cli(&mut self) {
        self.session = None;
    }
    pub fn exec(&mut self, line: &str, net: &mut Net, con: &mut dyn Term, now: i64) -> bool {
        let Some(mut s) = self.session.take() else {
            return true;
        };
        let exit = self.exec_in(&mut s, line, net, con, now);
        if !exit {
            self.session = Some(s);
        }
        exit
    }
    pub fn exec_in(
        &mut self,
        s: &mut Session,
        line: &str,
        net: &mut Net,
        con: &mut dyn Term,
        now: i64,
    ) -> bool {
        if line.trim().starts_with("show bgp ipv4 unicast") {
            if let Some(b) = &net.bgp {
                if line.contains("summary") || line.contains("neighbors") {
                    for (ip, p) in &b.engine.peers {
                        con.println(&format!(
                            "{} AS {} {:?} prefixes {}",
                            ip,
                            p.config.remote_as,
                            p.state,
                            p.adj_in.len()
                        ));
                    }
                } else {
                    for r in b.engine.rib.values() {
                        con.println(&format!(
                            "{}/{} via {} AS_PATH {:?} local-pref {} MED {} community {}",
                            r.prefix.address,
                            r.prefix.len,
                            r.attributes.next_hop,
                            r.attributes.path,
                            r.attributes.local_pref,
                            r.attributes.med,
                            r.attributes
                                .communities
                                .iter()
                                .map(|c| format!("{}:{}", c >> 16, c & 65535))
                                .collect::<Vec<_>>()
                                .join(",")
                        ));
                    }
                }
            } else {
                con.println("BGP is not configured.");
            }
            return false;
        }
        if line.trim() == "show ip route" {
            for r in net.stack.routes() {
                con.println(&format!(
                    "{}/{} via {} dev {} {:?} [{}/{}]",
                    r.dst,
                    r.prefix,
                    r.gateway,
                    net.stack
                        .iface(r.iface)
                        .map(|i| i.name.as_str())
                        .unwrap_or("?"),
                    r.source,
                    r.distance,
                    r.metric
                ));
            }
            return false;
        }
        if line.trim() == "show arp" {
            for i in 0..net.stack.iface_count() {
                for (ip, mac, state) in net.stack.neighbors(i) {
                    con.println(&format!("{} {} {:?}", ip, mac, state));
                }
            }
            return false;
        }
        let Some(router) = net.router.as_mut() else {
            return true;
        };
        if line.trim() == "show ip nat translations" {
            for m in &router.nat.mappings {
                con.println(&format!(
                    "{} {}:{} -> {}:{}",
                    m.protocol, m.inside, m.port, m.outside, m.external
                ));
            }
            return false;
        }
        if line.trim() == "show dhcp-server leases" {
            con.print(&router.dhcp.render());
            return false;
        }
        let (output, exit, save) = s.exec(&mut router.config, line);
        con.print(&output);
        if !output.starts_with('%') {
            router.config.apply(&mut net.stack, now);
        }
        if save {
            fs::write(CONFIG_PATH, router.config.render().as_bytes());
            services::enable("router", true);
        }
        let bgp = router.config.bgp.clone();
        let policy = router.config.policy.clone();
        if bgp.asn != 0
            && !bgp.router_id.is_unspecified()
            && net.bgp.as_ref().is_none_or(|b| b.engine.config != bgp)
        {
            if let Some(mut b) = net.bgp.take() {
                b.stop(&mut net.stack);
            }
            net.bgp = Some(crate::bgp_svc::BgpService::new(bgp, policy.clone(), now));
        }
        if let Some(b) = net.bgp.as_mut() {
            b.engine.policy = policy;
        }
        exit
    }
}
