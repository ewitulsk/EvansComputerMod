//! The shared test vectors (abi/ui-test-vectors/): every valid vector decodes
//! to its canonical JSON and re-encodes to the same bytes; every invalid one
//! is rejected. The Java codec runs the same files (UiProtoVectorsTest), so
//! the two can't drift.

use ecm_ui_proto::*;

const VALID: &str = include_str!("../../../../abi/ui-test-vectors/valid.txt");
const INVALID: &str = include_str!("../../../../abi/ui-test-vectors/invalid.txt");

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#'))
}

#[test]
fn every_valid_vector_round_trips() {
    let mut names = std::collections::BTreeSet::new();
    let mut count = 0;
    for line in lines(VALID) {
        let mut parts = line.splitn(3, ' ');
        let (name, hex, json) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let bytes = unhex(hex);
        let msg = Message::decode_exact(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(msg.to_json(), json, "{name}: JSON differs");
        assert_eq!(msg.to_bytes(), bytes, "{name}: re-encoding differs");
        names.insert(msg.name());
        count += 1;
    }
    assert!(count >= 100, "only {count} vectors");
    // Every message type has at least one vector.
    let all = [
        "Hello", "Welcome", "Error", "SurfaceCreate", "TreeOps", "Pointer", "Key", "Text", "Place", "Notify",
        "PickFile", "FilePicked", "WillSuspend", "SaveState", "Restore", "BackgroundDone", "BindChord",
        "SetTheme", "ThemeChanged", "PointerLock", "Configure", "AckConfigure", "Commit", "Frame",
    ];
    for n in all {
        assert!(names.contains(n), "no vector for {n}");
    }
}

#[test]
fn every_invalid_vector_is_rejected() {
    let mut count = 0;
    for line in lines(INVALID) {
        let mut parts = line.splitn(2, ' ');
        let (name, hex) = (parts.next().unwrap(), parts.next().unwrap());
        let bytes = unhex(hex);
        if let Ok(m) = Message::decode_exact(&bytes) {
            panic!("{name}: decoded as {}", m.to_json());
        }
        count += 1;
    }
    assert!(count >= 30, "only {count} invalid vectors");
}

#[test]
fn stream_decoder_splits_and_reassembles() {
    let a = Message::Commit(Commit { surface: 3 });
    let b = Message::Text(Text { surface: 3, text: "h\u{e9}llo".into() });
    let bytes = encode_all([&a, &b]);
    let mut d = StreamDecoder::new();
    let mut got = Vec::new();
    for chunk in bytes.chunks(3) {
        d.push(chunk);
        while let Some(m) = d.next_message().unwrap() {
            got.push(m);
        }
    }
    assert_eq!(got, vec![a.clone(), b.clone()]);
    assert_eq!(Message::decode_all(&bytes).unwrap(), vec![a, b]);
}

#[test]
fn specific_rejections_name_the_problem() {
    // Unknown opcode with a complete header is reported at once, not as incomplete.
    assert_eq!(Message::decode(&[0x77, 0x77, 0, 0, 0, 0, 0, 0]), Err(DecodeError::UnknownOpcode(0x7777)));
    // An oversize header is rejected before its body arrives.
    let mut h = vec![0x01, 0x06, 0, 0];
    h.extend_from_slice(&(MAX_MESSAGE + 1).to_le_bytes());
    assert_eq!(Message::decode(&h).unwrap_err().error_code(), ErrorCode::Limit);
    // A group may not carry text.
    let bad = Message::TreeOps(TreeOps {
        surface: 1,
        ops: vec![TreeOp::Create { id: 1, parent: 0, index: APPEND, kind: NodeKind::Group, props: vec![Prop::Text("x".into())] }],
    });
    assert_eq!(Message::decode(&bad.to_bytes()), Err(DecodeError::PropNotAllowed("TreeOp.Create.props")));
    // Props must be sorted.
    let bad = Message::TreeOps(TreeOps { surface: 1, ops: vec![TreeOp::Set { id: 1, props: vec![Prop::Y(1), Prop::X(1)] }] });
    assert_eq!(Message::decode(&bad.to_bytes()), Err(DecodeError::Unsorted("TreeOp.Set.props")));
}

#[test]
fn theme_tokens_stay_sorted() {
    let mut t = Theme::default();
    t.set("text", 1);
    t.set("bg", 2);
    t.set("accent", 3);
    t.set("bg", 4);
    assert_eq!(t.entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["accent", "bg", "text"]);
    assert_eq!(t.get("bg"), Some(4));
    let m = Message::SetTheme(SetTheme { theme: t });
    assert!(Message::decode_exact(&m.to_bytes()).is_ok());
}
