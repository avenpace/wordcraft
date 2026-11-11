//! Clawku: File › Open asks the host page for a file from its own storage instead of opening the
//! browser's file picker. Opt-in: only a page that loads the app with `?host=parent&open=host`
//! (Clawku) gets `<app>:pick` (`{ type, purpose }`) on the same-origin parent; every other page
//! keeps the browser picker. The host answers with the usual `<app>:open` message, or not at all.
//! Share goes to the host the same way (`<app>:share`): sharing a stored document with people is
//! the host's (Clawku's Share dialog), not exporting a copy.

use wasm_bindgen::JsValue;

/// Ask the host for a file to open; `false` when this page isn't hosted that way (use the picker).
pub fn host_pick(app: &str, purpose: &str) -> bool {
    post_to_host(app, "pick", Some(purpose))
}

/// Ask the host to share the document with people; `false` when this page isn't hosted that way
/// (Share exports a copy instead).
pub fn host_share(app: &str) -> bool {
    post_to_host(app, "share", None)
}

/// Whether this page is framed by a host that opens and shares documents itself (Clawku loads
/// the app with `?host=parent&open=host`).
pub fn hosted() -> bool {
    let Some(win) = web_sys::window() else { return false };
    let search = win.location().search().unwrap_or_default();
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else { return false };
    if params.get("host").as_deref() != Some("parent") || params.get("open").as_deref() != Some("host") {
        return false;
    }
    let Ok(Some(parent)) = win.parent() else { return false };
    !js_sys::Object::is(&parent, &win)
}

/// `{ type: "<app>:<kind>", purpose? }` to the same-origin parent of a page loaded with
/// `?host=parent&open=host`; `false` for any other page.
fn post_to_host(app: &str, kind: &str, purpose: Option<&str>) -> bool {
    let Some(win) = web_sys::window() else { return false };
    if !hosted() {
        return false;
    }
    let Ok(Some(parent)) = win.parent() else { return false };
    let Ok(origin) = win.location().origin() else { return false };
    let msg = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str(&format!("{app}:{kind}")));
    if let Some(purpose) = purpose {
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("purpose"), &JsValue::from_str(purpose));
    }
    parent.post_message(&msg, &origin).is_ok()
}
