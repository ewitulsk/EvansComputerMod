//! Layout and paint: what a page looks like at a width, read back from the
//! display list and the hit boxes.

use ecm_web::css::Dynamic;
use ecm_web::paint::{DisplayList, Item};
use ecm_web::style::UaColors;
use ecm_web::{Page, Url};

fn render_sheets(html: &str, css: Option<&str>, w: f32) -> (Page, DisplayList) {
    let mut p = Page::parse(Url::parse("http://h/index.html").unwrap(), html);
    let css = css.map(str::to_string);
    p.load_sheets(&mut |u| if u.path == "/style.css" { css.clone() } else { None });
    let r = p.render(w, 300.0, &UaColors::default(), &Dynamic::default(), &|_| None);
    (p, r.list)
}

fn render(html: &str, w: f32) -> (Page, DisplayList) {
    render_sheets(html, None, w)
}

/// (x, y, text, size) of every text item (bold copies dropped).
fn texts(l: &DisplayList) -> Vec<(i32, i32, String, u8)> {
    let mut out: Vec<(i32, i32, String, u8)> = Vec::new();
    for i in &l.items {
        if let Item::Text { x, y, text, size, .. } = i {
            if out.last().is_some_and(|(lx, ly, lt, _)| *lx + 1 == *x && ly == y && lt == text) {
                continue;
            }
            out.push((*x, *y, text.clone(), *size));
        }
    }
    out
}

fn text_at(l: &DisplayList, t: &str) -> (i32, i32, u8) {
    texts(l).into_iter().find(|(_, _, s, _)| s.contains(t)).map(|(x, y, _, s)| (x, y, s)).unwrap_or_else(|| panic!("no text {t:?} in {:?}", texts(l)))
}

/// The border box of the first element with this id.
fn rect_of(p: &Page, l: &DisplayList, id: &str) -> (f32, f32, f32, f32) {
    let n = p.dom.by_id(id).unwrap_or_else(|| panic!("no #{id}"));
    let h = l.node_rect(n).unwrap_or_else(|| panic!("#{id} not painted"));
    (h.x, h.y, h.w, h.h)
}

const VILLAGE: &str = "<!DOCTYPE html>\n<html><head><title>Tech Village 4 - AS 65004</title></head>\n<body>\n<h1>Tech Village 4</h1>\n\
<p>A plains village on the Tech Village fiber ring. AS 65004, served from the village data center.</p>\n<h2>Network</h2>\n<ul>\n\
<li>Village cable (ISP eth0): 100.68.1.0/24, gateway 100.68.1.1, DHCP .10-.200: plug any computer in and use 'iface ethN dhcp'</li>\n\
</ul>\n<h2>Chat</h2>\n<p>Run <code>chat</code> (village computers read /etc/chat.conf).</p>\n<h2>Other villages</h2>\n<ul>\n\
<li><a href=\"http://100.65.0.10/\">Tech Village 1</a> (desert, AS 65001)</li>\n<li><a href=\"http://100.66.0.10/\">Tech Village 2</a> (taiga, AS 65002)</li>\n</ul>\n</body></html>\n";

#[test]
fn village_page() {
    let (p, l) = render(VILLAGE, 400.0);
    assert_eq!(p.title, "Tech Village 4 - AS 65004");
    let (hx, hy, hs) = text_at(&l, "Tech Village 4");
    assert_eq!((hx, hs), (8, 32), "h1: body margin, 32 lu");
    assert!(hy >= 8);
    let (_, ny, ns) = text_at(&l, "Network");
    assert_eq!(ns, 24);
    assert!(ny > hy + 32);
    // Every line fits the 400 lu viewport.
    for (x, _, t, s) in texts(&l) {
        let face = ecm_ui_proto::fonts::face(ecm_ui_proto::Font::Ui, s).unwrap();
        assert!(x + face.text_width(&t) <= 400 - 8, "{t:?} at {x} overflows");
    }
    // The long list item wrapped onto several lines, all indented past the bullet.
    let li_lines: Vec<_> = texts(&l).into_iter().filter(|(_, y, _, _)| *y > ny && *y < text_at(&l, "Chat").1).collect();
    assert!(li_lines.len() >= 2, "{li_lines:?}");
    assert!(li_lines.iter().all(|(x, _, _, _)| *x == 8 + 24));
    // Bullets: filled round rects left of the items.
    let bullets = l.items.iter().filter(|i| matches!(i, Item::Rect { x, radius, .. } if *x < 32 && *radius > 0)).count();
    assert_eq!(bullets, 3);
    // Links are hit-testable and resolve.
    let (lx, ly, _) = text_at(&l, "Tech Village 1");
    let node = l.hit_test(lx as f32 + 5.0, ly as f32 + 5.0).unwrap();
    assert_eq!(p.link_at(node).unwrap().to_string(), "http://100.65.0.10/");
    // Off the link: no link.
    let n2 = l.hit_test(300.0, ly as f32 + 5.0);
    assert!(n2.and_then(|n| p.link_at(n)).is_none());
    assert_eq!(p.links().len(), 2);
    // Underlined links: a rect under the text.
    assert!(l.items.iter().any(|i| matches!(i, Item::Rect { x, y, h: 1, .. } if *x == lx && *y > ly && *y < ly + 14)));
}

#[test]
fn narrow_wraps_more() {
    let (_, wide) = render(VILLAGE, 600.0);
    let (_, narrow) = render(VILLAGE, 200.0);
    assert!(texts(&narrow).len() > texts(&wide).len());
    assert!(narrow.height > wide.height);
}

#[test]
fn block_boxes() {
    let html = "<div id=a style='width: 200px; margin: 0 auto; padding: 10px; border: 2px solid red'>x</div>\
                <div id=b style='width: 200px; padding: 10px; box-sizing: border-box; margin-left: 20px'>y</div>\
                <div id=c style='max-width: 100px; margin-top: 30px'>z</div>\
                <div id=d style='margin-top: 10px; width: 50%'>w</div>";
    let (p, l) = render(&format!("<body style='margin:0'>{html}</body>"), 400.0);
    let a = rect_of(&p, &l, "a");
    assert_eq!(a, (88.0, 0.0, 224.0, 12.0 * 1.25 + 24.0), "centred: (400 - 224) / 2");
    let b = rect_of(&p, &l, "b");
    assert_eq!((b.0, b.2), (20.0, 200.0), "border-box width");
    let c = rect_of(&p, &l, "c");
    assert_eq!(c.2, 100.0);
    assert_eq!(c.1, b.1 + b.3 + 30.0);
    let d = rect_of(&p, &l, "d");
    assert_eq!(d.1, c.1 + c.3 + 10.0, "margins collapse: max(0, 10)");
    assert_eq!(d.2, 200.0);
    // The border is drawn on the box.
    assert!(l.items.iter().any(|i| matches!(i, Item::Rect { border_w: 2, border_color: 0xff0000ff, .. })));
}

#[test]
fn flex_row() {
    let html = "<body style='margin:0'><div id=f style='display:flex; gap: 10px; width: 330px'>\
                <div id=a style='flex: 1'>a</div><div id=b style='flex: 2'>b</div><div id=c style='width: 50px'>c</div></div></body>";
    let (p, l) = render(html, 400.0);
    let (a, b, c) = (rect_of(&p, &l, "a"), rect_of(&p, &l, "b"), rect_of(&p, &l, "c"));
    assert_eq!(c.2, 50.0);
    // 330 - 50 - 20 gap = 260 split 1:2.
    assert!((a.2 - 86.67).abs() < 0.1 && (b.2 - 173.33).abs() < 0.1, "{a:?} {b:?}");
    assert!((b.0 - (a.0 + a.2 + 10.0)).abs() < 0.01 && (c.0 - (b.0 + b.2 + 10.0)).abs() < 0.01);
    assert_eq!(a.1, b.1);
}

#[test]
fn flex_justify_align_and_auto_margins() {
    let html = "<body style='margin:0'>\
        <div id=row style='display:flex; justify-content: space-between; align-items: center; height: 100px'>\
          <span id=a style='width: 40px'>left</span><span id=b style='font-size: 32px'>BIG</span><span id=c style='width: 40px'>right</span></div>\
        <nav style='display:flex'><a id=l href=/>Home</a><a id=r href=/x style='margin-left:auto'>Login</a></nav>\
        <div style='display:flex; justify-content:center'><div id=m style='width: 100px'>m</div></div></body>";
    let (p, l) = render(html, 400.0);
    let (a, b, c) = (rect_of(&p, &l, "a"), rect_of(&p, &l, "b"), rect_of(&p, &l, "c"));
    assert_eq!(a.0, 0.0);
    assert!((c.0 + c.2 - 400.0).abs() < 0.01, "right edge {c:?}");
    assert!(((b.0 + b.2 / 2.0) - 200.0).abs() < 1.0, "middle {b:?}");
    // Centred in the 100 lu line.
    assert!(((a.1 + a.3 / 2.0) - 50.0).abs() < 1.0 && ((b.1 + b.3 / 2.0) - 50.0).abs() < 1.0, "{a:?} {b:?}");
    let r = rect_of(&p, &l, "r");
    assert!((r.0 + r.2 - 400.0).abs() < 0.01, "margin-left: auto pushes Login right: {r:?}");
    assert_eq!(rect_of(&p, &l, "l").0, 0.0);
    assert_eq!(rect_of(&p, &l, "m").0, 150.0);
}

#[test]
fn flex_wrap_and_column() {
    let html = "<body style='margin:0'>\
        <div style='display:flex; flex-wrap: wrap; width: 250px; row-gap: 4px'>\
          <div id=a style='width:100px'>a</div><div id=b style='width:100px'>b</div><div id=c style='width:100px'>c</div></div>\
        <div style='display:flex; flex-direction: column; align-items: flex-start'>\
          <div id=x>first</div><div id=y style='margin-top: 6px'>second, longer</div></div>\
        <div style='display:flex; flex-direction: row-reverse; width: 200px'><div id=p1>1</div><div id=p2>2</div></div></body>";
    let (p, l) = render(html, 400.0);
    let (a, b, c) = (rect_of(&p, &l, "a"), rect_of(&p, &l, "b"), rect_of(&p, &l, "c"));
    assert_eq!((a.1, b.1), (0.0, 0.0));
    assert_eq!(c.0, 0.0, "the third wraps");
    assert_eq!(c.1, a.3 + 4.0);
    let (x, y) = (rect_of(&p, &l, "x"), rect_of(&p, &l, "y"));
    assert_eq!(y.1, x.1 + x.3 + 6.0);
    assert!(x.2 < y.2, "align-items: flex-start: items are their own width");
    let (p1, p2) = (rect_of(&p, &l, "p1"), rect_of(&p, &l, "p2"));
    assert!(p1.0 > p2.0 && (p1.0 + p1.2 - 200.0).abs() < 0.01, "row-reverse {p1:?} {p2:?}");
}

#[test]
fn flex_items_keep_their_longest_word() {
    let html = "<body style='margin:0'><div style='display:flex; width: 100px'>\
        <div id=a>Supercalifragilistic</div><div id=b>x</div></div></body>";
    let (p, l) = render(html, 400.0);
    let a = rect_of(&p, &l, "a");
    let face = ecm_ui_proto::fonts::face(ecm_ui_proto::Font::Ui, 12).unwrap();
    assert!(a.2 >= face.text_width("Supercalifragilistic") as f32);
}

#[test]
fn media_query_changes_layout() {
    let css = ".cols { display: flex } .cols > div { flex: 1 } @media (max-width: 480px) { .cols { display: block } nav { display: none } }";
    let html = "<head><link rel=stylesheet href=style.css></head><body><nav>Menu</nav><div class=cols><div id=a>A</div><div id=b>B</div></div></body>";
    let (p, wide) = render_sheets(html, Some(css), 800.0);
    let (a, b) = (rect_of(&p, &wide, "a"), rect_of(&p, &wide, "b"));
    assert_eq!(a.1, b.1, "side by side on a desktop");
    assert!(texts(&wide).iter().any(|t| t.2 == "Menu"));
    let (p, phone) = render_sheets(html, Some(css), 300.0);
    let (a, b) = (rect_of(&p, &phone, "a"), rect_of(&p, &phone, "b"));
    assert!(b.1 > a.1, "stacked on a phone");
    assert!(!texts(&phone).iter().any(|t| t.2 == "Menu"));
}

#[test]
fn missing_stylesheet_renders_unstyled() {
    let html = "<head><link rel=stylesheet href=gone.css></head><body><h1 class=x>Hello</h1></body>";
    let (_, l) = render_sheets(html, Some("h1 { color: red }"), 400.0);
    let ua = UaColors::default();
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, color, size: 32, .. } if text == "Hello" && *color == ua.text)));
    assert_eq!(l.background, ua.background);
}

#[test]
fn page_colours_and_text_styles() {
    let html = "<body style='background:#ffffff; color:#000; margin:0'><p style='text-align:center'>mid</p>\
        <p>a <b>bold</b> <span style='background:yellow; padding: 0 4px'>mark</span> <s>gone</s></p>\
        <pre>  two  spaces\nnext</pre><p style='text-transform: uppercase'>shout</p></body>";
    let (_, l) = render(html, 300.0);
    assert_eq!(l.background, 0xffffffff);
    let (mx, _, _) = text_at(&l, "mid");
    let face = ecm_ui_proto::fonts::face(ecm_ui_proto::Font::Ui, 12).unwrap();
    assert_eq!(mx, ((300 - face.text_width("mid")) as f32 / 2.0).round() as i32);
    // Bold: drawn twice, one lu apart.
    let bolds: Vec<_> = texts(&l).into_iter().filter(|t| t.2 == "bold").collect();
    assert_eq!(bolds.len(), 1);
    assert_eq!(l.items.iter().filter(|i| matches!(i, Item::Text { text, .. } if text == "bold")).count(), 2);
    // The highlighted span's background sits behind it.
    let (kx, ky, _) = text_at(&l, "mark");
    assert!(l.items.iter().any(|i| matches!(i, Item::Rect { x, y, fill: 0xffff00ff, .. } if *x == kx - 4 && *y <= ky)));
    // Preformatted text keeps its spaces, in the mono font.
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, font: ecm_ui_proto::Font::Mono, .. } if text == "  two  spaces")));
    assert!(texts(&l).iter().any(|t| t.2 == "next"));
    assert!(texts(&l).iter().any(|t| t.2 == "SHOUT"));
}

#[test]
fn ordered_lists_number() {
    let (_, l) = render("<ol start=3><li>a<li>b</ol><ol type=a style='list-style-type: lower-roman'><li>x<li>y<li>z<li>w</ol>", 300.0);
    let t: Vec<String> = texts(&l).into_iter().map(|t| t.2).collect();
    for want in ["3.", "4.", "i.", "iv."] {
        assert!(t.contains(&want.to_string()), "{want} in {t:?}");
    }
}

#[test]
fn hover_restyles() {
    let html = "<a id=l href=/x>link</a>";
    let mut p = Page::parse(Url::parse("http://h/").unwrap(), html);
    p.sheets.push(ecm_web::css::Stylesheet::parse("a:hover { color: #ff0000 }"));
    let r = p.render(300.0, 200.0, &UaColors::default(), &Dynamic::default(), &|_| None);
    assert!(r.dynamic_selectors);
    let a = p.dom.by_id("l").unwrap();
    let dy = Dynamic { hover: vec![a], ..Default::default() };
    let r = p.render(300.0, 200.0, &UaColors::default(), &dy, &|_| None);
    assert!(r.list.items.iter().any(|i| matches!(i, Item::Text { color: 0xff0000ff, .. })));
}

#[test]
fn junk_never_panics() {
    let junk = [
        "",
        "<",
        "<<<>>>",
        "<div style='display:flex;flex-wrap:wrap;width:-5px'><span style='font-size:-3px;margin:-100px'>x</span></div>",
        "<table><tr><td><table><tr><td>deep</table></table>",
        "<p style='width:calc(50% - 1000px);padding:50%'>x</p>",
        "<ul><li><ul><li><ul><li>x</ul></ul></ul><ol start=-4><li>neg</ol>",
        "<div style='display:inline-flex;flex-direction:column'><img alt='no image'><input type=checkbox checked><select><option>1</select><textarea>t\nu</textarea><button>b</button></div>",
        "<style>@media{}}}{{{a{color:}</style><p>after</p>",
        "<pre>\t\ttabs\r\n</pre><p>\u{2014}\u{201c}quotes\u{201d}\u{2026} \u{1f600}</p>",
    ];
    for j in junk {
        for w in [0.0, 1.0, 50.0, 400.0] {
            let (_, l) = render(j, w);
            assert!(l.height >= 0.0);
        }
    }
    // A thousand nested divs.
    let deep = "<div style='padding-left:1px'>".repeat(1000);
    let (_, l) = render(&deep, 300.0);
    assert!(l.height >= 0.0);
}
