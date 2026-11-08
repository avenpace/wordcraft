//! `window.wordcraft`: the page command API (`craft-web-api`, storytold/craft-libs#413).
//!
//! The crate installs `request`, `execute`, `commands`, `docInspect`, `uiInspect` and `screenshot`
//! over the same JSON methods as the desktop control server (`docs/control-protocol.md`), as
//! promises with a timeout, so an agent driving WordCraft in a browser tab (or an `<iframe>` in
//! another app) uses the same commands as the CLI and the MCP server:
//!
//! ```js
//! await wordcraft.execute("text.insert", {text: "Hello"})  // engine command → its result
//! await wordcraft.request("document.inspect")               // control method → its result
//! await wordcraft.commands()                                // command list
//! const {png} = await wordcraft.screenshot()                // the window as PNG (base64)
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`.
//!
//! Requests take the app's own control channel: [`channel`]'s receiver goes to
//! [`wordcraft_ui_egui::WordApp::with_control`], and the app runs what its sender queues on its
//! next frame, exactly as the desktop control server's requests (`apps/wordcraft/src/control_server.rs`).
//! A request the page stopped waiting for (its timeout fired) still runs; its reply is dropped
//! with the receiver.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};
use serde_json::{Value, json};
use wasm_bindgen_futures::JsFuture;
use wordcraft_ui_egui::ControlRequest;

/// The app id in `window.<app>`.
const APP: &str = "wordcraft";

/// How often a waiting call looks for the app's reply, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

thread_local! {
    /// The sending end of the app's control channel ([`channel`]): the page's requests go through
    /// it, and the app drains them each frame (`WordApp::logic` → `drain_control`).
    static TX: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    /// The repaint context, so a page call wakes a resting page for the frame that answers it.
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// The app's control channel: its receiver goes to [`WordApp::with_control`], its sender stays
/// here for the page's requests ([`send`]). Made once, with the app it drives (`web`).
pub fn channel() -> Receiver<ControlRequest> {
    let (tx, rx) = std::sync::mpsc::channel();
    TX.with(|t| *t.borrow_mut() = Some(tx));
    rx
}

/// Keep the repaint context [`send`] asks for the frame that answers a page call.
pub fn connect(ctx: egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

/// Queue one control request on the app's channel and wait for its reply, as the protocol's
/// `{ok, result}` / `{ok: false, error}` object. The wait polls without blocking the page (about
/// once a frame): the app answers on its next frame, and a hidden tab answers when it is shown
/// again.
fn send(method: &str, params: Value) -> Pin<Box<dyn Future<Output = Value>>> {
    let (req, reply) = ControlRequest::new(method, params);
    let queued = TX.with(|t| t.borrow().as_ref().is_some_and(|tx| tx.send(req).is_ok()));
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
    Box::pin(async move {
        if !queued {
            return json!({"ok": false, "error": "the app is not running"});
        }
        loop {
            match reply.try_recv() {
                Ok(v) => return v,
                Err(TryRecvError::Empty) => sleep(POLL_MS).await,
                Err(TryRecvError::Disconnected) => return json!({"ok": false, "error": "request dropped"}),
            }
        }
    })
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Install `window.wordcraft` ([`craft_web_api::page::PageApi`]): `request`, `execute`,
/// `commands`, `docInspect`, `uiInspect` and `screenshot` over the desktop control methods, as
/// promises with a timeout. Only called with the app's channel in place (`web`).
pub fn install() -> Result<(), wasm_bindgen::JsValue> {
    let request: Request = Rc::new(|method, params| send(&method, params));
    Rc::new(PageApi::new(APP, Methods::default(), request)).install()
}
