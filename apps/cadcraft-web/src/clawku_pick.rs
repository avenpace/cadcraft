//! Clawku: File › Open asks the host page for a file from its own storage instead of opening the
//! browser's file picker. Opt-in: only a page that loads the app with `?host=parent&open=host`
//! (Clawku) gets `<app>:pick` (`{ type, purpose }`) on the same-origin parent; every other page
//! keeps the browser picker. The host answers with the usual `<app>:open` message, or not at all.
//!
//! The same opt-in makes the app hosted ([`is_hosted`], `Services::hosted`): one drawing, no New
//! or Close, and Share asks the page (`<app>:share`, [`host_share`]).

use wasm_bindgen::JsValue;

/// The framing parent and this page's origin when the page is hosted that way; `None` when it
/// isn't (no such query, or not in a frame).
fn host_parent() -> Option<(web_sys::Window, String)> {
    let win = web_sys::window()?;
    if !crate::wants_host_open(&win.location().search().unwrap_or_default()) {
        return None;
    }
    let parent = win.parent().ok().flatten()?;
    if js_sys::Object::is(&parent, &win) {
        return None; // not framed
    }
    let origin = win.location().origin().ok()?;
    Some((parent, origin))
}

/// Whether this page is hosted by Clawku: `?host=parent&open=host` inside a frame.
pub fn is_hosted() -> bool {
    host_parent().is_some()
}

/// Post `{ type: "<app>:<kind>", ...fields }` to the hosting parent; `false` when not hosted.
fn post(app: &str, kind: &str, fields: &[(&str, &str)]) -> bool {
    let Some((parent, origin)) = host_parent() else { return false };
    let msg = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str(&format!("{app}:{kind}")));
    for (k, v) in fields {
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str(k), &JsValue::from_str(v));
    }
    parent.post_message(&msg, &origin).is_ok()
}

/// Ask the host for a file to open; `false` when this page isn't hosted that way (use the picker).
pub fn host_pick(app: &str, purpose: &str) -> bool {
    post(app, "pick", &[("purpose", purpose)])
}

/// Ask the host to share the open document (`<app>:share`); `false` when not hosted.
pub fn host_share(app: &str) -> bool {
    post(app, "share", &[])
}
