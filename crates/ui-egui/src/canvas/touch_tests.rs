//! Fingers on a touch screen, run as the hosts run them (`raw_input_hook`, then the UI pass), in
//! the order a browser sends them through eframe: a frame for a finger's `pointerdown` (its press
//! alone), then its `touchstart` (the press again with the touch), moves, a frame for `pointerup`
//! (the release alone), then `touchend`. A second finger's `pointerdown` and `pointerup` press
//! and release the primary button too; its touch events carry none.
//!
//! On the drawing one finger is the mouse (LINE picks points, PAN pans); two fingers pan and
//! pinch-zoom the view whatever the command and leave nothing the first began. Off the drawing a
//! finger drag scrolls the Tool Sets and the Layers/Properties palette
//! ([`crate::touch::FingerFilter`]). A mouse is left as it was.

use super::*;
use cadcraft_engine::Session;
use egui::{Event, TouchPhase};
use serde_json::json;

const SCREEN: egui::Vec2 = egui::vec2(1400.0, 900.0);

/// One headless frame of the whole window at `t` seconds, through the raw input hook.
fn frame(app: &mut CadApp, ctx: &egui::Context, t: f64, events: Vec<Event>) {
    let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), time: Some(t), events, ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

fn touch(n: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// The frames eframe's web runner makes of finger `n` (0 is the first, the primary touch).
fn finger(n: u64, phase: TouchPhase, pos: Pos2) -> Vec<Vec<Event>> {
    let primary = n == 0;
    match phase {
        TouchPhase::Start if primary => vec![vec![button(pos, true)], vec![button(pos, true), touch(n, phase, pos)]],
        TouchPhase::Start => vec![vec![button(pos, true)], vec![touch(n, phase, pos)]],
        TouchPhase::Move if primary => vec![vec![Event::PointerMoved(pos), touch(n, phase, pos)]],
        TouchPhase::Move => vec![vec![touch(n, phase, pos)]],
        TouchPhase::End | TouchPhase::Cancel if primary => {
            vec![vec![button(pos, false)], vec![button(pos, false), Event::PointerGone, touch(n, phase, pos)]]
        }
        TouchPhase::End | TouchPhase::Cancel => vec![vec![button(pos, false)], vec![touch(n, phase, pos)]],
    }
}

/// Runs `frames` one after another, 1/60 s apart, from `t`.
fn play(app: &mut CadApp, ctx: &egui::Context, t: &mut f64, frames: Vec<Vec<Event>>) {
    for events in frames {
        *t += 1.0 / 60.0;
        frame(app, ctx, *t, events);
    }
}

/// A drawing with one line across the middle of the view, drawn twice; the clock after it.
/// `coarse`: the device said its main pointer is a finger (`(pointer: coarse)`).
fn app_with(ctx: &egui::Context, coarse: bool) -> (CadApp, f64) {
    let mut app = CadApp::new(Session::new(), crate::Services::default());
    if coarse {
        app.expect_touch();
    }
    app.session.execute("line", &json!({"points": [[0, 0], [100, 0]]})).unwrap();
    app.canvas.zoom_pending = true;
    frame(&mut app, ctx, 0.0, vec![]);
    frame(&mut app, ctx, 0.1, vec![]);
    (app, 0.1)
}

fn entities(app: &CadApp) -> usize {
    app.session.doc().unwrap().model.iter().count()
}

fn view(app: &CadApp) -> cadcraft_engine::View {
    app.session.state().unwrap().view()
}

fn prompt(app: &CadApp) -> Option<String> {
    app.session.current_prompt().map(|p| p.message.clone())
}

/// Two fingers come down `gap` apart about `c`: the first presses and drags (40, 30), far enough
/// for a selection window or a rubber band, then the second lands. Each of `steps` moves them to
/// (centre, gap); then the second lifts, then the first.
fn two_fingers(app: &mut CadApp, ctx: &egui::Context, t: &mut f64, c: Pos2, gap: f32, steps: &[(Pos2, f32)]) {
    let off = egui::vec2(40.0, 30.0);
    let a = c - egui::vec2(gap / 2.0, 0.0) - off;
    play(app, ctx, t, finger(0, TouchPhase::Start, a));
    play(app, ctx, t, finger(0, TouchPhase::Move, a + off * 0.5));
    play(app, ctx, t, finger(0, TouchPhase::Move, a + off));
    let (mut a, mut b) = (a + off, c + egui::vec2(gap / 2.0, 0.0));
    play(app, ctx, t, finger(1, TouchPhase::Start, b));
    for &(at, gap) in steps {
        a = at - egui::vec2(gap / 2.0, 0.0);
        b = at + egui::vec2(gap / 2.0, 0.0);
        let mut events = finger(0, TouchPhase::Move, a).remove(0);
        events.push(touch(1, TouchPhase::Move, b));
        play(app, ctx, t, vec![events]);
    }
    play(app, ctx, t, finger(1, TouchPhase::End, b));
    play(app, ctx, t, finger(0, TouchPhase::End, a));
    play(app, ctx, t, vec![vec![], vec![]]);
}

/// Two fingers dragging together pan the view with them, and what the first finger began is
/// dropped: idle, no selection window is left open and nothing is selected; in LINE, no point is
/// placed. Also on a device that didn't say it has a touch screen, whose first press reaches egui
/// as a mouse's.
#[test]
fn a_two_finger_drag_pans_and_leaves_no_point_or_selection_window() {
    for coarse in [true, false] {
        for line in [false, true] {
            let ctx = egui::Context::default();
            let (mut app, mut t) = app_with(&ctx, coarse);
            if line {
                app.start("line");
            }
            let first = prompt(&app);
            let rect = app.canvas.rect.unwrap();
            let c = rect.center();
            let (before, xf) = (view(&app), app.canvas.xf.unwrap());
            let steps: Vec<_> = (1..=6).map(|i| (c + egui::vec2(8.0, 5.0) * i as f32, 100.0)).collect();
            two_fingers(&mut app, &ctx, &mut t, c, 100.0, &steps);
            let after = view(&app);
            let case = format!("coarse {coarse}, line {line}");
            assert!(app.session.pending_window.is_none(), "no selection window ({case})");
            assert!(app.session.selection().is_empty(), "nothing selected ({case})");
            assert_eq!(entities(&app), 1, "nothing drawn ({case})");
            assert_eq!(prompt(&app), first, "no point placed ({case})");
            assert!((after.height - before.height).abs() < 1e-9 * before.height, "no zoom ({case}): {} -> {}", before.height, after.height);
            // The content followed the fingers' centre: 48 right and 30 down on screen.
            let want = xf.screen_delta_to_world(egui::vec2(48.0, 30.0));
            let moved = before.center - after.center;
            assert!((moved - want).len() < 2.0 / xf.scale, "panned {moved:?}, want {want:?} ({case})");
        }
    }
}

/// Fingers spreading apart zoom about the point between them (not about the first finger, which
/// is the pointer): the drawing point under that centre stays under it.
#[test]
fn a_pinch_zooms_about_the_point_between_the_fingers() {
    let ctx = egui::Context::default();
    let (mut app, mut t) = app_with(&ctx, true);
    let c = app.canvas.rect.unwrap().center() + egui::vec2(-120.0, 60.0);
    let (h0, xf) = (view(&app).height, app.canvas.xf.unwrap());
    let under = xf.to_world(c);
    let steps: Vec<_> = (1..=5).map(|i| (c, 100.0 + 20.0 * i as f32)).collect();
    two_fingers(&mut app, &ctx, &mut t, c, 100.0, &steps);
    let h = view(&app).height;
    assert!((h0 / h - 2.0).abs() < 0.05, "the fingers twice as far apart zoom in twice: {h0} -> {h}");
    let now = app.canvas.xf.unwrap().to_screen(under);
    assert!((now - c).length() < 2.0, "the point between the fingers stays put: {now:?} vs {c:?}");
    assert!(app.session.pending_window.is_none() && app.session.selection().is_empty());
}

/// A finger tap on the drawing at `at`.
fn tap(app: &mut CadApp, ctx: &egui::Context, t: &mut f64, at: Pos2) {
    play(app, ctx, t, finger(0, TouchPhase::Start, at));
    play(app, ctx, t, finger(0, TouchPhase::End, at));
    play(app, ctx, t, vec![vec![]]);
    // Past the double-click time: the next tap is a click of its own.
    *t += 1.0;
}

/// One finger still picks LINE's points where it taps: two taps draw a segment between them.
#[test]
fn one_finger_picks_line_points() {
    for coarse in [true, false] {
        let ctx = egui::Context::default();
        let (mut app, mut t) = app_with(&ctx, coarse);
        app.start("line");
        let rect = app.canvas.rect.unwrap();
        let (a, b) = (rect.center() + egui::vec2(-150.0, -120.0), rect.center() + egui::vec2(100.0, 50.0));
        let xf = app.canvas.xf.unwrap();
        tap(&mut app, &ctx, &mut t, a);
        assert_ne!(prompt(&app).as_deref(), Some("Specify first point"), "the first point is placed (coarse {coarse})");
        tap(&mut app, &ctx, &mut t, b);
        assert_eq!(entities(&app), 2, "a segment drawn (coarse {coarse})");
        let doc = app.session.doc().unwrap();
        let seg = doc.model.iter().find_map(|e| match &e.kind {
            cadcraft_engine::doc::EntityKind::Line(l) if l.a.x != 0.0 || l.b.x != 100.0 => Some(l.clone()),
            _ => None,
        });
        let seg = seg.expect("the new line");
        let (wa, wb) = (xf.to_world(a), xf.to_world(b));
        let tol = 3.0 / xf.scale;
        assert!((seg.a.x - wa.x).abs() < tol && (seg.a.y - wa.y).abs() < tol, "{seg:?} from {wa:?}");
        assert!((seg.b.x - wb.x).abs() < tol && (seg.b.y - wb.y).abs() < tol, "{seg:?} to {wb:?}");
    }
}

/// While PAN runs one finger drags the view with it and places none of PAN's points.
#[test]
fn one_finger_pans_while_pan_runs() {
    let ctx = egui::Context::default();
    let (mut app, mut t) = app_with(&ctx, true);
    app.start("pan");
    let first = prompt(&app);
    let (before, xf) = (view(&app), app.canvas.xf.unwrap());
    let c = app.canvas.rect.unwrap().center();
    play(&mut app, &ctx, &mut t, finger(0, TouchPhase::Start, c));
    for i in 1..=6 {
        play(&mut app, &ctx, &mut t, finger(0, TouchPhase::Move, c + egui::vec2(10.0, 5.0) * i as f32));
    }
    play(&mut app, &ctx, &mut t, finger(0, TouchPhase::End, c + egui::vec2(60.0, 30.0)));
    play(&mut app, &ctx, &mut t, vec![vec![]]);
    assert_eq!(prompt(&app), first, "no point placed: PAN still asks for its base point");
    let moved = before.center - view(&app).center;
    let want = xf.screen_delta_to_world(egui::vec2(60.0, 30.0));
    assert!((moved - want).len() < 2.0 / xf.scale, "the content followed the finger: {moved:?}, want {want:?}");
}

/// A mouse drag on the drawing still opens and closes a selection window (PICKDRAG) and selects
/// the line it crosses, and a mouse's events pass the finger filter exactly as they came.
#[test]
fn a_mouse_drag_still_selects() {
    let ctx = egui::Context::default();
    let (mut app, mut t) = app_with(&ctx, false);
    // Right to left over the line's middle: a crossing window.
    let c = app.canvas.rect.unwrap().center();
    let (a, b) = (c + egui::vec2(60.0, -40.0), c + egui::vec2(-60.0, 40.0));
    let steps = [
        vec![Event::PointerMoved(a)],
        vec![button(a, true)],
        vec![Event::PointerMoved(a + (b - a) * 0.5)],
        vec![Event::PointerMoved(b)],
        vec![button(b, false)],
        vec![],
    ];
    for events in steps {
        let mut probe = crate::touch::FingerFilter::default();
        let mut raw = egui::RawInput { events: events.clone(), time: Some(t), ..Default::default() };
        probe.filter(&mut raw, |_| false, || crate::touch::Under::List);
        assert_eq!(raw.events, events, "the finger filter leaves a mouse alone");
        t += 1.0 / 60.0;
        frame(&mut app, &ctx, t, events);
    }
    assert_eq!(app.session.selection().len(), 1, "the crossing window selected the line");
    assert!(app.session.pending_window.is_none());
}

/// Every text the frame drew, with where, once each (texts drawn twice are left out).
fn texts(out: &egui::FullOutput) -> std::collections::HashMap<String, Pos2> {
    fn walk(shape: &egui::Shape, seen: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(t) => seen.push((t.galley.text().to_string(), t.pos)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, seen)),
            _ => {}
        }
    }
    let mut seen = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, &mut seen);
    }
    let mut once = std::collections::HashMap::new();
    for (text, pos) in &seen {
        if seen.iter().filter(|(t, _)| t == text).count() == 1 {
            once.insert(text.clone(), *pos);
        }
    }
    once
}

/// How far up the texts drawn in both frames went (the median, points).
fn rose(before: &std::collections::HashMap<String, Pos2>, after: &std::collections::HashMap<String, Pos2>) -> f32 {
    let mut d: Vec<f32> = before.iter().filter_map(|(k, p)| after.get(k).map(|q| p.y - q.y)).collect();
    assert!(!d.is_empty(), "texts in both frames");
    d.sort_by(f32::total_cmp);
    d[d.len() / 2]
}

/// A palette alone on a small window (it overflows), run as the app runs it: the raw input hook
/// (the finger filter), the finger's scroll handed to egui, the palette.
fn palette(app: &mut CadApp, ctx: &egui::Context, t: &mut f64, frames: Vec<Vec<Event>>, show: fn(&mut CadApp, &mut egui::Ui)) -> egui::FullOutput {
    let mut last = None;
    for events in frames {
        *t += 1.0 / 60.0;
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 360.0));
        let mut raw = egui::RawInput { screen_rect: Some(screen), time: Some(*t), events, ..Default::default() };
        app.raw_input_hook(ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            app.finger_scroll(ui.ctx());
            show(app, ui);
        });
        out.textures_delta.clear();
        last = Some(out);
    }
    last.expect("a frame")
}

/// A finger dragging up the Tool Sets and up the Layers/Properties palette scrolls them, 1:1, and
/// runs no tool or button; a tap on the Modeling tab then still switches to it. Also when the
/// device didn't say it has a touch screen, and the finger's first press reached egui as a
/// mouse's.
#[test]
fn a_finger_drag_scrolls_the_tool_sets_and_the_layers_palette() {
    let shows: [(&str, fn(&mut CadApp, &mut egui::Ui)); 2] = [
        ("tool sets", |app, ui| {
            egui::CentralPanel::default().show(ui, |ui| crate::palettes::toolsets(app, ui));
        }),
        ("palettes", |app, ui| crate::palettes::right_palettes(app, ui)),
    ];
    for coarse in [true, false] {
        for (name, show) in shows {
            let mut app = CadApp::new(Session::new(), crate::Services::default());
            if coarse {
                app.expect_touch();
            }
            for i in 0..30 {
                app.session.execute("layer.new", &json!({"name": format!("Layer {i}")})).ok();
            }
            app.ui.show_layer_list = true;
            app.ui.show_palettes = true;
            let ctx = egui::Context::default();
            let mut t = 0.0;
            let (entities0, running0, tab0) = (entities(&app), app.session.running.is_some(), app.ui.toolset_tab.clone());
            // A touch screen seen before (egui gives a scroll area its drag-to-scroll then).
            palette(&mut app, &ctx, &mut t, vec![vec![touch(9, TouchPhase::Cancel, egui::pos2(-10.0, -10.0))]], show);
            let before = texts(&palette(&mut app, &ctx, &mut t, vec![vec![]], show));
            let p = egui::pos2(150.0, 300.0);
            palette(&mut app, &ctx, &mut t, finger(0, TouchPhase::Start, p), show);
            let mut held = Default::default();
            for i in 1..=10 {
                held = texts(&palette(&mut app, &ctx, &mut t, finger(0, TouchPhase::Move, p - egui::vec2(0.0, 12.0 * i as f32)), show));
            }
            let case = format!("{name}, coarse {coarse}");
            let up = rose(&before, &held);
            // 1:1 (the palette's rows), or a frame's motion behind where egui's own drag-to-scroll
            // takes the drag (over the Tool Sets' buttons: its press reaches egui a frame late).
            assert!((120.0 - 13.0..=122.0).contains(&up), "the list follows the finger: {up} ({case})");
            palette(&mut app, &ctx, &mut t, finger(0, TouchPhase::End, p - egui::vec2(0.0, 120.0)), show);
            let after = texts(&palette(&mut app, &ctx, &mut t, vec![vec![], vec![]], show));
            // egui's own drag-to-scroll (over buttons) glides on a little after the lift.
            let glide = rose(&before, &after);
            assert!(glide >= up - 1.0, "and doesn't spring back: {up} -> {glide} ({case})");
            assert_eq!(entities(&app), entities0, "{case}");
            assert_eq!(app.session.running.is_some(), running0, "no tool started ({case})");
            assert_eq!(app.ui.toolset_tab, tab0, "{case}");
            assert!(!ctx.input(|i| i.pointer.primary_down()), "egui holds no button ({case})");
        }
    }
    // A tap on the Modeling tab still switches to it.
    let mut app = CadApp::new(Session::new(), crate::Services::default());
    app.expect_touch();
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let show = shows[0].1;
    palette(&mut app, &ctx, &mut t, vec![vec![], vec![]], show);
    let tab = texts(&palette(&mut app, &ctx, &mut t, vec![vec![]], show))["Modeling"] + egui::vec2(10.0, 2.0);
    palette(&mut app, &ctx, &mut t, finger(0, TouchPhase::Start, tab), show);
    palette(&mut app, &ctx, &mut t, finger(0, TouchPhase::End, tab), show);
    palette(&mut app, &ctx, &mut t, vec![vec![]], show);
    assert_eq!(app.ui.toolset_tab, "Modeling", "a tap clicks the tab");
}

/// The drawing's gesture tracker: a second finger on the drawing makes a gesture, which lasts
/// through the frame the last finger lifts; one that began off the drawing is not the drawing's.
#[test]
fn canvas_fingers_follow_a_gesture() {
    let mut f = crate::touch::CanvasFingers::default();
    let on = |p: Pos2| p.x < 500.0;
    let p = egui::pos2(100.0, 100.0);
    let feed = |f: &mut crate::touch::CanvasFingers, events: Vec<Event>| f.feed(&events, on);
    assert_eq!(feed(&mut f, vec![touch(1, TouchPhase::Start, p)]), crate::touch::FingerFrame { gesture: false, began: false });
    assert!(f.finger_down() && !f.pinching());
    assert_eq!(feed(&mut f, vec![touch(2, TouchPhase::Start, p)]), crate::touch::FingerFrame { gesture: true, began: true });
    assert!(f.pinching());
    assert!(feed(&mut f, vec![touch(2, TouchPhase::End, p)]).gesture, "one finger still down");
    assert_eq!(feed(&mut f, vec![touch(1, TouchPhase::End, p)]), crate::touch::FingerFrame { gesture: true, began: false });
    assert!(!feed(&mut f, vec![]).gesture, "over");
    // Begun off the drawing (a panel): not the drawing's.
    let off = egui::pos2(700.0, 100.0);
    feed(&mut f, vec![touch(3, TouchPhase::Start, off)]);
    assert!(!feed(&mut f, vec![touch(4, TouchPhase::Start, p)]).gesture);
    assert!(!f.pinching() && !f.finger_down());
}
