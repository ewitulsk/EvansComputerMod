//! Driving the apps through a scripted UI connection (tests and the golden recorder).

use ecm_ui::{samples as ui_samples, App, MockConnection};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;

/// A UI connection to a `w x h` display, the window configured full size and activated.
pub fn mock(w: i32, h: i32) -> MockConnection {
    let m = MockConnection::new();
    m.state.borrow_mut().idle_timeouts = true;
    let mut wl = ui_samples::welcome(false);
    if let Message::Welcome(ref mut x) = wl {
        x.display.w = w;
        x.display.h = h;
        x.display.size_class = proto::SizeClass::for_width(w);
    }
    m.push(&[wl]);
    m
}

/// The configure the shell sends for window 1.
pub fn configure(m: &MockConnection, w: i32, h: i32) {
    m.push(&[Message::Configure(proto::Configure { surface: 1, serial: 1, w, h, states: proto::window_states::FULLSCREEN | proto::window_states::ACTIVATED })]);
}

/// The hit id of widget `id` in window 1.
pub fn hit(app: &App, id: &str) -> u32 {
    app.core.window(1).unwrap().hit_id_of(id).unwrap_or_else(|| panic!("no widget {id}"))
}

/// Push a click on `id` (the caller steps the app).
pub fn push_click(app: &App, m: &MockConnection, id: &str) {
    m.push(&ui_samples::click(hit(app, id), 10, 10));
}

/// Push typed text (into the focused field).
pub fn push_text(m: &MockConnection, text: &str) {
    m.push(&[Message::Text(proto::Text { surface: 1, text: text.into() })]);
}

/// Everything the app sent, as text (labels' strings are in it).
pub fn sent_text(m: &MockConnection) -> String {
    format!("{:?}", m.sent_messages())
}
