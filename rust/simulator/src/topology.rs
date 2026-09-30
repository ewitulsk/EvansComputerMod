//! Topology / scenario file format (TOML).
//!
//! ```toml
//! [sim]                       # optional
//! seed = 42
//! clock = "virtual"           # or "real"
//! timeout = "10s"             # default `expect ... within`
//!
//! [[node]]
//! name = "h1"
//! ifaces = 1                  # default 5, like a terminal's usable faces
//! boot = ["ifconfig eth0 10.0.0.1/24"]   # typed after boot, one per prompt
//!
//! [[link]]                    # a cable: 2-member segment
//! a = "h1:eth0"
//! b = "sw1:eth0"
//! name = "h1-sw1"             # optional
//! drop = 5.0                  # % of frames dropped
//! delay = "2ms"               # propagation delay
//! duplicate = 1.0             # % delivered twice
//! reorder = 2.0               # % held back by reorder_delay (default 5ms)
//! pcap = "h1-sw1.pcap"        # capture from start (relative to --out-dir)
//!
//! [[segment]]                 # hub mesh: every member hears every frame
//! name = "hub"
//! members = ["a:eth0", "b:eth0", "c:eth0"]
//!
//! [scenario]                  # only for --scenario
//! name = "..."
//! steps = '''
//! send h1 "ping -c 1 10.0.0.2"
//! expect h1 /bytes from/ within 5s
//! '''
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::net::Faults;
use crate::util::parse_duration_ms;

pub const DEFAULT_IFACES: usize = 5;

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Dur {
    Ms(i64),
    Text(String),
}

impl Dur {
    pub fn ms(&self) -> Result<i64, String> {
        match self {
            Dur::Ms(v) if *v >= 0 => Ok(*v),
            Dur::Ms(v) => Err(format!("negative duration {}", v)),
            Dur::Text(s) => parse_duration_ms(s).ok_or_else(|| format!("bad duration '{}'", s)),
        }
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct SimSection {
    pub seed: Option<u64>,
    pub clock: Option<String>,
    pub timeout: Option<Dur>,
    pub kernel: Option<String>,
    pub programs: Option<String>,
    pub watchdog: Option<Dur>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    pub name: String,
    pub ifaces: Option<usize>,
    #[serde(default)]
    pub boot: Vec<String>,
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct FaultSpec {
    pub drop: Option<f64>,
    pub delay: Option<Dur>,
    pub duplicate: Option<f64>,
    pub reorder: Option<f64>,
    pub reorder_delay: Option<Dur>,
}

impl FaultSpec {
    pub fn to_faults(&self) -> Result<Faults, String> {
        Ok(Faults {
            drop_pct: self.drop.unwrap_or(0.0),
            delay_ms: self
                .delay
                .as_ref()
                .map(|d| d.ms())
                .transpose()?
                .unwrap_or(0),
            dup_pct: self.duplicate.unwrap_or(0.0),
            reorder_pct: self.reorder.unwrap_or(0.0),
            reorder_ms: self
                .reorder_delay
                .as_ref()
                .map(|d| d.ms())
                .transpose()?
                .unwrap_or(5),
        })
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LinkSpec {
    pub a: String,
    pub b: String,
    pub name: Option<String>,
    pub pcap: Option<String>,
    pub drop: Option<f64>,
    pub delay: Option<Dur>,
    pub duplicate: Option<f64>,
    pub reorder: Option<f64>,
    pub reorder_delay: Option<Dur>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct SegmentSpec {
    pub name: Option<String>,
    pub members: Vec<String>,
    pub pcap: Option<String>,
    pub drop: Option<f64>,
    pub delay: Option<Dur>,
    pub duplicate: Option<f64>,
    pub reorder: Option<f64>,
    pub reorder_delay: Option<Dur>,
}

macro_rules! fault_spec {
    ($t:ty) => {
        impl $t {
            pub fn faults(&self) -> FaultSpec {
                FaultSpec {
                    drop: self.drop,
                    delay: self.delay.clone(),
                    duplicate: self.duplicate,
                    reorder: self.reorder,
                    reorder_delay: self.reorder_delay.clone(),
                }
            }
        }
    };
}
fault_spec!(LinkSpec);
fault_spec!(SegmentSpec);

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSection {
    pub name: Option<String>,
    pub description: Option<String>,
    pub steps: String,
    /// Wall-clock budget for the whole scenario.
    pub wall_timeout: Option<Dur>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TopoFile {
    /// Include another topology file (nodes/links/segments are appended).
    pub topology: Option<String>,
    #[serde(default)]
    pub sim: SimSection,
    #[serde(default)]
    pub node: Vec<NodeSpec>,
    #[serde(default)]
    pub link: Vec<LinkSpec>,
    #[serde(default)]
    pub segment: Vec<SegmentSpec>,
    pub scenario: Option<ScenarioSection>,
    #[serde(skip)]
    pub dir: PathBuf,
}

impl TopoFile {
    pub fn load(path: &Path) -> Result<TopoFile, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let mut t: TopoFile =
            toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
        t.dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        if let Some(inc) = t.topology.take() {
            let inc_path = t.dir.join(inc);
            let base = TopoFile::load(&inc_path)?;
            let mut nodes = base.node;
            nodes.extend(std::mem::take(&mut t.node));
            t.node = nodes;
            let mut links = base.link;
            links.extend(std::mem::take(&mut t.link));
            t.link = links;
            let mut segs = base.segment;
            segs.extend(std::mem::take(&mut t.segment));
            t.segment = segs;
        }
        t.validate()?;
        Ok(t)
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for n in &self.node {
            if n.name.is_empty() || n.name.contains(':') || n.name.contains(char::is_whitespace) {
                return Err(format!("bad node name '{}'", n.name));
            }
            if !seen.insert(n.name.clone()) {
                return Err(format!("duplicate node '{}'", n.name));
            }
            if let Some(i) = n.ifaces {
                if i > 16 {
                    return Err(format!("node {}: at most 16 interfaces", n.name));
                }
            }
        }
        Ok(())
    }

    /// A topology of `n` unconnected computers pc1..pcN.
    pub fn unconnected(n: usize) -> TopoFile {
        TopoFile {
            node: (1..=n.max(1))
                .map(|i| NodeSpec {
                    name: format!("pc{}", i),
                    ifaces: None,
                    boot: vec![],
                    files: Default::default(),
                })
                .collect(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_example() {
        let t: TopoFile = toml::from_str(
            r#"
            [sim]
            seed = 3
            clock = "virtual"
            timeout = "5s"
            [[node]]
            name = "h1"
            ifaces = 1
            boot = ["ifconfig eth0 10.0.0.1/24"]
            [[node]]
            name = "sw1"
            [[link]]
            a = "h1:eth0"
            b = "sw1:eth0"
            drop = 5.0
            delay = "2ms"
            [[segment]]
            members = ["sw1:eth1", "sw1:eth2"]
            delay = 3
            [scenario]
            steps = '''
            wait 1s
            '''
            "#,
        )
        .unwrap();
        t.validate().unwrap();
        assert_eq!(t.node.len(), 2);
        let f = t.link[0].faults().to_faults().unwrap();
        assert_eq!((f.drop_pct, f.delay_ms), (5.0, 2));
        assert_eq!(t.segment[0].faults().to_faults().unwrap().delay_ms, 3);
        assert!(t.scenario.unwrap().steps.contains("wait"));
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<TopoFile>("[[node]]\nname='a'\nifacez=3\n").is_err());
    }
}
