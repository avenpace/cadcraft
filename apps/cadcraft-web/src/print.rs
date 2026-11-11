//! Printing in the browser: the plotted sheet (PDF, or a PNG/SVG plot) is loaded into a hidden
//! `<iframe>` from an object URL and that frame's print dialog opens, so File › Print prints
//! instead of storing a file. Used where a host page owns the files (Clawku,
//! `Services::print`); the standalone web build keeps saving plots as downloads.

use wasm_bindgen::JsCast as _;
use web_sys::js_sys;

/// The hidden frame (one print at a time; a new print replaces it).
const FRAME_ID: &str = "cadcraft_print_frame";
/// How long the frame and its object URL stay after printing starts: the print dialog reads them.
const REMOVE_AFTER_MS: i32 = 120_000;

/// Open the browser's print dialog for `bytes`, a plotted sheet called `name` (its extension
/// gives the type). `Ok` once the frame was set up; the dialog opens when it has loaded.
pub fn print(name: &str, bytes: &[u8]) -> Result<(), String> {
    let window = web_sys::window().ok_or("no browser window")?;
    let document = window.document().ok_or("no browser document")?;
    let body = document.body().ok_or("the page has no body")?;
    if let Some(old) = document.get_element_by_id(FRAME_ID) {
        old.remove();
    }
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime(name));
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(js_error)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_error)?;
    let frame = document
        .create_element("iframe")
        .map_err(js_error)?
        .dyn_into::<web_sys::HtmlIFrameElement>()
        .map_err(|_| "could not create a print frame".to_string())?;
    frame.set_id(FRAME_ID);
    frame.set_title(name);
    // Off screen but laid out: a `display:none` frame doesn't load a PDF viewer to print.
    frame.set_attribute("style", "position:fixed;right:0;bottom:0;width:1px;height:1px;border:0;opacity:0").map_err(js_error)?;
    let loaded = frame.clone();
    let on_load = wasm_bindgen::closure::Closure::once_into_js(move || match loaded.content_window() {
        Some(w) => {
            if let Err(e) = w.print() {
                log::warn!("print: {}", js_error(e));
            }
        }
        None => log::warn!("print: the print frame has no window"),
    });
    frame.set_onload(Some(on_load.unchecked_ref()));
    frame.set_src(&url);
    body.append_child(&frame).map_err(js_error)?;
    let (later, gone) = (url.clone(), frame);
    let cleanup = wasm_bindgen::closure::Closure::once_into_js(move || {
        gone.remove();
        let _ = web_sys::Url::revoke_object_url(&later);
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(cleanup.unchecked_ref(), REMOVE_AFTER_MS);
    Ok(())
}

/// The MIME type of a plotted sheet, by its extension.
fn mime(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => "application/pdf",
    }
}

fn js_error(e: wasm_bindgen::JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}
