//! The Rust code generator against the old Java one: every sample `.vpl`
//! (`tests/vpl/`) loads like the old editor loaded it (ids from 1) and must
//! generate exactly the Python the Java `VisualCodeGenerator` produced
//! (`tests/vpl/<name>.py`, exported by the one-off `VisualGoldenExportTest`
//! before the Java screen was removed).

use ecm_ui::Json;
use ecm_visual::blocks::{catalog_from_json, Registry};
use ecm_visual::codegen::generate;
use ecm_visual::graph::Graph;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("vpl")
}

fn registry() -> Registry {
    let mut r = Registry::builtin();
    let cat = Json::parse(&std::fs::read_to_string(dir().join("peripheral_catalog.json")).unwrap()).unwrap();
    r.add_peripherals(&catalog_from_json(&cat).unwrap());
    r
}

#[test]
fn every_sample_generates_the_same_python_as_the_java_generator() {
    let reg = registry();
    let mut checked = 0;
    let mut entries: Vec<_> = std::fs::read_dir(dir()).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for vpl in entries.iter().filter(|p| p.extension().is_some_and(|e| e == "vpl")) {
        let golden_path = vpl.with_extension("py");
        let golden = std::fs::read_to_string(&golden_path).unwrap_or_else(|_| panic!("no golden for {}", vpl.display()));
        let (g, _) = Graph::load(&std::fs::read_to_string(vpl).unwrap(), &reg, 1).unwrap();
        let got = generate(&g, &reg, None);
        if got != golden {
            let line = got.lines().zip(golden.lines()).position(|(a, b)| a != b).unwrap_or(got.lines().count().min(golden.lines().count()));
            panic!(
                "{}: generated Python differs from the Java golden at line {}\n--- got ---\n{got}<EOF>\n--- java ---\n{golden}<EOF>",
                vpl.file_name().unwrap().to_string_lossy(),
                line + 1
            );
        }
        checked += 1;
    }
    assert!(checked >= 9, "only {checked} samples");
}

#[test]
fn saving_and_reloading_a_sample_keeps_its_python() {
    let reg = registry();
    for name in ["branches", "math_constants", "events", "peripheral"] {
        let text = std::fs::read_to_string(dir().join(format!("{name}.vpl"))).unwrap();
        let (g, _) = Graph::load(&text, &reg, 1).unwrap();
        let (again, skipped) = Graph::load(&g.save(&reg), &reg, 1).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(generate(&again, &reg, None), generate(&g, &reg, None), "{name}");
        assert_eq!(again.blocks.len(), g.blocks.len());
    }
}
