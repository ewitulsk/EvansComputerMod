//! Positioning, tables, pictures and form controls: read back from the
//! display list and the hit boxes.

use ecm_web::css::Dynamic;
use ecm_web::paint::{DisplayList, Item};
use ecm_web::style::UaColors;
use ecm_web::{Page, Url};

fn render_with(html: &str, w: f32, images: &dyn Fn(&str) -> Option<(f32, f32)>, caret: Option<(usize, usize)>) -> (Page, DisplayList) {
    let p = Page::parse(Url::parse("http://h/dir/index.html").unwrap(), html);
    let r = p.render_with_caret(w, 300.0, &UaColors::default(), &Dynamic::default(), images, caret);
    (p, r.list)
}

fn render(html: &str, w: f32) -> (Page, DisplayList) {
    render_with(html, w, &|_| None, None)
}

fn rect_of(p: &Page, l: &DisplayList, id: &str) -> (f32, f32, f32, f32) {
    let n = p.dom.by_id(id).unwrap_or_else(|| panic!("no #{id}"));
    let h = l.node_rect(n).or_else(|| l.fixed_hits.iter().find(|h| h.node == n).copied()).unwrap_or_else(|| panic!("#{id} not painted"));
    (h.x, h.y, h.w, h.h)
}

fn text_index(l: &DisplayList, t: &str) -> usize {
    l.items.iter().position(|i| matches!(i, Item::Text { text, .. } if text == t)).unwrap_or_else(|| panic!("no {t:?}"))
}

#[test]
fn absolute_inside_a_relative_box() {
    let html = "<body style='margin:0'><div id=box style='position:relative; margin: 20px; width: 200px; height: 100px; padding: 5px'>\
        <span id=badge style='position:absolute; top: 4px; right: 6px; width: 30px; height: 10px'>new</span>in flow</div>\
        <div id=next>after</div><div id=bottom style='position:absolute; left: 10px; bottom: 0; width: 50px; height: 20px'></div></body>";
    let (p, l) = render(html, 400.0);
    let b = rect_of(&p, &l, "box");
    let badge = rect_of(&p, &l, "badge");
    // Against the padding box: right edge 6 in from the box's right padding edge.
    assert_eq!(badge, (b.0 + b.2 - 6.0 - 30.0, b.1 + 4.0, 30.0, 10.0));
    // Out of the flow: the next block follows the box directly.
    assert_eq!(rect_of(&p, &l, "next").1, b.1 + b.3 + 20.0);
    // No positioned ancestor: the viewport (300 high).
    assert_eq!(rect_of(&p, &l, "bottom"), (10.0, 280.0, 50.0, 20.0));
}

#[test]
fn absolute_over_the_content_and_z_order() {
    let html = "<div style='position:relative'><div id=a style='position:absolute; z-index: 2'>top</div>\
        <div style='position:absolute; z-index: 1'>middle</div>flow</div>";
    let (_, l) = render(html, 300.0);
    assert!(text_index(&l, "flow") < text_index(&l, "middle"), "positioned boxes paint over the flow");
    assert!(text_index(&l, "middle") < text_index(&l, "top"), "by z-index");
}

#[test]
fn fixed_stays_in_the_viewport_layer() {
    let mut html = String::from("<div id=bar style='position:fixed; top:0; left:0; right:0; height: 20px; background: #102030'>menu</div>");
    for i in 0..50 {
        html.push_str(&format!("<p>p{i}</p>"));
    }
    let (p, l) = render(&html, 300.0);
    assert!(l.fixed.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "menu")));
    assert!(!l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "menu")));
    let bar = rect_of(&p, &l, "bar");
    assert_eq!(bar, (0.0, 0.0, 300.0, 20.0), "left + right: as wide as the viewport");
    // A point at the top of the view is the bar, however far down the page is scrolled.
    let n = p.dom.by_id("bar").unwrap();
    assert_eq!(l.hit_test_view(50.0, 5.0, 400.0), Some(n));
    assert_ne!(l.hit_test_view(50.0, 50.0, 400.0), Some(n));
}

#[test]
fn table_columns_from_their_cells() {
    let html = "<body style='margin:0'><table id=t style='border-spacing: 4px'>\
        <tr><th id=h1>Name</th><th id=h2>Description of the item</th></tr>\
        <tr><td id=c1>Hopper</td><td id=c2>Moves items</td></tr>\
        <tr><td id=c3>Comparator, long name</td><td id=c4>x</td></tr></table></body>";
    let (p, l) = render(html, 600.0);
    let (h1, h2, c1, c2, c3) = (rect_of(&p, &l, "h1"), rect_of(&p, &l, "h2"), rect_of(&p, &l, "c1"), rect_of(&p, &l, "c2"), rect_of(&p, &l, "c3"));
    // Columns line up, separated by the spacing.
    assert_eq!((h1.0, h1.2), (c1.0, c1.2));
    assert_eq!((h2.0, h2.2), (c2.0, c2.2));
    assert_eq!(h1.0, 4.0);
    assert_eq!(h2.0, h1.0 + h1.2 + 4.0);
    // The first column is as wide as its widest cell's text (plus padding).
    let face = ecm_ui_proto::fonts::face(ecm_ui_proto::Font::Ui, 12).unwrap();
    assert!(c3.2 >= face.text_width("Comparator, long name") as f32 + 8.0);
    // Rows stack; cells of a row are equally tall.
    assert_eq!(c1.1, h1.1 + h1.3 + 4.0);
    assert_eq!(c1.3, c2.3);
    // An auto-width table is no wider than its columns.
    assert!(rect_of(&p, &l, "t").2 < 600.0);
    // Narrow: columns shrink to their longest word (text wraps); never below it, as in CSS.
    let (p, l) = render(html, 120.0);
    let narrow = rect_of(&p, &l, "t").2;
    assert!(narrow < rect_of(&p, &render(html, 600.0).1, "t").2 && narrow >= face.text_width("Description") as f32, "{narrow}");
}

#[test]
fn table_fills_a_given_width_and_aligns_vertically() {
    let html = "<body style='margin:0'><table style='width: 300px; border-spacing: 0'><tr>\
        <td id=a style='padding:0'>short</td><td id=b style='padding:0'>one<br>two<br>three</td></tr></table></body>";
    let (p, l) = render(html, 600.0);
    let (a, b) = (rect_of(&p, &l, "a"), rect_of(&p, &l, "b"));
    assert_eq!(a.2 + b.2, 300.0);
    assert_eq!(a.3, b.3);
    // Cells are middle-aligned by default: "short" sits halfway down.
    let short_y = l.items.iter().find_map(|i| match i {
        Item::Text { y, text, .. } if text == "short" => Some(*y as f32),
        _ => None,
    });
    assert!(short_y.unwrap() > a.1 + 5.0, "{short_y:?} vs {a:?}");
}

#[test]
fn pictures_and_backgrounds() {
    let images = |u: &str| match u {
        "http://h/dir/cat.png" => Some((64.0, 32.0)),
        "http://h/bg.png" => Some((16.0, 16.0)),
        _ => None,
    };
    let html = "<body style='margin:0'><img id=i src=cat.png><img id=s src=cat.png width=32><img id=m src=missing.png alt='a missing picture'>\
        <div id=d style='background-image: url(/bg.png); width: 100px; height: 50px'>x</div></body>";
    let p = Page::parse(Url::parse("http://h/dir/index.html").unwrap(), html);
    let r = p.render(400.0, 300.0, &UaColors::default(), &Dynamic::default(), &images);
    assert_eq!(r.images, ["http://h/dir/cat.png", "http://h/dir/missing.png", "http://h/bg.png"]);
    let l = &r.list;
    let imgs: Vec<(i32, i32, i32, i32, String)> = l
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Image { x, y, w, h, src } => Some((*x, *y, *w, *h, src.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(imgs[0].2, 64);
    assert_eq!((imgs[1].2, imgs[1].3), (32, 16), "width only: the height keeps the shape");
    assert!(imgs.iter().any(|i| i.4 == "http://h/bg.png" && i.2 == 16));
    // The missing one shows its alt text instead.
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "a missing picture")));
}

#[test]
fn a_focused_field_has_a_caret_and_controls_show_their_state() {
    let html = "<input id=t name=n value='hello world' size=8><input id=p type=password value=abc><input id=c type=checkbox checked>\
        <select id=s><option>One<option selected>Two</select><button>Go</button>";
    let p0 = Page::parse(Url::parse("http://h/").unwrap(), html);
    let t = p0.dom.by_id("t").unwrap();
    let (p, l) = render_with(html, 400.0, &|_| None, Some((t, 11)));
    let field = rect_of(&p, &l, "t");
    // A 1 lu caret inside the field, near its right edge (the text moved left).
    let caret = l.items.iter().find_map(|i| match i {
        Item::Rect { x, y, w: 1, h, .. } if (*x as f32) > field.0 && (*x as f32) < field.0 + field.2 && *h >= 8 => Some((*x, *y)),
        _ => None,
    });
    assert!(caret.is_some(), "no caret in {field:?}");
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "***")), "password shown as stars");
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "Two")));
    assert!(l.items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "Go")));
    // No caret without focus.
    let (_, l2) = render(html, 400.0);
    assert!(!l2.items.iter().any(|i| matches!(i, Item::Rect { w: 1, .. })));
}
