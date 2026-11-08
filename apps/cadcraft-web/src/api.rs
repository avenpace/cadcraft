//! `window.cadcraft`: the page command API (`craft-web-api`, storytold/craft-libs) over the app's
//! control channel (`cadcraft_ui_egui::control`, `docs/control-protocol.md`).
//!
//! The crate installs `request`, `execute`, `commands`, `docInspect`, `uiInspect` and `screenshot`
//! over the same JSON methods as the desktop control server, as promises with a timeout, so an
//! agent driving CADCraft in a browser tab (or an `<iframe>` in another app) uses the same
//! commands as the CLI and the MCP server:
//!
//! ```js
//! await cadcraft.execute("line", {points: [[0, 0], [10, 0]]})  // engine command → its result
//! await cadcraft.request("cmdline.state")                      // control method → its result
//! await cadcraft.commands()                                    // command list
//! await cadcraft.docInspect({})                                // drawing summary and entities
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`. Installed only with
//! `?control` (`main.rs`): a page that never asks for it gets no globals.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use cadcraft_ui_egui::{ControlRequest, PendingReply};
use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// The app id in `window.<app>`.
const APP: &str = "cadcraft";

/// How often a waiting call looks for the app's reply, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

thread_local! {
    static QUEUE: RefCell<Vec<ControlRequest>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// The context to repaint when a request is queued, so even an idle tab drains it at once.
pub fn connect(ctx: egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

/// The page's requests since the last frame, for the app's control channel.
pub fn take_requests() -> Vec<ControlRequest> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// A queued control request whose reply has not arrived. Dropping it before the reply (the page's
/// timeout fired) cancels the request if the app has not started it: the page was told it failed,
/// so the app must not run it late.
struct Waiting {
    reply: PendingReply,
    answered: bool,
}

impl Waiting {
    /// The protocol's reply object once the app has answered or gone away; `None` while the
    /// request is still waiting.
    fn poll(&mut self) -> Option<Value> {
        let reply = self.reply.try_take()?;
        self.answered = true;
        Some(reply)
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.answered {
            self.reply.cancel_if_open();
        }
    }
}

/// Queue a control request for the next frame and wait for the app's reply: the protocol's
/// `{ok, result}` / `{ok: false, error}` object, as the app sends it.
fn send(method: &str, params: Value) -> Pin<Box<dyn Future<Output = Value>>> {
    let (request, reply) = ControlRequest::new(method, params);
    QUEUE.with(|q| q.borrow_mut().push(request));
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
    let mut waiting = Waiting { reply, answered: false };
    Box::pin(async move {
        loop {
            if let Some(reply) = waiting.poll() {
                return reply;
            }
            sleep(POLL_MS).await;
        }
    })
}

/// Yield to the page for `ms` milliseconds; the reply arrives on a frame the repaint asked for.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Install `window.cadcraft`: the crate's functions over this app's control methods
/// (`Methods::default()`, which the handler answers as-is).
pub fn install() -> Result<(), JsValue> {
    let request: Request = Rc::new(|method, params| send(&method, params));
    Rc::new(PageApi::new(APP, Methods::default(), request)).install()
}
