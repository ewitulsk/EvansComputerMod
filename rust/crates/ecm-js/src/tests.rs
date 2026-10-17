//! Page scripts on a real QuickJS against ecm-web documents.

use super::*;

fn page(html: &str) -> Option<Page> {
    Some(Page::parse(Url::parse("http://h/dir/index.html").unwrap(), html))
}

fn script() -> Script {
    Script::new().expect("runtime")
}

fn eval(s: &mut Script, p: &mut Option<Page>, src: &str) -> String {
    s.eval(p, src).unwrap_or_else(|| panic!("{src} failed: {:?}", s.host.borrow().console))
}

fn text_of(p: &Option<Page>, id: &str) -> String {
    let d = &p.as_ref().unwrap().dom;
    d.text_content(d.by_id(id).unwrap())
}

#[test]
fn dom_queries_and_changes() {
    let mut p = page("<ul id=l><li class='a b'>one</li><li>two</li></ul><p id=out>x</p><input id=i value=v>");
    let mut s = script();
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('l').children.length"), "2");
    assert_eq!(eval(&mut s, &mut p, "document.querySelector('li.b').textContent"), "one");
    assert_eq!(eval(&mut s, &mut p, "document.querySelectorAll('#l > li').length"), "2");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('nope')"), "null");
    eval(&mut s, &mut p, "var li = document.createElement('li'); li.textContent = 'three'; li.className = 'new'; document.getElementById('l').appendChild(li); 1");
    assert_eq!(text_of(&p, "l"), "onetwothree");
    assert!(s.take_dirty());
    eval(&mut s, &mut p, "document.getElementById('out').innerHTML = '<b id=bb>bold</b> text'; 1");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('bb').tagName"), "B");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('out').innerHTML"), "<b id=\"bb\">bold</b> text");
    eval(&mut s, &mut p, "var x = document.querySelector('li'); x.classList.remove('a'); x.classList.add('c'); x.classList.toggle('b'); 1");
    assert_eq!(eval(&mut s, &mut p, "document.querySelector('li').className"), "c");
    eval(&mut s, &mut p, "var o = document.getElementById('out'); o.style.backgroundColor = 'red'; o.style.fontSize = '16px'; 1");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('out').getAttribute('style')"), "background-color: red; font-size: 16px");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('out').style.fontSize"), "16px");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('i').value"), "v");
    eval(&mut s, &mut p, "document.getElementById('i').value = 'typed'; 1");
    let d = &p.as_ref().unwrap().dom;
    assert_eq!(forms::value(d, d.by_id("i").unwrap()), "typed");
    eval(&mut s, &mut p, "document.querySelector('li').remove(); document.title = 'New title'; 1");
    assert_eq!(text_of(&p, "l"), "twothree");
    assert_eq!(p.as_ref().unwrap().title, "New title");
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('l').firstElementChild.nextElementSibling.textContent"), "three");
    // The same element is the same object.
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('l') === document.querySelector('ul')"), "true");
}

#[test]
fn page_scripts_and_load_events() {
    let mut p = page("<p id=a>-</p><p id=b>-</p>");
    let mut s = script();
    s.run_scripts(
        &mut p,
        &[
            ("inline".into(), "var n = 0; document.addEventListener('DOMContentLoaded', function () { n++; document.getElementById('a').textContent = 'ready ' + n; });".into()),
            ("second".into(), "window.addEventListener('load', function () { document.getElementById('b').textContent = typeof n; }); undefined_function();".into()),
        ],
    );
    assert_eq!(text_of(&p, "a"), "ready 1");
    assert_eq!(text_of(&p, "b"), "number", "scripts share the global scope; an error in one doesn't stop the rest");
    assert!(s.host.borrow().console.iter().any(|l| l.contains("undefined_function")), "{:?}", s.host.borrow().console);
}

#[test]
fn clicks_bubble_and_prevent_default() {
    let mut p = page("<div id=box><a id=link href=/x><span id=inner>go</span></a></div><button id=btn>+1</button><p id=count>0</p>");
    let mut s = script();
    s.run_scripts(
        &mut p,
        &[(
            "s".into(),
            "var seen = []; document.getElementById('box').addEventListener('click', function (e) { seen.push('box:' + e.target.id); });\
             document.getElementById('link').addEventListener('click', function (e) { seen.push('link'); e.preventDefault(); });\
             var c = 0; document.getElementById('btn').onclick = function () { c++; document.getElementById('count').textContent = String(c); };"
                .into(),
        )],
    );
    let d = &p.as_ref().unwrap().dom;
    let inner = d.by_id("inner").unwrap();
    let btn = d.by_id("btn").unwrap();
    assert!(s.dispatch(&mut p, inner, "click", ""), "the link's listener prevented the default");
    assert_eq!(eval(&mut s, &mut p, "seen.join(',')"), "link,box:inner");
    assert!(!s.dispatch(&mut p, btn, "click", ""));
    assert!(!s.dispatch(&mut p, btn, "click", ""));
    assert_eq!(text_of(&p, "count"), "2");
}

#[test]
fn timers_and_animation_frames() {
    let mut p = page("<p id=t>0</p>");
    let mut s = script();
    s.run_scripts(
        &mut p,
        &[("t".into(), "var ticks = 0; var id = setInterval(function () { ticks++; document.getElementById('t').textContent = ticks; if (ticks == 3) clearInterval(id); }, 10); setTimeout(function (a) { window.once = a; }, 0, 'arg'); requestAnimationFrame(function (t) { window.frame = typeof t; });".into())],
    );
    assert!(s.pending());
    let start = Instant::now();
    while s.pending() && start.elapsed() < Duration::from_secs(2) {
        s.tick(&mut p);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(text_of(&p, "t"), "3");
    assert_eq!(eval(&mut s, &mut p, "window.once + ' ' + window.frame"), "arg number");
    assert!(!s.pending(), "the interval was cleared");
    assert_eq!(s.tick(&mut p), None);
}

#[test]
fn a_busy_script_is_stopped_and_the_page_keeps_working() {
    let mut p = page("<button id=b>x</button><p id=o>ok</p>");
    let mut s = script();
    s.run_scripts(&mut p, &[("loop".into(), "document.getElementById('b').addEventListener('click', function () { while (true) {} });".into())]);
    let b = p.as_ref().unwrap().dom.by_id("b").unwrap();
    let t = Instant::now();
    s.dispatch(&mut p, b, "click", "");
    let took = t.elapsed();
    assert!(took < Duration::from_secs(2), "stopped by the budget ({took:?})");
    assert!(took >= BUDGET, "ran its budget first ({took:?})");
    assert_eq!(s.stopped, 1);
    assert!(s.host.borrow().console.iter().any(|l| l.contains("ran too long")));
    // The context still works afterwards.
    assert_eq!(eval(&mut s, &mut p, "document.getElementById('o').textContent"), "ok");
    // A busy timer too.
    eval(&mut s, &mut p, "setTimeout(function () { for (;;) {} }, 0); 1");
    s.tick(&mut p);
    assert_eq!(s.stopped, 2);
}

#[test]
fn fetch_storage_navigation_and_dialogs() {
    let mut p = page("<p id=o></p>");
    let mut s = script();
    let asked = Rc::new(RefCell::new(Vec::new()));
    {
        let mut h = s.host.borrow_mut();
        h.fetch = Some(Box::new(|method, url, body| {
            if url.path == "/data.json" {
                Ok(Fetched { status: 200, body: format!("{{\"m\":\"{method}\",\"body\":\"{}\",\"n\":3}}", body.unwrap_or("")), content_type: "application/json".into(), url: url.to_string() })
            } else {
                Err("nobody answered".into())
            }
        }));
        let a = asked.clone();
        h.modal = Some(Box::new(move |text, ask| {
            a.borrow_mut().push((text.to_string(), ask));
            !text.contains("No")
        }));
    }
    eval(
        &mut s,
        &mut p,
        "fetch('../data.json', {method: 'post', body: 'x=1'}).then(function (r) { return r.json(); }).then(function (j) { document.getElementById('o').textContent = j.m + ' ' + j.body + ' ' + j.n; }); 1",
    );
    assert_eq!(text_of(&p, "o"), "POST x=1 3", "relative to the page, the promise chain ran");
    eval(&mut s, &mut p, "fetch('/gone').catch(function (e) { window.failed = String(e); }); 1");
    assert!(eval(&mut s, &mut p, "window.failed").contains("nobody answered"));
    eval(&mut s, &mut p, "localStorage.setItem('visits', '1'); localStorage.setItem('a', 'b'); 1");
    assert_eq!(eval(&mut s, &mut p, "localStorage.getItem('visits') + localStorage.length + localStorage.getItem('none')"), "12null");
    assert_eq!(s.host.borrow().storage.get("visits").map(String::as_str), Some("1"));
    assert!(s.host.borrow().storage_dirty);
    assert_eq!(eval(&mut s, &mut p, "confirm('Delete?') + ' ' + confirm('No way')"), "true false");
    eval(&mut s, &mut p, "alert('Saved'); 1");
    assert_eq!(asked.borrow().last().unwrap(), &("Saved".to_string(), false));
    assert_eq!(eval(&mut s, &mut p, "location.href + ' ' + location.pathname + ' ' + location.host"), "http://h/dir/index.html /dir/index.html h");
    eval(&mut s, &mut p, "location.href = 'other.html'; 1");
    assert_eq!(s.host.borrow().navigate, Some(Nav::Go("http://h/dir/other.html".into())));
    eval(&mut s, &mut p, "history.back(); 1");
    assert_eq!(s.host.borrow().navigate, Some(Nav::Back));
    eval(&mut s, &mut p, "console.log('hi', {a: 1}, [2]); console.error('bad')");
    let c = s.host.borrow().console.clone();
    assert!(c.contains(&"hi {\"a\":1} [2]".to_string()) && c.contains(&"[error] bad".to_string()), "{c:?}");
}

#[test]
fn keys_reach_listeners() {
    let mut p = page("<input id=i><p id=o></p>");
    let mut s = script();
    s.run_scripts(&mut p, &[("k".into(), "document.addEventListener('keydown', function (e) { document.getElementById('o').textContent += e.key; if (e.key === 'Enter') e.preventDefault(); });".into())]);
    let i = p.as_ref().unwrap().dom.by_id("i").unwrap();
    assert!(!s.dispatch(&mut p, i, "keydown", "a"));
    assert!(s.dispatch(&mut p, i, "keydown", "Enter"));
    assert_eq!(text_of(&p, "o"), "aEnter");
}
