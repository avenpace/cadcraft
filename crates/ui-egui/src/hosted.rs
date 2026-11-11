//! Hosted: a page that owns the files runs the app ([`crate::Services::hosted`]; Clawku frames
//! the web build with `?host=parent&open=host`). The page keeps the file list, New, Open, Close,
//! Rename, Share and versions, so the app shows the one drawing the page sent:
//!
//! - every way to make, show or close another drawing, or an empty app, is hidden ([`hides`]):
//!   New, New from Template, the sample drawing, the Start page, Close / Close All, Open Recent,
//!   Recover and the Drawing Recovery Manager, Quit, and the file tabs. Typed or from a shortcut
//!   they say the page manages files instead of running.
//! - Save hands the drawing back under the name the page sent; Save As under the same name,
//!   marked as a Save As (the page asks for the new name) ([`save`]).
//! - Share asks the page to share it ([`crate::Services::share`]).
//! - IMPORT and a linetype file come from the browser's own file picker
//!   ([`crate::Services::pick_upload`], [`CadApp::receive_upload`]): only opening a drawing goes
//!   to the page.
//!
//! Not hosted, nothing here applies.

use serde_json::{Value, json};

use crate::CadApp;

/// What a file picked with the browser's own picker ([`crate::Services::pick_upload`]) is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upload {
    /// IMPORT: the drawing's objects are copied into the current drawing.
    Import,
    /// The Linetype Manager's Load ▸ From File: every linetype in a `.lin` file.
    Linetype,
}

impl Upload {
    /// The file types the picker offers (an `<input accept>` list).
    pub fn accept(self) -> &'static str {
        match self {
            Upload::Import => ".dxf,.dwg",
            Upload::Linetype => ".lin",
        }
    }
}

/// The commands a hosted app hides and refuses when the user starts them: each makes, shows or
/// closes another drawing (or an empty app), or needs files beside the drawing the web can't
/// reach (external references).
const HIDDEN: &[&str] = &[
    "new",
    "qnew",
    "ui.newtemplate",
    "ui.sample",
    "ui.start",
    "close",
    "closeall",
    "recover",
    "drawingrecovery",
    "ui.quit",
    "ui.toggle.filetabs",
    "xattach",
    "xopen",
    "attach",
];

/// Hidden too: what the hosted web build can't do at all (a 3D mouse needs the desktop app).
const DESKTOP_ONLY: &[&str] = &["ui.spacemouse", "ui.spacemouse.calibrate", "ui.spacemouse.navigate"];

/// File ▸ Share..., added to the File menu when hosted.
pub const SHARE_ID: &str = "share";
pub const SHARE_LABEL: &str = "Share...";

/// Whether a hosted app hides the command `id` (an alias resolves to its command; the recent
/// files are hidden with Open Recent).
pub fn hides(id: &str) -> bool {
    let id = cadcraft_engine::cmd::resolve_alias(id);
    HIDDEN.contains(&id.as_str()) || DESKTOP_ONLY.contains(&id.as_str()) || id.starts_with("ui.recent.")
}

/// A user's command (no parameters) in a hosted app: a hidden one says the page manages files,
/// Save As and Share go to the page. `None`: not hosted, or the command runs as usual. JSON calls (the
/// page's own control requests) always run.
pub(crate) fn route(app: &mut CadApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !app.services.hosted || !params.is_null() {
        return None;
    }
    if id == SHARE_ID {
        return Some(match app.services.share.as_ref() {
            Some(share) => {
                share();
                Ok(Value::Null)
            }
            None => Err("SHARE: the page that hosts ClawCAD doesn't share drawings".into()),
        });
    }
    // Save As: the page asks for the new name; the drawing goes to it under its current one.
    if matches!(id, "ui.saveas" | "saveas") {
        return Some(save(app, true));
    }
    if hides(id) {
        let name = id.trim_start_matches("ui.").to_ascii_uppercase();
        let msg = if DESKTOP_ONLY.contains(&id) {
            format!("{name}: not available here")
        } else {
            format!("{name}: the page that hosts ClawCAD manages the drawing files")
        };
        app.session.echo(msg.clone());
        app.set_status(msg);
        return Some(Ok(Value::Null));
    }
    None
}

/// Hosted Save (`save_as` false) and Save As (`true`): the drawing goes to the page under its
/// current name, the one the page sent (a DWG the page converted is edited and saved as
/// `<name>.dxf`, which the page converts back). Save As is marked as one around the save
/// ([`crate::Services::host_save_as`]), so the page asks for the new name and the original stays.
pub(crate) fn save(app: &mut CadApp, save_as: bool) -> Result<Value, String> {
    let st = app.session.state().map_err(|e| e.to_string())?;
    let name = st.path.clone().unwrap_or_else(|| st.title.clone());
    if !save_as && st.path.is_some() {
        // Saved once already: QSAVE writes it again (and asks first when that loses content).
        if app.hold_lossy_save() {
            return Ok(Value::Null);
        }
        return app.run("qsave", json!({}));
    }
    // The page sent a file: it keeps its own version (no default save version, which is for
    // drawings that never had a file).
    let params = json!({ "path": name });
    if save_as && let Some(mark) = app.services.host_save_as.as_ref() {
        mark(true);
    }
    let r = app.run("saveas", params);
    if save_as && let Some(mark) = app.services.host_save_as.as_ref() {
        mark(false);
    }
    r
}

impl CadApp {
    /// A file the browser's own picker read for `purpose` ([`crate::Services::pick_upload`]):
    /// IMPORT copies the drawing's objects into the current drawing, a `.lin` file loads its
    /// linetypes. Errors are echoed on the command line and shown in the status bar.
    pub fn receive_upload(&mut self, purpose: Upload, name: &str, bytes: &[u8]) {
        let r = match purpose {
            Upload::Import => self.run("import", json!({ "data": cadcraft_engine::cmd::file::base64_encode(bytes), "name": name })),
            Upload::Linetype => self.run("linetype", json!({ "load": "*", "text": String::from_utf8_lossy(bytes) })),
        };
        if let Err(e) = r {
            self.set_status(e);
        }
    }
}
