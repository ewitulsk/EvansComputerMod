//! Block definitions: the palette and what each block generates.
//!
//! One source of truth: `blocks.json` (compiled in) holds the built-in
//! categories, and peripheral blocks are made from the host's peripheral
//! catalog (`periph_methods("@catalog")`, the same `@PeripheralMethod`
//! metadata `help` uses), one category per peripheral type and one block
//! per method, exactly as the old Java `VisualBlockRegistry` built them.

use ecm_ui::Json;

/// The built-in block definitions.
pub const BLOCKS_JSON: &str = include_str!("../blocks.json");

/// A block input or output.
#[derive(Clone, Debug, PartialEq)]
pub struct PortDef {
    pub name: String,
    /// `flow`, `string`, `number`, `boolean` or `any`.
    pub ty: String,
    pub default: Option<String>,
    /// `Label:value` pairs; clicking the field cycles through them.
    pub options: Vec<String>,
}

impl PortDef {
    pub fn new(name: &str, ty: &str, default: Option<&str>) -> PortDef {
        PortDef { name: name.into(), ty: ty.into(), default: default.map(String::from), options: Vec::new() }
    }

    pub fn is_flow(&self) -> bool {
        self.ty == "flow"
    }

    pub fn has_options(&self) -> bool {
        !self.options.is_empty()
    }

    fn split(opt: &str) -> (&str, &str) {
        match opt.find(':') {
            Some(i) => (&opt[..i], &opt[i + 1..]),
            None => (opt, opt),
        }
    }

    /// The label shown for a stored value (the value itself if unknown).
    pub fn label_for_value<'a>(&'a self, value: &'a str) -> &'a str {
        for o in &self.options {
            let (label, v) = Self::split(o);
            if o.contains(':') && v == value {
                return label;
            }
        }
        value
    }

    /// The option after `current` (the first one if `current` isn't an option).
    pub fn next_option_value(&self, current: &str) -> String {
        if self.options.is_empty() {
            return current.to_string();
        }
        for (i, o) in self.options.iter().enumerate() {
            if Self::split(o).1 == current {
                return Self::split(&self.options[(i + 1) % self.options.len()]).1.to_string();
            }
        }
        Self::split(&self.options[0]).1.to_string()
    }
}

/// One kind of block.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    pub name: String,
    pub label: String,
    pub inputs: Vec<PortDef>,
    pub outputs: Vec<PortDef>,
    /// Python template; `{port}` placeholders. Empty for control blocks
    /// (they have a `generator`) and pass-through constants.
    pub code: String,
    pub generator: Option<String>,
    /// What the block does, for its info popup (blocks.json `description`;
    /// peripheral blocks: from the catalog).
    pub description: String,
    /// A short example of the block in use (blocks.json `example`).
    pub example: String,
}

impl BlockDef {
    pub fn generator_or_name(&self) -> &str {
        self.generator.as_deref().unwrap_or(&self.name)
    }
    pub fn data_inputs(&self) -> impl Iterator<Item = &PortDef> {
        self.inputs.iter().filter(|p| !p.is_flow())
    }
    pub fn data_outputs(&self) -> impl Iterator<Item = &PortDef> {
        self.outputs.iter().filter(|p| !p.is_flow())
    }
    pub fn flow_inputs(&self) -> impl Iterator<Item = &PortDef> {
        self.inputs.iter().filter(|p| p.is_flow())
    }
    pub fn flow_outputs(&self) -> impl Iterator<Item = &PortDef> {
        self.outputs.iter().filter(|p| p.is_flow())
    }
    pub fn input(&self, name: &str) -> Option<&PortDef> {
        self.inputs.iter().find(|p| p.name == name)
    }
    pub fn output(&self, name: &str) -> Option<&PortDef> {
        self.outputs.iter().find(|p| p.name == name)
    }

    /// What the info popup shows: the description, one line per input and
    /// output (name, type, default, choices) and the example.
    pub fn info(&self) -> BlockInfo {
        let ty_name = |t: &str| match t {
            "flow" => "flow",
            "string" => "text",
            "number" => "number",
            "boolean" => "yes/no",
            _ => "any value",
        };
        let port_line = |p: &PortDef, input: bool| -> String {
            if p.is_flow() {
                return if input {
                    format!("{} (flow): runs this block when the block wired above it is done", p.name)
                } else {
                    format!("{} (flow): what runs {}", p.name, flow_when(&p.name))
                };
            }
            let mut s = format!("{} ({})", p.name, ty_name(&p.ty));
            if input {
                if p.has_options() {
                    let opts: Vec<&str> = p.options.iter().map(|o| o.split(':').next().unwrap_or(o)).collect();
                    s.push_str(&format!(": one of {}", opts.join(", ")));
                    if let Some(d) = &p.default {
                        s.push_str(&format!(" (default {})", p.label_for_value(d)));
                    }
                } else if let Some(d) = p.default.as_deref().filter(|d| !d.is_empty()) {
                    s.push_str(&format!(", default {d}"));
                }
                s.push_str("; type a value or wire one in");
            }
            s
        };
        BlockInfo {
            title: self.label.clone(),
            description: if self.description.is_empty() { format!("The {} block.", self.label) } else { self.description.clone() },
            inputs: self.inputs.iter().map(|p| port_line(p, true)).collect(),
            outputs: self.outputs.iter().map(|p| port_line(p, false)).collect(),
            example: self.example.clone(),
            code: self.code.clone(),
        }
    }
}

/// When a named flow output runs.
fn flow_when(name: &str) -> &'static str {
    match name {
        "body" => "inside (each time round, or when the event happens)",
        "done" => "after it has finished",
        "true" => "when the condition holds",
        "false" => "when it doesn't",
        _ => "next",
    }
}

/// The text of a block's info popup.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub title: String,
    pub description: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub example: String,
    /// The Python template ("" for control blocks and constants).
    pub code: String,
}

/// A palette category.
#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub name: String,
    /// 0xRRGGBBAA.
    pub color: u32,
    pub blocks: Vec<BlockDef>,
    /// For generated peripheral categories: the peripheral type.
    pub peripheral: Option<String>,
    /// Folded into its header when the editor opens (blocks.json
    /// `collapsed`; every peripheral category). The core categories start
    /// open, the extra ones folded.
    pub collapsed: bool,
}

/// One method of a peripheral type, from the host catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct PeripheralMethod {
    pub name: String,
    pub description: String,
    /// (name, port type, default)
    pub params: Vec<(String, String, String)>,
    /// Port type of the result, or "" for none.
    pub returns: String,
}

/// A registered peripheral type, from the host catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct PeripheralType {
    pub name: String,
    pub description: String,
    pub methods: Vec<PeripheralMethod>,
}

/// Parse the catalog in its JSON form
/// (`[[type, description, [[method, description, [[param, type, default]], returns]]]]`).
pub fn catalog_from_json(j: &Json) -> Result<Vec<PeripheralType>, String> {
    let bad = || "malformed peripheral catalog".to_string();
    let s = |j: &Json| j.as_str().map(String::from).ok_or_else(bad);
    let mut out = Vec::new();
    for t in j.as_array().ok_or_else(bad)? {
        let t = t.as_array().ok_or_else(bad)?;
        let [name, desc, methods] = t else { return Err(bad()) };
        let mut ms = Vec::new();
        for m in methods.as_array().ok_or_else(bad)? {
            let m = m.as_array().ok_or_else(bad)?;
            let [mname, mdesc, params, ret] = m else { return Err(bad()) };
            let mut ps = Vec::new();
            for p in params.as_array().ok_or_else(bad)? {
                let p = p.as_array().ok_or_else(bad)?;
                let [pn, pt, pd] = p else { return Err(bad()) };
                ps.push((s(pn)?, s(pt)?, s(pd)?));
            }
            ms.push(PeripheralMethod { name: s(mname)?, description: s(mdesc)?, params: ps, returns: s(ret)? });
        }
        out.push(PeripheralType { name: s(name)?, description: s(desc)?, methods: ms });
    }
    Ok(out)
}

/// Every block the editor knows.
#[derive(Clone, Debug)]
pub struct Registry {
    pub categories: Vec<Category>,
}

fn parse_color(hex: &str) -> u32 {
    let h = hex.trim_start_matches('#');
    (u32::from_str_radix(h, 16).unwrap_or(0x888888) << 8) | 0xff
}

/// Java's `String.hashCode`.
pub fn java_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(u as i32);
    }
    h
}

/// The old registry's deterministic color for a peripheral type
/// (`VisualBlockRegistry.generateColor`), as 0xRRGGBBAA.
pub fn peripheral_color(name: &str) -> u32 {
    let hue = (java_hash(name).wrapping_abs() % 360) as f32 / 360.0f32;
    let (s, b) = (0.6f32, 0.8f32);
    let hi = ((hue * 6.0) as i32) % 6;
    let f = hue * 6.0 - hi as f32;
    let p = b * (1.0 - s);
    let q = b * (1.0 - f * s);
    let t = b * (1.0 - (1.0 - f) * s);
    let (r, g, bl) = match hi {
        0 => (b, t, p),
        1 => (q, b, p),
        2 => (p, b, t),
        3 => (p, q, b),
        4 => (t, p, b),
        _ => (b, p, q),
    };
    let c = |v: f32| ((v * 255.0) as i32 as u32) & 0xff;
    (c(r) << 24) | (c(g) << 16) | (c(bl) << 8) | 0xff
}

/// `snake_case` → `Snake Case`.
pub fn capitalize_label(snake: &str) -> String {
    snake
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Registry {
    /// The built-in blocks (blocks.json).
    pub fn builtin() -> Registry {
        Registry::parse(BLOCKS_JSON).expect("blocks.json is valid")
    }

    pub fn parse(text: &str) -> Result<Registry, String> {
        let j = Json::parse(text)?;
        let cats = j.get("categories").and_then(Json::as_array).ok_or("blocks.json has no categories")?;
        let mut categories = Vec::new();
        for c in cats {
            let name = c.get("name").and_then(Json::as_str).ok_or("category without a name")?;
            let color = parse_color(c.get("color").and_then(Json::as_str).unwrap_or("#888888"));
            let mut blocks = Vec::new();
            for b in c.get("blocks").and_then(Json::as_array).unwrap_or(&[]) {
                let ports = |key: &str, with_defaults: bool| -> Result<Vec<PortDef>, String> {
                    let mut v = Vec::new();
                    for p in b.get(key).and_then(Json::as_array).unwrap_or(&[]) {
                        let pname = p.get("name").and_then(Json::as_str).ok_or("port without a name")?;
                        let ty = p.get("type").and_then(Json::as_str).ok_or("port without a type")?;
                        let mut port = PortDef::new(pname, ty, None);
                        if with_defaults {
                            port.default = p.get("default").map(|d| match d {
                                Json::Str(s) => s.clone(),
                                other => other.to_string(),
                            });
                            port.options = p
                                .get("options")
                                .and_then(Json::as_array)
                                .map(|a| a.iter().filter_map(|o| o.as_str().map(String::from)).collect())
                                .unwrap_or_default();
                        }
                        v.push(port);
                    }
                    Ok(v)
                };
                blocks.push(BlockDef {
                    name: b.get("name").and_then(Json::as_str).ok_or("block without a name")?.to_string(),
                    label: b.get("label").and_then(Json::as_str).ok_or("block without a label")?.to_string(),
                    inputs: ports("inputs", true)?,
                    outputs: ports("outputs", false)?,
                    code: b.get("code").and_then(Json::as_str).unwrap_or("").to_string(),
                    generator: b.get("generator").and_then(Json::as_str).map(String::from),
                    description: b.get("description").and_then(Json::as_str).unwrap_or("").to_string(),
                    example: b.get("example").and_then(Json::as_str).unwrap_or("").to_string(),
                });
            }
            let collapsed = matches!(c.get("collapsed"), Some(Json::Bool(true)));
            categories.push(Category { name: name.to_string(), color, blocks, peripheral: None, collapsed });
        }
        Ok(Registry { categories })
    }

    /// Add one category per peripheral type, one block per method
    /// (`VisualBlockRegistry.generateBlocksFromPeripherals`).
    pub fn add_peripherals(&mut self, types: &[PeripheralType]) {
        for t in types {
            let mut blocks = Vec::new();
            for m in &t.methods {
                let mut inputs = vec![PortDef::new("flow", "flow", None), PortDef::new("name", "string", Some(""))];
                for (pn, pt, pd) in &m.params {
                    inputs.push(PortDef::new(pn, pt, Some(pd)));
                }
                let mut outputs = vec![PortDef::new("flow", "flow", None)];
                if !m.returns.is_empty() {
                    outputs.push(PortDef::new("result", &m.returns, None));
                }
                let mut code = String::new();
                if !m.returns.is_empty() {
                    code.push_str("{result} = ");
                }
                code.push_str(&format!("(peripheral.wrap({{name}}) or peripheral.find(\"{}\")).{}(", t.name, m.name));
                let args: Vec<String> = m.params.iter().map(|(pn, _, _)| format!("{{{pn}}}")).collect();
                code.push_str(&args.join(", "));
                code.push(')');
                let label = if m.description.is_empty() { capitalize_label(&m.name) } else { m.description.clone() };
                let what = if m.description.is_empty() { format!("Calls {}() on a {} peripheral.", m.name, t.name) } else { format!("{}.", m.description.trim_end_matches('.')) };
                let mut description = format!("{what} Peripheral: {} ({}).", t.name, if t.description.is_empty() { "no description" } else { t.description.as_str() });
                description.push_str(" Leave name empty to use the first one found next to the computer, or type the name `peripherals` lists.");
                if !m.returns.is_empty() {
                    description.push_str(" result is what the method returns.");
                }
                let example = format!(
                    "Start -> {label}{} with name empty.",
                    if m.params.is_empty() { String::new() } else { format!(" ({})", m.params.iter().map(|(n, _, d)| if d.is_empty() { n.clone() } else { format!("{n} {d}") }).collect::<Vec<_>>().join(", ")) }
                );
                blocks.push(BlockDef { name: format!("{}_{}", t.name, m.name), label, inputs, outputs, code, generator: None, description, example });
            }
            if !blocks.is_empty() {
                self.categories.push(Category {
                    name: capitalize_label(&t.name),
                    color: peripheral_color(&t.name),
                    blocks,
                    peripheral: Some(t.name.clone()),
                    collapsed: true,
                });
            }
        }
    }

    pub fn find(&self, name: &str) -> Option<&BlockDef> {
        self.categories.iter().flat_map(|c| c.blocks.iter()).find(|b| b.name == name)
    }

    pub fn category_of(&self, name: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.blocks.iter().any(|b| b.name == name))
    }

    /// The category color of a block (grey if unknown).
    pub fn color_for(&self, name: &str) -> u32 {
        self.category_of(name).map_or(0x888888ff, |c| c.color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_blocks_parse_with_defaults_and_options() {
        let r = Registry::builtin();
        let names: Vec<&str> = r.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Control", "Output", "Timing", "Redstone", "Events", "Math", "Compare", "Convert", "Constants", "App"]);
        let side = r.find("set_redstone").unwrap().input("side").unwrap();
        assert_eq!(side.default.as_deref(), Some("3"));
        assert_eq!(side.label_for_value("2"), "Front");
        assert_eq!(side.next_option_value("5"), "0");
        assert_eq!(side.next_option_value("nope"), "0");
        assert_eq!(r.find("loop").unwrap().generator_or_name(), "loop");
        assert_eq!(r.find("print").unwrap().code, "terminal.println({text})");
        assert_eq!(r.color_for("print"), 0x4caf50ff);
        assert!(r.find("to_str").unwrap().input("value").unwrap().default.is_none());
    }

    #[test]
    fn every_block_has_a_description_and_an_example() {
        let r = Registry::builtin();
        for b in r.categories.iter().flat_map(|c| &c.blocks) {
            assert!(b.description.len() > 10, "{} has no description", b.name);
            assert!(!b.example.is_empty(), "{} has no example", b.name);
            let info = b.info();
            assert_eq!(info.inputs.len(), b.inputs.len());
            assert_eq!(info.outputs.len(), b.outputs.len());
        }
        let side = r.find("set_redstone").unwrap().info();
        assert!(side.inputs[1].contains("one of Down, Up, Front, Back, Left, Right (default Back)"), "{:?}", side.inputs);
        assert!(r.find("loop").unwrap().info().outputs[1].starts_with("done (flow): what runs after"));
    }

    #[test]
    fn core_categories_start_open_and_the_extra_ones_folded() {
        let mut r = Registry::builtin();
        let j = Json::parse(include_str!("../tests/vpl/peripheral_catalog.json")).unwrap();
        r.add_peripherals(&catalog_from_json(&j).unwrap());
        let folded: Vec<&str> = r.categories.iter().filter(|c| c.collapsed).map(|c| c.name.as_str()).collect();
        assert_eq!(folded, ["Redstone", "Events", "Convert", "App", "Test Lamp"]);
        let lamp = r.find("test_lamp_set_label").unwrap().info();
        assert!(lamp.description.starts_with("Show a label. Peripheral: test_lamp (Test lamp)."), "{}", lamp.description);
        assert_eq!(r.find("test_lamp_set_label").unwrap().example, "Start -> Show a label (text, bright False) with name empty.");
    }

    #[test]
    fn peripheral_blocks_match_the_old_registry() {
        let j = Json::parse(include_str!("../tests/vpl/peripheral_catalog.json")).unwrap();
        let mut r = Registry::builtin();
        r.add_peripherals(&catalog_from_json(&j).unwrap());
        let c = r.categories.last().unwrap();
        assert_eq!((c.name.as_str(), c.peripheral.as_deref()), ("Test Lamp", Some("test_lamp")));
        let get = r.find("test_lamp_get_level").unwrap();
        assert_eq!(get.label, "Get Level");
        assert_eq!(get.code, "{result} = (peripheral.wrap({name}) or peripheral.find(\"test_lamp\")).get_level()");
        let set = r.find("test_lamp_set_label").unwrap();
        assert_eq!(set.label, "Show a label");
        assert_eq!(set.code, "(peripheral.wrap({name}) or peripheral.find(\"test_lamp\")).set_label({text}, {bright})");
        assert_eq!(set.input("bright").unwrap().default.as_deref(), Some("False"));
        // Java: Math.abs("test_lamp".hashCode()) % 360 drives the hue.
        assert_eq!(java_hash("test_lamp"), {
            let mut h: i32 = 0;
            for b in "test_lamp".bytes() {
                h = h.wrapping_mul(31).wrapping_add(b as i32);
            }
            h
        });
        assert_eq!(peripheral_color("test_lamp") & 0xff, 0xff);
    }

    #[test]
    fn labels_capitalize_snake_case() {
        assert_eq!(capitalize_label("redstone_link"), "Redstone Link");
        assert_eq!(capitalize_label("get__level_"), "Get Level");
    }
}
