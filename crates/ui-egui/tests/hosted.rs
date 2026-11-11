//! Hosted by a page that owns the files (Clawku, `Services::hosted`): one drawing, no New, Close,
//! Start page or Open Recent; Save hands the drawing back under the name the page sent, Save As
//! is marked as one; Share goes to the page; IMPORT and a linetype file use the browser's own
//! picker, never the page's open picker; there is no Help at all (Clawku gives it). Not hosted,
//! the menus keep all of it.

use std::cell::RefCell;
use std::rc::Rc;

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::Drawing;
use cadcraft_ui_egui::hosted::{HELP_MSG, Upload};
use cadcraft_ui_egui::menus::{Entry, OPEN_RECENT, tree};
use cadcraft_ui_egui::{CadApp, Services};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// What the hosted app asked of its page.
#[derive(Default)]
struct Told {
    save_as: Vec<bool>,
    shares: usize,
    uploads: Vec<Upload>,
    opens: usize,
    names_asked: Vec<String>,
}

fn hosted_app() -> (CadApp, Rc<RefCell<Told>>) {
    let told = Rc::new(RefCell::new(Told::default()));
    let (a, b, c, d, e) = (told.clone(), told.clone(), told.clone(), told.clone(), told.clone());
    let services = Services {
        hosted: true,
        no_file_system: true,
        host_save_as: Some(Box::new(move |on| a.borrow_mut().save_as.push(on))),
        share: Some(Box::new(move || b.borrow_mut().shares += 1)),
        pick_upload: Some(Box::new(move |p| c.borrow_mut().uploads.push(p))),
        pick_open: Some(Box::new(move || {
            d.borrow_mut().opens += 1;
            None
        })),
        pick_save: Some(Box::new(move |name: &str| {
            e.borrow_mut().names_asked.push(name.to_string());
            Some(name.to_string())
        })),
        ..Services::default()
    };
    (CadApp::new(Session::empty(), services), told)
}

fn ids(entries: &[Entry], out: &mut Vec<String>) {
    for e in entries {
        match e {
            Entry::Item { id, .. } => out.push(id.clone()),
            Entry::Sub { label, children } => {
                out.push(format!("sub:{label}"));
                ids(children, out);
            }
        }
    }
}

fn menu_ids(app: &CadApp) -> Vec<String> {
    let mut out = Vec::new();
    for (_, entries) in tree(app) {
        ids(&entries, &mut out);
    }
    out
}

/// Hosted, the menus offer no way to another drawing or an empty app, and File has Share; not
/// hosted they keep New, Close, Start, Open Recent and the sample drawing, and have no Share.
#[test]
fn hosted_menus_hide_new_close_start_recent_and_show_share() {
    let (app, _) = hosted_app();
    let shown = menu_ids(&app);
    for id in [
        "new",
        "ui.newtemplate",
        "ui.sample",
        "ui.start",
        "close",
        "closeall",
        "recover",
        "drawingrecovery",
        "xattach",
        "ui.toggle.filetabs",
        "ui.spacemouse",
    ] {
        assert!(!shown.iter().any(|s| s == id), "hosted menus show {id}");
    }
    assert!(!shown.iter().any(|s| *s == format!("sub:{OPEN_RECENT}")), "hosted menus show Open Recent");
    for id in ["share", "open", "qsave", "saveas", "plot", "import", "export"] {
        assert!(shown.iter().any(|s| s == id), "hosted menus lost {id}");
    }

    let standalone = CadApp::new(Session::empty(), Services::default());
    let shown = menu_ids(&standalone);
    for id in ["new", "ui.sample", "ui.start", "close", "closeall", "recover"] {
        assert!(shown.iter().any(|s| s == id), "standalone menus lost {id}");
    }
    assert!(shown.iter().any(|s| *s == format!("sub:{OPEN_RECENT}")));
    assert!(!shown.iter().any(|s| s == "share"), "standalone File menu has Share");
}

/// The whole hosted flow on one drawing (one test: it changes the working folder, where the
/// native build writes what the web build hands to the page).
#[test]
fn hosted_app_keeps_one_drawing_saves_under_the_page_name_and_routes_share_and_import() {
    set_io(IoHooks {
        read: |_, _| Ok(Drawing::new_metric()),
        write: |_, _| Ok(b"0\nEOF\n".to_vec()),
        plot: Some(|_, _, _| Ok(b"%PDF-1.4".to_vec())),
    });
    let dir = std::env::temp_dir().join(format!("cadcraft-ui-hosted-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();

    let (mut app, told) = hosted_app();
    // The page sends a DWG converted to DXF as `<name>.dxf`.
    app.open_bytes("plan.dxf", b"0\nEOF\n");
    assert_eq!(app.session.docs.len(), 1);

    // New, Close, the Start page and the rest do nothing, typed, from a menu or a shortcut.
    for id in ["new", "qnew", "close", "closeall", "ui.start", "ui.sample", "ui.newtemplate", "recover", "drawingrecovery", "xattach"] {
        app.start(id);
        assert_eq!(app.session.docs.len(), 1, "{id} changed the drawings");
        assert!(!app.ui.start_tab, "{id} showed the Start page");
    }
    assert!(app.cmdline("new").is_none());
    assert_eq!(app.session.docs.len(), 1);

    // Save: the page's name, no Save As mark, no name asked.
    app.start("qsave");
    assert!(told.borrow().save_as.is_empty(), "Save was marked as a Save As");
    assert!(told.borrow().names_asked.is_empty(), "Save asked for a name");
    assert_eq!(
        std::fs::read(dir.join("plan.dxf")).ok().as_deref(),
        Some(&b"0\nEOF\n"[..]),
        "Save did not keep the page's name: {:?}",
        app.session.log
    );
    assert!(app.session.state().is_ok_and(|s| !s.is_dirty()));
    // Again, now that it has its file: still that name.
    std::fs::remove_file(dir.join("plan.dxf")).unwrap();
    app.start("qsave");
    assert!(dir.join("plan.dxf").exists());
    assert!(told.borrow().save_as.is_empty());

    // Save As: the same name, marked around its save; the page asks for the new name.
    app.start("ui.saveas");
    assert_eq!(told.borrow().save_as, [true, false], "Save As was not marked around its save");
    assert!(told.borrow().names_asked.is_empty(), "Save As asked for a name in the app");
    assert_eq!(app.session.state().map(|s| s.title.clone()).ok().as_deref(), Some("plan.dxf"));

    // Share goes to the page.
    app.start("share");
    assert_eq!(told.borrow().shares, 1);

    // IMPORT uses the browser's own picker, not the page's open picker; a picked .lin file
    // loads its linetypes into the open drawing.
    app.start("import");
    assert_eq!(told.borrow().uploads, [Upload::Import]);
    assert_eq!(told.borrow().opens, 0, "IMPORT asked the page for a document");
    app.receive_upload(Upload::Linetype, "mine.lin", b"*MINE,dashes\nA,1,-0.5\n");
    assert!(app.session.state().is_ok_and(|s| s.drawing().linetypes.iter().any(|l| l.name.eq_ignore_ascii_case("MINE"))));
    assert_eq!(app.session.docs.len(), 1);

    // File > Open still asks the page.
    app.start("open");
    assert_eq!(told.borrow().opens, 1);

    // The page's own JSON calls still run.
    let _ = app.run("new", json!({}));
    assert_eq!(app.session.docs.len(), 2);
    let _ = app.run("close", json!({ "index": 1 }));
    assert_eq!(app.session.docs.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The real window (menu bar and tool bar painted), wide enough that no tool bar button
/// overflows into "More toolbar commands", with one drawing open.
fn window(app: CadApp) -> Harness<'static, CadApp> {
    let mut app = app;
    app.run("new", json!({})).unwrap();
    app.ui.start_tab = false;
    app.ui.in_window_menu = true;
    let mut h = Harness::builder().with_size(egui::vec2(1800.0, 900.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

/// Hosted, there is no Help anywhere: no Help menu (About included) in the menu tree or the
/// painted menu bar, no Help (?) button on the tool bar; F1 and HELP, ? or ABOUT typed open
/// nothing and say help is in Clawku. Not hosted, the Help menu, About and the button stay.
#[test]
fn hosted_has_no_help_menu_button_or_command() {
    let (mut app, _) = hosted_app();
    assert!(!tree(&app).iter().any(|(m, _)| m == "Help"), "hosted menu bar has a Help menu");
    let shown = menu_ids(&app);
    for id in ["ui.dialog.about", "ui.dialog.commands", "ui.updates.check", "ui.sample"] {
        assert!(!shown.iter().any(|s| s == id), "hosted menus show {id}");
    }
    let _ = app.run("new", json!({}));
    for typed in ["help", "HELP", "?", "about", "ui.dialog.commands", "ui.dialog.about"] {
        app.session.log.clear();
        assert!(app.cmdline(typed).is_none(), "{typed} was refused as unknown");
        assert_eq!(app.ui.dialog, None, "{typed} opened a dialog");
        assert!(app.session.log.iter().any(|l| l == HELP_MSG), "{typed} said {:?}", app.session.log);
    }
    // F1 runs its menu command, which says the same.
    app.start("ui.dialog.commands");
    assert_eq!(app.ui.dialog, None, "F1 opened the Command Reference");

    let h = window(hosted_app().0);
    assert_eq!(h.query_all_by_label_contains("Help").count(), 0, "the hosted window shows Help");
    assert!(h.query_by_label("File").is_some(), "the hosted window has no menu bar");

    let standalone = CadApp::new(Session::empty(), Services::default());
    let help = tree(&standalone).into_iter().find(|(m, _)| m == "Help").map(|(_, e)| e).unwrap_or_default();
    let mut help_ids = Vec::new();
    ids(&help, &mut help_ids);
    for id in ["ui.dialog.about", "ui.dialog.commands", "ui.sample"] {
        assert!(help_ids.iter().any(|s| s == id), "standalone Help menu lost {id}");
    }
    let h = window(CadApp::new(Session::empty(), Services::default()));
    assert!(h.query_all_by_label("Help").count() >= 2, "standalone window lost the Help menu or button");
}

/// Hosted, the tool bar's first button (Open) asks the page for a document (`pick_open`, which
/// the web shell posts as `cadcraft:pick` with purpose "document"), never anything else.
#[test]
fn hosted_toolbar_open_asks_the_page() {
    let (app, told) = hosted_app();
    let mut h = window(app);
    let open = h.query_all_by_label_contains("Open (").next().expect("the hosted tool bar has no Open button");
    open.click();
    h.run_steps(3);
    assert_eq!(told.borrow().opens, 1, "Open did not ask the page");
    assert!(told.borrow().uploads.is_empty(), "Open used the browser's own picker");
    assert_eq!(h.state().session.docs.len(), 1);
}
