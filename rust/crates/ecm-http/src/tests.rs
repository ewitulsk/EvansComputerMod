use super::*;
use crate::native::Native;
use ecm_tls::x509::{self, SubjectName};
use ecm_tls::{Authority, Identity};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// A server on a loopback port answering each connection with `answer(request)`.
fn server(tls: Option<Identity>, answer: impl Fn(&str) -> Vec<u8> + Send + 'static) -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut plain: Box<dyn Stream> = match &tls {
                Some(id) => match id.accept(s) {
                    Ok(t) => Box::new(t),
                    Err(_) => continue,
                },
                None => Box::new(s),
            };
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = plain.read(&mut buf).unwrap_or(0);
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).into_owned();
                if let Some(i) = text.find("\r\n\r\n") {
                    let want: usize = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                    if req.len() >= i + 4 + want {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&req).into_owned();
            log.lock().unwrap().push(text.clone());
            let _ = plain.write_all(&answer(&text));
            let _ = plain.flush();
        }
    });
    (port, seen)
}

fn ok(body: &[u8]) -> Vec<u8> {
    let mut v = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n", body.len()).into_bytes();
    v.extend_from_slice(body);
    v
}

fn client() -> Client<Native> {
    Client::new(Native::default()).agent("test/1")
}

#[test]
fn content_length_chunked_and_until_close() {
    let (port, seen) = server(None, |req| {
        let path = req.split(' ').nth(1).unwrap_or("");
        match path {
            "/len" => ok(b"hello"),
            "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n".to_vec(),
            "/close" => b"HTTP/1.0 200 OK\r\n\r\nuntil the end".to_vec(),
            "/continue" => [b"HTTP/1.1 100 Continue\r\n\r\n".to_vec(), ok(b"after")].concat(),
            "/short" => b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc".to_vec(),
            "/nothing" => b"HTTP/1.1 204 No Content\r\n\r\n".to_vec(),
            "/junk" => b"SSH-2.0-OpenSSH\r\n\r\n".to_vec(),
            _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\ngone".to_vec(),
        }
    });
    let base = format!("http://127.0.0.1:{port}");
    let mut c = client();
    let r = c.get(&format!("{base}/len")).unwrap();
    assert_eq!((r.status, r.text().as_str(), r.length, r.tls, r.peer), (200, "hello", 5, None, [127, 0, 0, 1]));
    assert_eq!(r.header("content-type"), Some("text/plain"));
    assert_eq!(c.get(&format!("{base}/chunked")).unwrap().text(), "hello world");
    assert_eq!(c.get(&format!("{base}/close")).unwrap().text(), "until the end");
    assert_eq!(c.get(&format!("{base}/continue")).unwrap().text(), "after");
    assert_eq!(c.get(&format!("{base}/nothing")).unwrap().status, 204);
    let nf = c.get(&format!("{base}/missing")).unwrap();
    assert_eq!((nf.status, nf.ok(), nf.text().as_str()), (404, false, "gone"));
    assert!(matches!(c.get(&format!("{base}/short")), Err(HttpError::Protocol(m)) if m.contains("3 of 10")));
    assert!(matches!(c.get(&format!("{base}/junk")), Err(HttpError::Protocol(m)) if m.contains("not HTTP")));
    let first = seen.lock().unwrap()[0].clone();
    assert!(first.starts_with("GET /len HTTP/1.1\r\n"), "{first}");
    assert!(first.contains(&format!("Host: 127.0.0.1:{port}\r\n")) && first.contains("User-Agent: test/1\r\n") && first.contains("Connection: close\r\n"));
    // Nothing listens on port 1.
    assert!(matches!(c.get("http://127.0.0.1:1/"), Err(HttpError::Connect(_))));
    assert!(matches!(c.get("ftp://x/"), Err(HttpError::BadUrl(_))));
}

#[test]
fn posts_redirects_and_dropped_credentials() {
    let (other, other_seen) = server(None, |_| ok(b"other host"));
    let (port, seen) = server(None, move |req| {
        let path = req.split(' ').nth(1).unwrap_or("");
        match path {
            "/a" => b"HTTP/1.1 302 Found\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n".to_vec(),
            "/b" => b"HTTP/1.1 307 Temporary Redirect\r\nLocation: c?x=1\r\nContent-Length: 0\r\n\r\n".to_vec(),
            "/c?x=1" => ok(b"landed"),
            "/away" => format!("HTTP/1.1 302 Found\r\nLocation: http://localhost:{other}/there\r\nContent-Length: 0\r\n\r\n").into_bytes(),
            "/form" => b"HTTP/1.1 303 See Other\r\nLocation: /c?x=1\r\nContent-Length: 0\r\n\r\n".to_vec(),
            "/loop" => b"HTTP/1.1 301 Moved\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n".to_vec(),
            "/echo" => ok(req.split("\r\n\r\n").nth(1).unwrap_or("").as_bytes()),
            _ => ok(b"?"),
        }
    });
    let base = format!("http://127.0.0.1:{port}");
    let mut c = client();
    let r = c.get(&format!("{base}/a")).unwrap();
    assert_eq!((r.text().as_str(), r.redirects, r.url.target.as_str()), ("landed", 2, "/c?x=1"));
    assert!(matches!(c.get(&format!("{base}/loop")), Err(HttpError::TooManyRedirects)));

    // A body and a bearer token; the token does not follow a redirect to another host.
    let post = Request::new("post", &format!("{base}/echo")).unwrap().body("application/json", b"{\"a\":1}".to_vec()).bearer("ecm_k_secret");
    assert_eq!(c.send(&post, &mut Transfer::default()).unwrap().text(), "{\"a\":1}");
    let sent = seen.lock().unwrap().last().unwrap().clone();
    assert!(sent.contains("Content-Length: 7\r\n") && sent.contains("Authorization: Bearer ecm_k_secret\r\n") && sent.contains("Content-Type: application/json\r\n"), "{sent}");
    let away = Request::get(&format!("{base}/away")).unwrap().bearer("ecm_k_secret");
    assert_eq!(c.send(&away, &mut Transfer::default()).unwrap().text(), "other host");
    assert!(!other_seen.lock().unwrap()[0].contains("Authorization"), "the token left for another host");
    // 303 after a POST becomes a GET without the body.
    let form = Request::new("POST", &format!("{base}/form")).unwrap().body("text/plain", b"data".to_vec());
    assert_eq!(c.send(&form, &mut Transfer::default()).unwrap().text(), "landed");
    let last = seen.lock().unwrap().last().unwrap().clone();
    assert!(last.starts_with("GET /c?x=1") && !last.contains("Content-Type"), "{last}");
}

#[test]
fn downloads_stream_to_a_sink_with_progress_and_a_limit() {
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let data = big.clone();
    let (port, _) = server(None, move |req| if req.contains(" /big ") { ok(&data) } else { b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\nno".to_vec() });
    let base = format!("http://127.0.0.1:{port}");
    let mut c = client();
    let mut file = Vec::new();
    let mut steps: Vec<(u64, Option<u64>)> = Vec::new();
    {
        let mut progress = |d: u64, t: Option<u64>| steps.push((d, t));
        let mut t = Transfer { sink: Some(&mut file), progress: Some(&mut progress), max_body: 1 << 20 };
        let r = c.send(&Request::get(&format!("{base}/big")).unwrap(), &mut t).unwrap();
        assert_eq!((r.status, r.body.len(), r.length), (200, 0, 200_000));
    }
    assert_eq!(file, big);
    assert_eq!(steps.first(), Some(&(0, Some(200_000))));
    assert_eq!(steps.last(), Some(&(200_000, Some(200_000))));
    assert!(steps.windows(2).all(|w| w[0].0 <= w[1].0));
    // An error page is not written to the file.
    let mut file = Vec::new();
    let mut t = Transfer { sink: Some(&mut file), ..Transfer::default() };
    let r = c.send(&Request::get(&format!("{base}/missing")).unwrap(), &mut t).unwrap();
    assert_eq!((r.status, r.text().as_str()), (404, "no"));
    assert!(file.is_empty());
    // Larger than allowed: refused from the Content-Length, before the body.
    let mut t = Transfer { max_body: 1000, ..Transfer::default() };
    assert_eq!(c.send(&Request::get(&format!("{base}/big")).unwrap(), &mut t), Err(HttpError::TooLarge(1000)));
}

#[test]
fn https_with_an_ingame_authority() {
    let ca = [7u8; 32];
    let key = [3u8; 32];
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let names = [SubjectName::Dns("localhost".into()), SubjectName::Ip([127, 0, 0, 1])];
    let leaf = x509::leaf_certificate(&ca, "Test CA", &x509::public_of(&key), &names, &[1; 16], now - 60, now + 3600);
    let id = Identity::new(vec![leaf], x509::pkcs8(&key)).unwrap();
    let (port, _) = server(Some(id), |req| ok(format!("secure {}", req.split(' ').nth(1).unwrap_or("")).as_bytes()));

    let mut c = client();
    c.trust = Some(Trust::with(vec![Authority::from_key(&x509::public_of(&ca), "Test CA", "test.anchor")]));
    let r = c.get(&format!("https://localhost:{port}/x")).unwrap();
    assert_eq!((r.text().as_str(), r.tls), ("secure /x", Some("TLSv1.3")));
    assert_eq!(c.get(&format!("https://127.0.0.1:{port}/y")).unwrap().text(), "secure /y");

    // No authority trusted on this computer.
    let mut bare = client();
    bare.trust = Some(Trust::public_only());
    assert!(matches!(bare.get(&format!("https://localhost:{port}/")), Err(HttpError::Tls(TlsError::UnknownAuthority(_)))));
    // ... unless checking is off.
    bare.check = Check::None;
    assert_eq!(bare.get(&format!("https://localhost:{port}/k")).unwrap().text(), "secure /k");

    // The same server seen at a public address: the in-game authority does not count there.
    let mut public = Client::new(Native { pretend_peer: Some([203, 0, 113, 7]) });
    public.trust = Some(Trust::with(vec![Authority::from_key(&x509::public_of(&ca), "Test CA", "test.anchor")]));
    assert!(matches!(public.get(&format!("https://localhost:{port}/")), Err(HttpError::Tls(TlsError::UnknownAuthority(_)))));

    // The staged check names the certificate.
    let state = probe::NetState { interfaces: vec![("eth0".into(), true, "10.0.0.2".into())], default_route: true };
    let mut bare = client();
    bare.trust = Some(Trust::public_only());
    assert_eq!(probe::check(&mut bare, &state, &format!("https://localhost:{port}/")).code(), "certificate");
    assert_eq!(probe::check(&mut c, &state, &format!("https://localhost:{port}/")), probe::Online::Yes(200));
    assert_eq!(probe::check(&mut c, &state, "http://127.0.0.1:1/").code(), "unreachable");
}

/// The real internet: a public site's certificate chain is checked against the
/// built-in roots with the pure-Rust provider. Needs a network, so it only
/// runs when asked: `cargo test -p ecm-http -- --ignored public_site`.
#[test]
#[ignore]
fn public_site_over_https() {
    let mut c = client();
    c.trust = Some(Trust::public_only());
    for url in ["https://example.com/", "https://letsencrypt.org/", "https://www.cloudflare.com/"] {
        let r = c.send(&Request::get(url).unwrap(), &mut Transfer { max_body: 4 << 20, ..Transfer::default() }).unwrap_or_else(|e| panic!("{url}: {e}"));
        assert!(r.status < 400, "{url}: HTTP {}", r.status);
        assert!(r.tls.is_some() && ecm_tls::is_public(r.peer), "{url}: {:?} {:?}", r.tls, r.peer);
        println!("{url}: HTTP {} over {} from {:?}, {} bytes", r.status, r.tls.unwrap(), r.peer, r.length);
    }
    // A name the certificate is not for (the address answers, the name is wrong).
    let e = c.get("https://wrong.host.badssl.com/").unwrap_err();
    assert!(matches!(e, HttpError::Tls(TlsError::WrongName(_))), "{e:?}");
    let e = c.get("https://expired.badssl.com/").unwrap_err();
    assert!(matches!(e, HttpError::Tls(TlsError::Expired(_))), "{e:?}");
    let e = c.get("https://self-signed.badssl.com/").unwrap_err();
    assert!(matches!(e, HttpError::Tls(TlsError::UnknownAuthority(_))), "{e:?}");
}
