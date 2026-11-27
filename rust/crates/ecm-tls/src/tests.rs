use super::*;
use std::io::{self, Read, Write};
use std::collections::VecDeque;
use std::sync::{Arc as A, Mutex};
use x509::SubjectName;

/// One end of an in-memory connection; reading an empty pipe blocks (spins) until the other thread writes.
#[derive(Clone)]
struct Pipe {
    rx: A<Mutex<(VecDeque<u8>, bool)>>,
    tx: A<Mutex<(VecDeque<u8>, bool)>>,
}

fn pipe() -> (Pipe, Pipe) {
    let a = A::new(Mutex::new((VecDeque::new(), false)));
    let b = A::new(Mutex::new((VecDeque::new(), false)));
    (Pipe { rx: a.clone(), tx: b.clone() }, Pipe { rx: b, tx: a })
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut q = self.rx.lock().unwrap();
            if !q.0.is_empty() {
                let n = buf.len().min(q.0.len());
                for b in buf.iter_mut().take(n) {
                    *b = q.0.pop_front().unwrap();
                }
                return Ok(n);
            }
            if q.1 {
                return Ok(0);
            }
            drop(q);
            std::thread::yield_now();
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx.lock().unwrap().0.extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.tx.lock().unwrap().1 = true;
    }
}

const NOW: u64 = 1_795_000_000;
const CA: [u8; 32] = [7u8; 32];
const CA_NAME: &str = "Test Village CA";

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn identity(names: &[&str], from: u64, to: u64, ca: &[u8; 32]) -> Identity {
    let key = [3u8; 32];
    let names: Vec<SubjectName> = names.iter().map(|n| SubjectName::parse(n).unwrap()).collect();
    let leaf = x509::leaf_certificate(ca, CA_NAME, &x509::public_of(&key), &names, &[1; 16], from, to);
    Identity::new(vec![leaf], x509::pkcs8(&key)).unwrap()
}

fn village() -> Trust {
    Trust::with(vec![Authority::from_key(&x509::public_of(&CA), CA_NAME, "village.anchor")])
}

/// A server thread answering one request with `PONG <request>`; returns what the client got.
fn talk(id: Identity, trust: &mut Trust, name: &str, peer: [u8; 4], check: Check) -> Result<String, TlsError> {
    let (c, s) = pipe();
    let server = std::thread::spawn(move || {
        let Ok(mut t) = id.accept(s) else { return };
        let mut buf = [0u8; 64];
        let n = t.read(&mut buf).unwrap_or(0);
        let _ = t.write_all(format!("PONG {}", String::from_utf8_lossy(&buf[..n])).as_bytes());
        t.close();
    });
    let r = trust.connect(c, name, peer, check).map(|mut t| {
        t.write_all(b"ping").unwrap();
        t.flush().unwrap();
        let mut out = String::new();
        t.read_to_string(&mut out).unwrap();
        format!("{} {}", t.version(), out)
    });
    let _ = server.join();
    r
}

#[test]
fn ingame_authority_by_key_for_names_and_addresses() {
    let t = now();
    let id = identity(&["shop.ecm", "100.65.0.21"], t - 3600, t + 86_400, &CA);
    let mut trust = village();
    assert_eq!(talk(id.clone(), &mut trust, "shop.ecm", [100, 65, 0, 21], Check::Full).unwrap(), "TLSv1.3 PONG ping");
    assert_eq!(talk(id.clone(), &mut trust, "100.65.0.21", [100, 65, 0, 21], Check::Full).unwrap(), "TLSv1.3 PONG ping");
    // Another name, or an address the certificate does not list.
    assert!(matches!(talk(id.clone(), &mut trust, "bank.ecm", [100, 65, 0, 21], Check::Full), Err(TlsError::WrongName(_))));
    assert!(matches!(talk(id, &mut trust, "10.0.0.9", [10, 0, 0, 9], Check::Full), Err(TlsError::WrongName(_))));
}

#[test]
fn unknown_authority_expired_and_no_authorities() {
    let t = now();
    // Signed by someone this computer does not trust.
    let other = identity(&["shop.ecm"], t - 3600, t + 86_400, &[8u8; 32]);
    let mut trust = village();
    let e = talk(other.clone(), &mut trust, "shop.ecm", [10, 0, 0, 1], Check::Full).unwrap_err();
    assert!(matches!(e, TlsError::UnknownAuthority(_)), "{e:?}");
    assert!(e.to_string().contains("cert trust"), "{e}");
    // ... unless told not to check.
    assert_eq!(talk(other.clone(), &mut trust, "shop.ecm", [10, 0, 0, 1], Check::None).unwrap(), "TLSv1.3 PONG ping");
    // Out of date, and not valid yet.
    for (from, to) in [(t - 2 * 86_400, t - 86_400), (t + 86_400, t + 2 * 86_400)] {
        let e = talk(identity(&["shop.ecm"], from, to, &CA), &mut trust, "shop.ecm", [10, 0, 0, 1], Check::Full).unwrap_err();
        assert!(matches!(e, TlsError::Expired(_)), "{e:?}");
    }
    // A computer with an empty /etc/ssl/certs is told so before any bytes move.
    let e = talk(other, &mut Trust::public_only(), "shop.ecm", [10, 0, 0, 1], Check::Full).unwrap_err();
    assert!(e.to_string().contains("trusts no in-game authority"), "{e}");
}

#[test]
fn an_ingame_authority_cannot_vouch_for_a_public_address() {
    let t = now();
    let id = identity(&["repo.evanwitulski.com"], t - 3600, t + 86_400, &CA);
    let mut trust = village();
    // The same certificate is fine inside the game ...
    assert!(talk(id.clone(), &mut trust, "repo.evanwitulski.com", [100, 64, 0, 9], Check::Full).is_ok());
    // ... and refused at a public address, where only the real roots count.
    let e = talk(id, &mut trust, "repo.evanwitulski.com", [203, 0, 113, 9], Check::Full).unwrap_err();
    assert!(matches!(e, TlsError::UnknownAuthority(_)), "{e:?}");
    assert!(e.to_string().contains("no public authority"), "{e}");
}

#[test]
fn public_and_game_addresses() {
    for ip in [[10, 0, 0, 1], [172, 16, 0, 1], [172, 31, 255, 1], [192, 168, 1, 1], [100, 64, 0, 1], [100, 127, 0, 1], [127, 0, 0, 1], [169, 254, 1, 1], [0, 0, 0, 0], [224, 0, 0, 254]] {
        assert!(!is_public(ip), "{ip:?}");
    }
    for ip in [[1, 1, 1, 1], [100, 63, 0, 1], [100, 128, 0, 1], [172, 15, 0, 1], [172, 32, 0, 1], [192, 167, 0, 1], [203, 0, 113, 9]] {
        assert!(is_public(ip), "{ip:?}");
    }
}

#[test]
fn a_real_chain_is_checked_against_the_public_roots() {
    assert!(webpki_roots::TLS_SERVER_ROOTS.len() > 50);
    // A server that is not TLS at all.
    let (c, mut s) = pipe();
    let h = std::thread::spawn(move || {
        let mut b = [0u8; 16];
        let _ = s.read(&mut b);
        let _ = s.write_all(b"HTTP/1.0 400 Bad Request\r\n\r\n");
    });
    let e = Trust::public_only().connect(c, "example.com", [203, 0, 113, 1], Check::Full).err().unwrap();
    assert!(matches!(e, TlsError::Handshake(_)), "{e:?}");
    let _ = h.join();
}

#[test]
fn trust_folder_pem_and_anchor_files() {
    let dir = std::env::temp_dir().join(format!("ecm-tls-certs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = x509::root_certificate(&CA, CA_NAME, &[2; 16], NOW - 86_400, NOW + 86_400 * 3650);
    std::fs::write(dir.join("village.pem"), x509::pem(x509::CERTIFICATE, &root)).unwrap();
    std::fs::write(dir.join("player.anchor"), Authority::anchor_line(&x509::public_of(&[9u8; 32]), "Evan's CA")).unwrap();
    std::fs::write(dir.join("broken.anchor"), "rsa 00 nope\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
    let mut trust = Trust::load(dir.to_str().unwrap());
    let names: Vec<&str> = trust.ingame.iter().map(|a| a.common_name.as_str()).collect();
    assert_eq!(names, vec!["Evan's CA", CA_NAME]);
    assert_eq!(trust.skipped, vec!["broken.anchor"]);
    // The root certificate from the .pem file verifies a leaf, like the key-only anchor does.
    let t = now();
    assert!(talk(identity(&["shop.ecm"], t - 60, t + 3600, &CA), &mut trust, "shop.ecm", [10, 0, 0, 1], Check::Full).is_ok());
    assert!(talk(identity(&["shop.ecm"], t - 60, t + 3600, &[9u8; 32]), &mut trust, "shop.ecm", [10, 0, 0, 1], Check::Full).is_err(), "issuer name differs");
    assert!(Trust::load("/no/such/folder").ingame.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn identity_from_pem_and_bulk_data() {
    let t = now();
    let key = [3u8; 32];
    let leaf = x509::leaf_certificate(&CA, CA_NAME, &x509::public_of(&key), &[SubjectName::Dns("files.ecm".into())], &[4; 16], t - 60, t + 3600);
    let both = format!("{}{}", x509::pem(x509::CERTIFICATE, &leaf), x509::pem(x509::PRIVATE_KEY, &x509::pkcs8(&key)));
    let id = Identity::from_pem(&both, &both).unwrap();
    assert!(Identity::from_pem(&both, "").is_err());
    assert!(Identity::new(vec![leaf.clone()], x509::pkcs8(&[4u8; 32])).is_err(), "key of another certificate");

    let (c, s) = pipe();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7) as u8).collect();
    let expect = data.clone();
    let server = std::thread::spawn(move || {
        let mut t = id.accept(s).unwrap();
        t.write_all(&data).unwrap();
        t.close();
    });
    let mut t = village().connect(c, "files.ecm", [10, 0, 0, 1], Check::Full).unwrap();
    assert_eq!(t.peer_certificates(), vec![leaf]);
    let mut got = Vec::new();
    t.read_to_end(&mut got).unwrap();
    assert_eq!(got, expect);
    server.join().unwrap();
}
