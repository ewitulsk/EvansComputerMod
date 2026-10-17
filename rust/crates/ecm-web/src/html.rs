//! A forgiving HTML parser: a tokenizer and a small tree builder.
//!
//! It never fails. Unknown tags become ordinary elements (their text shows),
//! stray end tags are ignored, unclosed elements close at the end, and the
//! usual implied ends are applied: a block start closes an open `<p>`, a new
//! `<li>` closes the previous one, likewise `<option>`, `<tr>`, `<td>`/`<th>`
//! and `<dt>`/`<dd>`. `<script>`, `<style>`, `<title>` and `<textarea>` hold
//! raw text up to their end tag.

use crate::dom::{Dom, Element, NodeData, NodeId, DOCUMENT};

/// Elements that never have children.
pub const VOID: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Elements whose start closes an open `<p>`.
const CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption", "figure", "footer", "form",
    "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "main", "nav", "ol", "p", "pre", "section", "table", "ul",
];

/// Elements nest at most this deep; deeper start tags become siblings (no
/// page can overflow the stack of the layout that follows).
pub const MAX_DEPTH: usize = 96;

/// Raw text elements: everything up to the end tag is text.
const RAW: &[&str] = &["script", "style", "title", "textarea", "xmp"];

/// Elements that stop the search for an element to close implicitly.
const SCOPE: &[&str] = &["html", "body", "table", "td", "th", "button", "ul", "ol", "select"];

#[derive(Debug, PartialEq)]
enum Token {
    Start { tag: String, attrs: Vec<(String, String)>, self_closing: bool },
    End(String),
    Text(String),
}

/// Parse a document.
pub fn parse(src: &str) -> Dom {
    let mut dom = Dom::new();
    let mut stack: Vec<NodeId> = vec![DOCUMENT];
    for tok in tokenize(src) {
        match tok {
            Token::Text(t) => {
                if t.is_empty() {
                    continue;
                }
                let cur = *stack.last().unwrap();
                // Merge with a text node just before.
                if let Some(&last) = dom.nodes[cur].children.last() {
                    if let NodeData::Text(prev) = &mut dom.nodes[last].data {
                        prev.push_str(&t);
                        continue;
                    }
                }
                dom.push(cur, NodeData::Text(t));
            }
            Token::Start { tag, attrs, self_closing } => {
                implied_ends(&dom, &mut stack, &tag);
                // A second <html>/<body>: merge its attributes, no new element.
                if (tag == "html" || tag == "body") && dom.find(&tag).is_some() {
                    continue;
                }
                let cur = *stack.last().unwrap();
                let id = dom.push(cur, NodeData::Element(Element { tag: tag.clone(), attrs }));
                if !self_closing && !VOID.contains(&tag.as_str()) && stack.len() < MAX_DEPTH {
                    stack.push(id);
                }
            }
            Token::End(tag) => {
                if let Some(pos) = stack.iter().rposition(|n| dom.tag(*n) == Some(tag.as_str())) {
                    if pos > 0 {
                        stack.truncate(pos);
                    }
                }
            }
        }
    }
    dom
}

/// Close what the start of `tag` closes.
fn implied_ends(dom: &Dom, stack: &mut Vec<NodeId>, tag: &str) {
    let close_in_scope = |stack: &mut Vec<NodeId>, targets: &[&str], scope: &[&str]| {
        for i in (1..stack.len()).rev() {
            let t = dom.tag(stack[i]).unwrap_or("");
            if targets.contains(&t) {
                stack.truncate(i);
                return;
            }
            if scope.contains(&t) || SCOPE.contains(&t) {
                return;
            }
        }
    };
    if CLOSES_P.contains(&tag) {
        close_in_scope(stack, &["p"], &[]);
    }
    match tag {
        "li" => close_in_scope(stack, &["li"], &[]),
        "dt" | "dd" => close_in_scope(stack, &["dt", "dd"], &["dl"]),
        "option" => close_in_scope(stack, &["option"], &[]),
        "tr" => {
            close_in_scope(stack, &["td", "th"], &["tr"]);
            close_in_scope(stack, &["tr"], &["thead", "tbody", "tfoot"]);
        }
        "td" | "th" => close_in_scope(stack, &["td", "th"], &["tr"]),
        "thead" | "tbody" | "tfoot" => {
            close_in_scope(stack, &["td", "th"], &["tr"]);
            close_in_scope(stack, &["tr"], &["thead", "tbody", "tfoot"]);
            close_in_scope(stack, &["thead", "tbody", "tfoot"], &[]);
        }
        _ => {}
    }
}

fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let s = src;
    let b = s.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    let flush = |out: &mut Vec<Token>, from: usize, to: usize| {
        if to > from {
            out.push(Token::Text(decode_entities(&s[from..to])));
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            flush(&mut out, text_start, i);
            let end = rest[4..].find("-->").map_or(s.len(), |e| i + 4 + e + 3);
            i = end;
            text_start = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut out, text_start, i);
            i = rest.find('>').map_or(s.len(), |e| i + e + 1);
            text_start = i;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if next == b'/' && b.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
            flush(&mut out, text_start, i);
            let close = rest.find('>').map_or(s.len(), |e| i + e + 1);
            let name: String = s[i + 2..close.min(s.len())]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
                .collect::<String>()
                .to_ascii_lowercase();
            out.push(Token::End(name));
            i = close;
            text_start = i;
            continue;
        }
        if !next.is_ascii_alphabetic() {
            // A lone '<' is text.
            i += 1;
            continue;
        }
        flush(&mut out, text_start, i);
        let (tok, end) = start_tag(s, i);
        i = end;
        text_start = i;
        if let Token::Start { tag, self_closing: false, .. } = &tok {
            if RAW.contains(&tag.as_str()) {
                let tag = tag.clone();
                out.push(tok);
                // Raw text to the matching end tag (case-insensitive).
                let lower = s[i..].to_ascii_lowercase();
                let needle = format!("</{tag}");
                let stop = lower.find(&needle).map_or(s.len(), |e| i + e);
                let raw = &s[i..stop];
                let text = if tag == "title" || tag == "textarea" { decode_entities(raw) } else { raw.to_string() };
                // A textarea drops one leading newline, like browsers.
                let text = if tag == "textarea" { text.strip_prefix('\n').map(str::to_string).unwrap_or(text) } else { text };
                if !text.is_empty() {
                    out.push(Token::Text(text));
                }
                out.push(Token::End(tag));
                i = s[stop..].find('>').map_or(s.len(), |e| stop + e + 1);
                text_start = i;
                continue;
            }
        }
        out.push(tok);
    }
    flush(&mut out, text_start, s.len());
    out
}

/// A start tag at `at` (`<` there): the token and where it ends.
fn start_tag(s: &str, at: usize) -> (Token, usize) {
    let b = s.as_bytes();
    let mut i = at + 1;
    let name_start = i;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    let tag = s[name_start..i].to_ascii_lowercase();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        match b[i] {
            b'>' => {
                i += 1;
                break;
            }
            b'/' => {
                i += 1;
                if b.get(i) == Some(&b'>') {
                    self_closing = true;
                    i += 1;
                    break;
                }
                continue;
            }
            _ => {}
        }
        let n0 = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'>' && !(b[i] == b'/' && b.get(i + 1) == Some(&b'>')) {
            i += 1;
        }
        let name = s[n0..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            match b.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    let v0 = i + 1;
                    let end = s[v0..].find(q as char).map_or(s.len(), |e| v0 + e);
                    value = decode_entities(&s[v0..end]);
                    i = (end + 1).min(s.len());
                }
                _ => {
                    let v0 = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&s[v0..i]);
                }
            }
        }
        if !name.is_empty() && !attrs.iter().any(|(k, _)| *k == name) {
            attrs.push((name, value));
        }
    }
    (Token::Start { tag, attrs, self_closing }, i.min(s.len()))
}

/// Character references: `&amp;`, `&#65;`, `&#x41;` and the common names.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest[1..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '#')).map(|e| e + 1).unwrap_or(rest.len());
        let name = &rest[1..end];
        let decoded = if let Some(num) = name.strip_prefix('#') {
            let v = match num.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => num.parse::<u32>().ok(),
            };
            v.and_then(char::from_u32).map(|c| c.to_string())
        } else {
            named_entity(name).map(str::to_string)
        };
        match decoded {
            Some(d) if !name.is_empty() => {
                out.push_str(&d);
                rest = &rest[end..];
                if rest.starts_with(';') {
                    rest = &rest[1..];
                }
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(n: &str) -> Option<&'static str> {
    Some(match n {
        "amp" | "AMP" => "&",
        "lt" | "LT" => "<",
        "gt" | "GT" => ">",
        "quot" | "QUOT" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "copy" => "\u{a9}",
        "reg" => "\u{ae}",
        "trade" => "\u{2122}",
        "hellip" => "\u{2026}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "laquo" => "\u{ab}",
        "raquo" => "\u{bb}",
        "middot" => "\u{b7}",
        "bull" => "\u{2022}",
        "deg" => "\u{b0}",
        "times" => "\u{d7}",
        "divide" => "\u{f7}",
        "plusmn" => "\u{b1}",
        "euro" => "\u{20ac}",
        "pound" => "\u{a3}",
        "yen" => "\u{a5}",
        "cent" => "\u{a2}",
        "sect" => "\u{a7}",
        "para" => "\u{b6}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201c}",
        "rdquo" => "\u{201d}",
        "larr" => "\u{2190}",
        "rarr" => "\u{2192}",
        "uarr" => "\u{2191}",
        "darr" => "\u{2193}",
        "frac12" => "\u{bd}",
        "frac14" => "\u{bc}",
        "frac34" => "\u{be}",
        "iexcl" => "\u{a1}",
        "iquest" => "\u{bf}",
        "shy" => "",
        "zwnj" | "zwj" => "",
        "eacute" => "\u{e9}",
        "egrave" => "\u{e8}",
        "aacute" => "\u{e1}",
        "agrave" => "\u{e0}",
        "ouml" => "\u{f6}",
        "uuml" => "\u{fc}",
        "auml" => "\u{e4}",
        "szlig" => "\u{df}",
        "ntilde" => "\u{f1}",
        "ccedil" => "\u{e7}",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(dom: &Dom, id: NodeId) -> Vec<String> {
        dom.element_children(id).map(|c| dom.tag(c).unwrap().to_string()).collect()
    }

    #[test]
    fn builds_a_tree() {
        let d = parse("<!DOCTYPE html><html><head><title>T &amp; U</title></head><body><h1 class=big>Hi</h1><p>a<b>b</b>c</p></body></html>");
        assert_eq!(d.title(), "T & U");
        let body = d.find("body").unwrap();
        assert_eq!(tags(&d, body), ["h1", "p"]);
        let h1 = d.find("h1").unwrap();
        assert!(d.element(h1).unwrap().has_class("big"));
        assert_eq!(d.text_content(d.find("p").unwrap()), "abc");
    }

    #[test]
    fn implied_ends() {
        let d = parse("<ul><li>one<li>two</ul><p>para<div>block</div><p>a<p>b");
        let ul = d.find("ul").unwrap();
        assert_eq!(tags(&d, ul), ["li", "li"]);
        assert_eq!(tags(&d, 0), ["ul", "p", "div", "p", "p"]);
        let d = parse("<table><tr><td>1<td>2<tr><td>3</table>");
        let table = d.find("table").unwrap();
        assert_eq!(tags(&d, table), ["tr", "tr"]);
        assert_eq!(d.find_all("td").len(), 3);
        // A list inside a list item doesn't close the item.
        let d = parse("<ul><li>a<ul><li>b</ul><li>c</ul>");
        assert_eq!(tags(&d, d.find("ul").unwrap()), ["li", "li"]);
    }

    #[test]
    fn forgiving() {
        // Stray end tags, a lone '<', unknown tags, unclosed elements, junk.
        let d = parse("</div>a < b <blink>keep</blink> <x-thing>me</x-thing><p>open");
        assert_eq!(d.text_content(0), "a < b keep meopen");
        assert_eq!(d.find("blink").map(|n| d.text_content(n)), Some("keep".into()));
        let _ = parse("<a href='x' <<>>< / ><b");
        let _ = parse("<!-- never closed");
        let _ = parse("<script>if (a < b) {}");
        let _ = parse("&#xZZ; &unknown; & &#99999999;");
    }

    #[test]
    fn attributes_and_raw_text() {
        let d = parse("<a href=\"/x?a=1&amp;b=2\" data-x='q' checked>l</a><input value=v disabled/><script>var s = '<b>';</script><style>p > a {}</style>");
        let a = d.find("a").unwrap();
        assert_eq!(d.attr(a, "href"), Some("/x?a=1&b=2"));
        assert_eq!(d.attr(a, "data-x"), Some("q"));
        assert_eq!(d.attr(a, "checked"), Some(""));
        let input = d.find("input").unwrap();
        assert_eq!(d.attr(input, "value"), Some("v"));
        assert!(d.nodes[input].children.is_empty());
        assert_eq!(d.text_content(d.find("script").unwrap()), "var s = '<b>';");
        assert!(d.find("b").is_none());
        assert_eq!(d.text_content(d.find("style").unwrap()), "p > a {}");
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a&lt;b&gt;&amp;&#65;&#x42;&nbsp;&copy"), "a<b>&AB\u{a0}\u{a9}");
        assert_eq!(decode_entities("AT&T"), "AT&T");
    }
}
