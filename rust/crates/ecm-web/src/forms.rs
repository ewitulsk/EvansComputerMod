//! Forms: the controls on a page, their values (kept in the DOM: `value`
//! and `checked` attributes, a textarea's text, an option's `selected`), and
//! what a submit sends (GET: the query; POST: an urlencoded body).
//!
//! The browser doesn't check `required` or patterns itself: the server
//! decides, as a CGI script or a `web.serve` handler does.

use crate::dom::{Dom, NodeData, NodeId};
use crate::url::{form_encode, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// `input` text, password, email, number, search, url, tel (anything typed).
    Text { password: bool },
    TextArea,
    Checkbox,
    Radio,
    Select,
    /// Submits its form (`input type=submit|image`, `button` without type or type=submit).
    Submit,
    /// `type=reset`: puts the form's values back as the page had them (v1: reloads them from nothing; ignored).
    Reset,
    /// `type=button`, `button type=button`: does nothing without scripts.
    Button,
    Hidden,
}

impl Control {
    pub fn is_text(self) -> bool {
        matches!(self, Control::Text { .. } | Control::TextArea)
    }
}

/// What kind of control `node` is, if any.
pub fn control(dom: &Dom, node: NodeId) -> Option<Control> {
    let e = dom.element(node)?;
    let ty = e.attr("type").unwrap_or("").to_ascii_lowercase();
    Some(match e.tag.as_str() {
        "input" => match ty.as_str() {
            "checkbox" => Control::Checkbox,
            "radio" => Control::Radio,
            "submit" | "image" => Control::Submit,
            "reset" => Control::Reset,
            "button" => Control::Button,
            "hidden" => Control::Hidden,
            "password" => Control::Text { password: true },
            _ => Control::Text { password: false },
        },
        "textarea" => Control::TextArea,
        "select" => Control::Select,
        "button" => match ty.as_str() {
            "button" => Control::Button,
            "reset" => Control::Reset,
            _ => Control::Submit,
        },
        _ => return None,
    })
}

pub fn disabled(dom: &Dom, node: NodeId) -> bool {
    dom.attr(node, "disabled").is_some() || dom.closest(node, |e| e.tag == "fieldset" && e.attr("disabled").is_some()).is_some()
}

/// The control a click at `node` acts on: the control itself, the one inside
/// a `<label>`, or the one its `for` names.
pub fn target(dom: &Dom, node: NodeId) -> Option<NodeId> {
    if let Some(c) = dom.closest(node, |e| matches!(e.tag.as_str(), "input" | "textarea" | "select" | "button")) {
        return Some(c);
    }
    let label = dom.closest(node, |e| e.tag == "label")?;
    if let Some(id) = dom.attr(label, "for") {
        return dom.by_id(id).filter(|n| control(dom, *n).is_some());
    }
    dom.descendants(label).into_iter().find(|n| control(dom, *n).is_some_and(|c| c != Control::Hidden))
}

/// The form a control belongs to (its `form` attribute, else the enclosing `<form>`).
pub fn form_of(dom: &Dom, node: NodeId) -> Option<NodeId> {
    if let Some(id) = dom.attr(node, "form") {
        if let Some(f) = dom.by_id(id).filter(|f| dom.tag(*f) == Some("form")) {
            return Some(f);
        }
    }
    dom.closest(node, |e| e.tag == "form")
}

/// A text control's current value.
pub fn value(dom: &Dom, node: NodeId) -> String {
    match dom.tag(node) {
        Some("textarea") => dom.text_content(node),
        Some("select") => selected(dom, node).map(|o| option_value(dom, o)).unwrap_or_default(),
        _ => dom.attr(node, "value").unwrap_or("").to_string(),
    }
}

/// Set a text control's value.
pub fn set_value(dom: &mut Dom, node: NodeId, v: &str) {
    if dom.tag(node) == Some("textarea") {
        dom.set_text(node, v);
    } else if let Some(e) = dom.element_mut(node) {
        e.set_attr("value", v);
    }
}

pub fn checked(dom: &Dom, node: NodeId) -> bool {
    dom.attr(node, "checked").is_some()
}

/// Click a checkbox (flips) or a radio button (on, the others of its group off).
pub fn toggle(dom: &mut Dom, node: NodeId) {
    match control(dom, node) {
        Some(Control::Checkbox) => {
            let on = checked(dom, node);
            set_checked(dom, node, !on);
        }
        Some(Control::Radio) => {
            let name = dom.attr(node, "name").unwrap_or("").to_string();
            let form = form_of(dom, node);
            if !name.is_empty() {
                let group: Vec<NodeId> = dom
                    .find_all("input")
                    .into_iter()
                    .filter(|n| control(dom, *n) == Some(Control::Radio) && dom.attr(*n, "name") == Some(name.as_str()) && form_of(dom, *n) == form)
                    .collect();
                for g in group {
                    set_checked(dom, g, false);
                }
            }
            set_checked(dom, node, true);
        }
        _ => {}
    }
}

fn set_checked(dom: &mut Dom, node: NodeId, on: bool) {
    if let Some(e) = dom.element_mut(node) {
        if on {
            e.set_attr("checked", "");
        } else {
            e.attrs.retain(|(k, _)| k != "checked");
        }
    }
}

/// A select's `<option>`s in order.
pub fn options(dom: &Dom, select: NodeId) -> Vec<NodeId> {
    dom.descendants(select).into_iter().filter(|n| dom.tag(*n) == Some("option")).collect()
}

/// The chosen option (the last `selected`, else the first).
pub fn selected(dom: &Dom, select: NodeId) -> Option<NodeId> {
    let opts = options(dom, select);
    opts.iter().rev().copied().find(|o| dom.attr(*o, "selected").is_some()).or(opts.first().copied())
}

pub fn option_label(dom: &Dom, option: NodeId) -> String {
    crate::dom::collapse_ws(&dom.text_content(option))
}

pub fn option_value(dom: &Dom, option: NodeId) -> String {
    dom.attr(option, "value").map(str::to_string).unwrap_or_else(|| option_label(dom, option))
}

/// Choose option `index` of a select.
pub fn choose(dom: &mut Dom, select: NodeId, index: usize) {
    let opts = options(dom, select);
    for (i, o) in opts.iter().enumerate() {
        if let Some(e) = dom.element_mut(*o) {
            e.attrs.retain(|(k, _)| k != "selected");
            if i == index {
                e.set_attr("selected", "");
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Submission {
    pub post: bool,
    pub url: Url,
    /// POST: the urlencoded body.
    pub body: Option<String>,
}

/// The name/value pairs a form sends, in document order (`submitter`: the button pressed).
pub fn entries(dom: &Dom, form: NodeId, submitter: Option<NodeId>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for n in dom.descendants(crate::dom::DOCUMENT) {
        let Some(c) = control(dom, n) else { continue };
        if form_of(dom, n) != Some(form) || disabled(dom, n) {
            continue;
        }
        let name = dom.attr(n, "name").unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        match c {
            Control::Text { .. } | Control::TextArea | Control::Hidden | Control::Select => {
                let v = value(dom, n);
                // A textarea's lines are sent with CRLF, as browsers do.
                let v = if c == Control::TextArea { v.replace("\r\n", "\n").replace('\n', "\r\n") } else { v };
                out.push((name, v));
            }
            Control::Checkbox | Control::Radio => {
                if checked(dom, n) {
                    out.push((name, dom.attr(n, "value").unwrap_or("on").to_string()));
                }
            }
            Control::Submit => {
                if submitter == Some(n) {
                    out.push((name, dom.attr(n, "value").unwrap_or("").to_string()));
                }
            }
            Control::Reset | Control::Button => {}
        }
    }
    out
}

/// What submitting `form` (with `submitter`) from a page at `base` does.
pub fn submission(dom: &Dom, base: &Url, form: NodeId, submitter: Option<NodeId>) -> Option<Submission> {
    let action = submitter
        .and_then(|s| dom.attr(s, "formaction"))
        .or_else(|| dom.attr(form, "action"))
        .unwrap_or("");
    let mut url = base.join(action)?;
    let method = submitter.and_then(|s| dom.attr(s, "formmethod")).or_else(|| dom.attr(form, "method")).unwrap_or("get");
    let post = method.eq_ignore_ascii_case("post");
    let encoded = entries(dom, form, submitter).iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect::<Vec<_>>().join("&");
    url.fragment = None;
    if post {
        Some(Submission { post, url, body: Some(encoded) })
    } else {
        url.query = Some(encoded);
        Some(Submission { post, url, body: None })
    }
}

/// The form's first submit button (Enter in a text field presses it).
pub fn default_button(dom: &Dom, form: NodeId) -> Option<NodeId> {
    dom.descendants(crate::dom::DOCUMENT).into_iter().find(|n| control(dom, *n) == Some(Control::Submit) && form_of(dom, *n) == Some(form))
}

/// The next (or previous) focusable control after `from` (Tab).
pub fn next_control(dom: &Dom, from: Option<NodeId>, back: bool) -> Option<NodeId> {
    let all: Vec<NodeId> = dom
        .descendants(crate::dom::DOCUMENT)
        .into_iter()
        .filter(|n| control(dom, *n).is_some_and(|c| c != Control::Hidden) && !disabled(dom, *n))
        .collect();
    if all.is_empty() {
        return None;
    }
    let pos = from.and_then(|f| all.iter().position(|n| *n == f));
    let i = match (pos, back) {
        (None, false) => 0,
        (None, true) => all.len() - 1,
        (Some(p), false) => (p + 1) % all.len(),
        (Some(p), true) => (p + all.len() - 1) % all.len(),
    };
    Some(all[i])
}

impl Dom {
    /// Replace an element's children with one text node.
    pub fn set_text(&mut self, node: NodeId, text: &str) {
        let kids = std::mem::take(&mut self.nodes[node].children);
        for k in kids {
            self.nodes[k].parent = None;
        }
        if !text.is_empty() {
            self.push(node, NodeData::Text(text.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    const FORM: &str = "<form action=/cgi/guest.py method=post>\
        <label>Name <input name=name value=Al></label>\
        <label for=m>Message</label><textarea id=m name=msg>hi\nthere</textarea>\
        <input type=checkbox name=public value=yes checked><input type=checkbox name=other>\
        <input type=radio name=color value=red checked><input type=radio name=color value=blue>\
        <select name=size><option>S<option value=m selected>Medium</select>\
        <input type=hidden name=token value=t1><input name=off disabled value=x>\
        <button name=go value=send>Send</button><input type=submit name=alt value=Other></form>\
        <form id=f2><input name=q></form><input form=f2 name=outside value=1>";

    #[test]
    fn post_body_in_document_order() {
        let d = html::parse(FORM);
        let form = d.find("form").unwrap();
        let go = d.find("button").unwrap();
        let s = submission(&d, &Url::parse("http://h/page.html").unwrap(), form, Some(go)).unwrap();
        assert!(s.post);
        assert_eq!(s.url.to_string(), "http://h/cgi/guest.py");
        assert_eq!(s.body.unwrap(), "name=Al&msg=hi%0D%0Athere&public=yes&color=red&size=m&token=t1&go=send");
    }

    #[test]
    fn editing_toggling_choosing() {
        let mut d = html::parse(FORM);
        let form = d.find("form").unwrap();
        let input = d.find("input").unwrap();
        set_value(&mut d, input, "Bo & Cy");
        let ta = d.find("textarea").unwrap();
        set_value(&mut d, ta, "new");
        let boxes = d.find_all("input");
        toggle(&mut d, boxes[1]); // public off
        toggle(&mut d, boxes[2]); // other on
        toggle(&mut d, boxes[4]); // blue: red goes off
        let sel = d.find("select").unwrap();
        choose(&mut d, sel, 0);
        let e = entries(&d, form, None);
        let get = |k: &str| e.iter().filter(|(n, _)| n == k).map(|(_, v)| v.as_str()).collect::<Vec<_>>();
        assert_eq!(get("name"), ["Bo & Cy"]);
        assert_eq!(get("msg"), ["new"]);
        assert!(get("public").is_empty());
        assert_eq!(get("other"), ["on"]);
        assert_eq!(get("color"), ["blue"]);
        assert_eq!(get("size"), ["S"]);
        assert!(get("off").is_empty(), "disabled controls aren't sent");
        assert!(get("go").is_empty() && get("alt").is_empty(), "only the button pressed is sent");
    }

    #[test]
    fn get_forms_labels_and_tab() {
        let d = html::parse(FORM);
        let f2 = d.by_id("f2").unwrap();
        let s = submission(&d, &Url::parse("http://h/dir/p.html#x").unwrap(), f2, None).unwrap();
        assert!(!s.post);
        assert_eq!(s.url.to_string(), "http://h/dir/p.html?q=&outside=1");
        // A label's text points at its control.
        let labels = d.find_all("label");
        let name_text = d.nodes[labels[0]].children[0];
        assert_eq!(target(&d, name_text), d.find("input"));
        assert_eq!(target(&d, labels[1]), d.by_id("m"));
        assert_eq!(default_button(&d, d.find("form").unwrap()), d.find("button"));
        let first = next_control(&d, None, false).unwrap();
        assert_eq!(first, d.find("input").unwrap());
        assert_eq!(next_control(&d, Some(first), false), d.find("textarea"));
    }
}
