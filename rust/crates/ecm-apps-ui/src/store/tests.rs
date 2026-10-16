//! The App Store against the sample repository in memory, an in-memory app
//! manager and a scripted UI connection.

use super::*;
use crate::appctl::FakeCtl;
use crate::samples;
use ecm_pkg::client::MemoryNet;
use ecm_ui::{samples as ui_samples, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;

pub const REPO_URL: &str = "http://10.0.7.1:8080/";

pub fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("ecm-store-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

fn client(name: &str, secret: Option<&str>, pin: bool) -> Client<MemoryNet> {
    let c = Client::new(&tmp(name), samples::memory_repo(REPO_URL, secret));
    let key = secret.and_then(ecm_pkg::sign::public_of);
    c.add_repo("samples", REPO_URL, if pin { key.as_deref() } else { None }).unwrap();
    c
}

fn store_with(c: &mut Client<MemoryNet>) -> Store {
    let (catalog, errors) = load_catalog(c, true, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    Store { catalog, installed: c.installed(), repos: c.repos(), ..Default::default() }
}

#[test]
fn cards_details_and_planning() {
    let mut c = client("model", None, false);
    let mut s = store_with(&mut c);
    let names: Vec<String> = s.discover().into_iter().map(|c| c.name).collect();
    assert_eq!(names, vec!["Counter", "Net Check", "Notes"]);
    assert!(s.discover().iter().all(|c| c.action == Action::Install && !c.verified && c.trust == "unverified publisher"));
    s.query = "visual".into();
    assert_eq!(s.discover().len(), 1);
    s.query.clear();

    let d = s.detail_of("dev.ecm.netcheck").unwrap();
    assert_eq!(d.permissions.len(), 1);
    assert_eq!(d.permissions[0].1.title, "Use the network");
    assert_eq!(d.permissions[0].2, None, "not installed: no answer yet");
    assert!(d.size.unwrap() > 1000);
    let notes = s.detail_of("dev.ecm.notes").unwrap();
    assert_eq!(notes.screenshots, vec!["screenshots/1.png"]);
    assert_eq!(notes.card.publisher, "ECM Samples");

    // Installed at an older version: an update; installed at the same: Open.
    let (_, old) = ecm_pkg::package::build(vec![
        ("app.toml".into(), b"id = \"dev.ecm.notes\"\nname = \"Notes\"\nversion = \"0.9.0\"\nentry = \"main.py\"\n".to_vec()),
        ("main.py".into(), b"print(1)\n".to_vec()),
    ])
    .unwrap();
    c.unpack(&old, "file x", &ecm_pkg::package::Trust::Unsigned, None).unwrap();
    s.installed = c.installed();
    assert_eq!(s.updates().len(), 1);
    assert_eq!(s.updates()[0].action, Action::Update { from: "0.9.0".into() });
    s.tab = Some(Tab::Installed);
    assert_eq!(s.cards().len(), 1);
    assert!(s.detail_of("dev.ecm.notes").unwrap().can_remove);

    // Queueing shows up on the card.
    s.act("dev.ecm.notes");
    s.act("dev.ecm.notes");
    assert_eq!(s.queue.len(), 1, "queued once");
    assert!(matches!(s.detail_of("dev.ecm.notes").unwrap().card.action, Action::Busy { .. }));
}

#[test]
fn trust_badges_follow_signatures_and_pinning() {
    let (_, secret) = ecm_pkg::sign::keypair_from_seed(&[5u8; 32]);
    let mut c = client("signed", Some(&secret), true);
    let s = store_with(&mut c);
    assert!(s.discover().iter().all(|c| c.verified && c.trust == "verified publisher"));
    let mut c = client("unpinned", Some(&secret), false);
    let s = store_with(&mut c);
    assert!(s.discover().iter().all(|c| !c.verified && c.trust == "unverified publisher (signed)"));
    // A pinned key with unsigned packages: refused, never installed.
    let mut c = client("refused", None, false);
    c.add_repo("samples", REPO_URL, Some(&"ab".repeat(32))).unwrap();
    let s = store_with(&mut c);
    assert!(s.discover().iter().all(|c| c.action == Action::Refused));
}

/// A store app on a scripted connection, configured at `w x h`.
pub fn start(name: &str, w: i32, h: i32) -> (StoreApp<MemoryNet, FakeCtl>, MockConnection) {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    mock.push(&[wl]);
    let mut ctl = FakeCtl::default();
    ctl.add(crate::STORE_ID, "App Store", "1.0.0", &["packages", "network"], &[]);
    let mut sa = StoreApp::new(Box::new(mock.clone()), client(name, None, false), ctl, true).unwrap();
    mock.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::ACTIVATED })]);
    // Take the configure, then draw.
    sa.step(0);
    sa.step(0);
    (sa, mock)
}

pub fn click(sa: &mut StoreApp<MemoryNet, FakeCtl>, mock: &MockConnection, id: &str) {
    sa.step(0);
    let hit = sa.app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"));
    mock.push(&ui_samples::click(hit, 10, 10));
    sa.step(0);
    sa.step(0);
}

#[test]
fn install_open_and_remove_through_the_window() {
    let (mut sa, mock) = start("flow", 640, 400);
    sa.step(0);
    assert_eq!(sa.store.borrow().catalog.apps.len(), 3);
    assert_eq!(sa.store.borrow().status, "3 apps from 1 repository");
    let uploads = mock.state.borrow().uploads.len();
    assert_eq!(uploads, 3, "three icons, decoded and uploaded as RGBA");
    assert!(mock.state.borrow().uploads.iter().all(|u| u.1 == 1));

    // Open Net Check's page: its permission in words, then install.
    click(&mut sa, &mock, "name:dev.ecm.netcheck");
    assert_eq!(sa.store.borrow().detail.as_deref(), Some("dev.ecm.netcheck"));
    let sent_before = mock.sent().len();
    click(&mut sa, &mock, "page:dev.ecm.netcheck");
    let s = sa.store.borrow();
    assert_eq!(s.status, "Installed Net Check 1.0.0 (unverified publisher)");
    assert!(s.installed_version("dev.ecm.netcheck").is_some());
    drop(s);
    assert!(sa.ctl.log.contains(&"reindex".to_string()), "the app manager reindexed");
    assert!(sa.client.path("/apps/dev.ecm.netcheck/main.py").len() > 0);
    assert!(std::fs::metadata(sa.client.path("/apps/dev.ecm.netcheck/main.py")).is_ok());
    assert!(mock.sent().len() > sent_before, "progress was drawn");

    // Now Open launches it through the app manager.
    sa.ctl.add("dev.ecm.netcheck", "Net Check", "1.0.0", &["network"], &[]);
    click(&mut sa, &mock, "page:dev.ecm.netcheck");
    assert!(sa.ctl.log.iter().any(|l| l == "launch dev.ecm.netcheck"));
    assert_eq!(sa.store.borrow().status, "Opened Net Check");
    // Its page shows the current answer ("Asks first").
    let d = sa.store.borrow().detail_of("dev.ecm.netcheck").unwrap();
    assert_eq!(d.permissions[0].2, Some(PermState::Ask));

    // Installed tab, then remove from the page.
    click(&mut sa, &mock, "remove");
    assert_eq!(sa.store.borrow().status, "Removed Net Check (its data is kept)");
    assert!(sa.client.installed_manifest("dev.ecm.netcheck").is_none());
}

#[test]
fn screenshots_load_on_the_page_and_errors_are_shown() {
    let (mut sa, mock) = start("shots", 640, 400);
    click(&mut sa, &mock, "name:dev.ecm.notes");
    assert!(sa.store.borrow().pics.contains_key("shot:dev.ecm.notes:screenshots/1.png"));
    assert_eq!(mock.state.borrow().uploads.len(), 4);
    // The repository goes away: Refresh keeps the saved list and says so.
    sa.client.net.down.push("10.0.7.1".into());
    click(&mut sa, &mock, "back");
    click(&mut sa, &mock, "refresh");
    let s = sa.store.borrow();
    assert!(s.status.contains("showing the saved list"), "{}", s.status);
    assert_eq!(s.catalog.apps.len(), 3);
}

#[test]
fn every_size_class_renders() {
    for (w, h) in [(360, 640), (640, 400), (1024, 640)] {
        let (mut sa, mock) = start(&format!("size{w}"), w, h);
        sa.step(0);
        assert!(sa.app.core.window(1).unwrap().hit_id_of("act:dev.ecm.notes").is_some(), "{w}: install button");
        click(&mut sa, &mock, "name:dev.ecm.notes");
        assert!(sa.app.core.window(1).unwrap().hit_id_of("detail").is_some(), "{w}: the page");
    }
}


/// Click a text field and type into it (what the compositor sends).
fn type_into(sa: &mut StoreApp<MemoryNet, FakeCtl>, mock: &MockConnection, id: &str, text: &str) {
    click(sa, mock, id);
    mock.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
    sa.step(0);
    sa.step(0);
}

#[test]
fn repositories_are_added_and_removed_in_the_window() {
    // A phone with no repository: the Repositories page adds one by its URL.
    for (w, h) in [(240, 384), (640, 400), (1024, 640)] {
        let (mut sa, mock) = start(&format!("repos{w}"), w, h);
        sa.client.remove_repo("samples").unwrap();
        click(&mut sa, &mock, "refresh");
        assert!(sa.store.borrow().catalog.apps.is_empty());
        assert!(sa.store.borrow().status.starts_with("No repositories yet"), "{}", sa.store.borrow().status);
        click(&mut sa, &mock, "repos");
        assert!(sa.store.borrow().repos_page);
        // Control: Add with no address says what to type.
        click(&mut sa, &mock, "add_repo");
        assert!(sa.store.borrow().status.starts_with("Type the repository's address"), "{w}");
        assert!(sa.client.repos().is_empty());
        type_into(&mut sa, &mock, "repo_url", "10.0.7.1:8080/");
        click(&mut sa, &mock, "add_repo");
        let s = sa.store.borrow();
        assert_eq!(s.repos.len(), 1, "{w}");
        assert_eq!(s.repos[0].name, "10-0-7-1");
        assert_eq!(s.repos[0].url, REPO_URL);
        assert_eq!(s.catalog.apps.len(), 3, "{w}: the index was fetched");
        assert_eq!(s.status, "Added 10-0-7-1 (http://10.0.7.1:8080/): 3 apps", "{w}");
        assert!(!s.repos_page, "back to the apps");
        drop(s);
        assert!(sa.app.core.window(1).unwrap().hit_id_of("act:dev.ecm.netcheck").is_some(), "{w}: Net Check is listed");
        // Remove it again from the page.
        click(&mut sa, &mock, "repos");
        click(&mut sa, &mock, "remove_repo:10-0-7-1");
        assert!(sa.client.repos().is_empty());
        assert!(sa.store.borrow().catalog.apps.is_empty());
        assert_eq!(sa.store.borrow().status, "Removed the repository 10-0-7-1");
    }
    assert_eq!(repo_name_for("http://village.example:8080/"), "village-example");
    assert_eq!(repo_name_for("http:///"), "repo");
}

fn found(host: [u8; 4], port: u16, name: &str) -> ecm_disco::Found {
    ecm_disco::Found { kind: ecm_disco::types::PKG.into(), host, port, name: name.into() }
}

#[test]
fn repositories_on_the_network_are_listed_without_adding_them() {
    for (w, h) in [(240, 384), (1024, 640)] {
        // A phone with no repository, on a network where a computer runs pkg serve and disco serve.
        let (mut sa, mock) = start(&format!("found{w}"), w, h);
        sa.client.remove_repo("samples").unwrap();
        sa.client.net.found = vec![found([10, 0, 7, 1], 8080, "Lab-Apps")];
        click(&mut sa, &mock, "refresh");
        {
            let s = sa.store.borrow();
            assert_eq!(s.catalog.apps.len(), 3, "{w}: its apps are in the store");
            assert_eq!(s.status, "3 apps from 1 repository (1 found on the network)", "{w}");
            assert!(s.repos.is_empty(), "nothing was written to /etc/pkg/repos");
            assert_eq!(s.found[0].url, REPO_URL);
            assert!(s.catalog.apps.iter().all(|a| a.repo.name == "Lab-Apps" && a.repo.key.is_none()));
        }
        assert!(sa.client.repos().is_empty());
        // Install from it works like from any repository (unverified: nothing pinned).
        click(&mut sa, &mock, "act:dev.ecm.netcheck");
        assert_eq!(sa.store.borrow().status, "Installed Net Check 1.0.0 (unverified publisher)", "{w}");
        assert_eq!(sa.store.borrow().found.len(), 1, "{w}: still listed after the install's refresh");
        // The Repositories page offers to keep it.
        click(&mut sa, &mock, "repos");
        click(&mut sa, &mock, "add_found:Lab-Apps");
        let s = sa.store.borrow();
        assert_eq!(s.repos.len(), 1, "{w}");
        assert_eq!((s.repos[0].name.as_str(), s.repos[0].url.as_str()), ("Lab-Apps", REPO_URL));
        assert!(s.found.is_empty(), "{w}: configured now, not 'found' twice");
        assert_eq!(s.catalog.apps.len(), 3);
        drop(s);
        // Control: nothing on the network, nothing configured.
        sa.client.remove_repo("Lab-Apps").unwrap();
        sa.client.net.found.clear();
        click(&mut sa, &mock, "refresh");
        assert!(sa.store.borrow().status.starts_with("No repositories yet"), "{w}");
    }
}

#[test]
fn found_repositories_skip_configured_ones_and_get_free_names() {
    let configured = vec![Repo { name: "Lab".into(), url: "http://10.0.7.1:8080/".into(), key: Some("ab".repeat(32)) }];
    let got = ecm_pkg::client::found_repos(
        &configured,
        &[
            found([10, 0, 7, 1], 8080, "Lab"),          // the same URL: already configured (and pinned)
            found([10, 0, 7, 2], 8080, "Lab"),          // the name is taken
            found([10, 0, 7, 3], 8080, "has spaces"),   // not a repository name: from the address
            found([10, 0, 7, 3], 8080, ""),             // the same one again
            ecm_disco::Found { kind: "chat".into(), host: [10, 0, 7, 4], port: 7777, name: "x".into() },
        ],
    );
    let v: Vec<(&str, &str)> = got.iter().map(|r| (r.name.as_str(), r.url.as_str())).collect();
    assert_eq!(v, vec![("Lab-2", "http://10.0.7.2:8080/"), ("10-0-7-3", "http://10.0.7.3:8080/")]);
    assert!(got.iter().all(|r| r.key.is_none()), "a key from the network is never pinned");
}
