//! Fingers on a touch screen (a phone or tablet: ClawCAD inside Clawku).
//!
//! egui hands every finger's contact over as [`egui::Event::Touch`] and makes the first finger
//! the mouse pointer too (emulated primary-button presses, moves and releases). Left alone, a
//! finger dragged over a panel's list drags its row and one in a text field selects, and nothing
//! scrolls; two fingers on the drawing place a point or open a selection window when the first
//! one lifts.
//!
//! - On the drawing one finger is the mouse: it picks points, opens selection windows, clicks
//!   grips, as the current command does (and pans while PAN runs, `canvas::finger_pan`). Two
//!   fingers pan the view and pinch-zoom it about the point between them, whatever the command,
//!   and drop what the first finger began: its press places no point, opens no window and
//!   clicks nothing ([`CanvasFingers`]).
//! - Off the drawing, [`FingerFilter`] lets a finger scroll panels, lists and text fields (Tool
//!   Sets, Layers, Properties, the command history, dialogs): see its docs.
//!
//! Mouse, touchpad, wheel and keyboard input pass unchanged.
//!
//! (The finger filter of ClawVector, `vectorcraft-ui-egui::touch`.)

use egui::{Event, Pos2, TouchId, TouchPhase, Vec2};

/// Finger travel (points) that makes a press off the canvas a scroll rather than a tap or a long
/// press.
pub const SLOP: f32 = 8.0;
/// Seconds a finger rests off the canvas before its press goes through (a drag then selects text
/// or moves a row, as on iOS).
pub const LONG_PRESS: f64 = 0.45;

/// What egui's last pass found under a finger that came down off the canvas ([`under_finger`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Under {
    /// A text field: a drag scrolls the scroll area around it, a tap focuses, a long press then a
    /// drag selects.
    Field,
    /// A widget that takes drags (a Layers row, a swatch, a slider) in a list that scrolls: an
    /// up-or-down drag scrolls the list, a sideways one goes to the widget, a long press then a
    /// drag goes to the widget too.
    List,
    /// Anything else (a button, a menu item, a list's background, a scroll bar): egui's own
    /// handling, whose scroll areas already scroll when dragged by finger.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// Down off the canvas, not yet moved past [`SLOP`] or held for [`LONG_PRESS`].
    Pending,
    /// Dragging off the canvas: the scroll area under the finger scrolls.
    Scrolling,
    /// Held, then the press went through: egui sees a drag.
    Held,
    /// A second finger came down on a gesture of this filter.
    Multi,
    /// On the canvas, or egui's own handling: the events pass as they came.
    Native,
}

impl Phase {
    /// The emulated pointer press/release of this phase is held back.
    fn owned(self) -> bool {
        matches!(self, Phase::Pending | Phase::Scrolling | Phase::Held | Phase::Multi)
    }
}

/// Fingers off the canvas on a touch screen (phones and tablets): panels, lists, menus, dialogs
/// and text fields scroll with a one-finger drag.
///
/// A touch arrives as [`egui::Event::Touch`] plus emulated primary-button pointer events, so left
/// alone a finger dragging a Layers row reorders it and one dragging in a text field selects.
/// [`FingerFilter::filter`] runs in the raw input hook, before egui sees the frame, and holds the
/// emulated press of a finger that comes down off the canvas back for a frame, while egui finds
/// what is under it ([`Under`]):
/// - [`Under::Other`]: the press goes through a frame late, at the place the finger came down;
/// - [`Under::Field`] and [`Under::List`]: a drag past [`SLOP`] scrolls ([`Self::take_scroll`]),
///   1:1 with the finger, and never presses; a tap is a click where the finger came down; a rest of
///   [`LONG_PRESS`] lets the press through, so a drag then selects or moves; a sideways drag on a
///   [`Under::List`] widget goes to it at once (a slider, a scrub field);
/// - a second finger ends what the first began.
///
/// A finger coming down on the canvas passes untouched: one finger is the current command there. A
/// mouse, a touchpad and the wheel pass as they came too.
///
/// A browser sends a finger's `pointerdown` (and eframe runs a frame for it) before its
/// `touchstart`, which carries the press again with the touch. On a touch screen
/// ([`Self::expect_touch`], or once a finger has been seen) a primary press alone in its frame
/// waits a frame: dropped if a finger comes down then (its own press comes with it), else given to
/// egui (a mouse's). A press that went through before a finger was ever seen is released where it
/// was when a finger off the canvas takes over, so egui isn't left holding a button.
#[derive(Debug, Default)]
pub struct FingerFilter {
    phase: Phase,
    /// None while [`Phase::Pending`] waits a frame to see what egui found under the finger.
    under: Option<Under>,
    /// The finger that started the gesture.
    finger: Option<TouchId>,
    down: Vec<TouchId>,
    start: Pos2,
    start_time: f64,
    last: Pos2,
    frame_time: Option<f64>,
    /// This frame's scroll, in the wheel's convention (positive = content moves down/right).
    scroll: Vec2,
    /// Presses may be a finger's: the device's main pointer is coarse ([`Self::expect_touch`]) or
    /// a finger has been seen.
    expect_touch: bool,
    /// A primary press from a frame with no finger in it, held for a frame (see the type's docs).
    held: Option<(Pos2, egui::Modifiers)>,
    /// Where a primary press went through to egui with no release since.
    passed_down: Option<Pos2>,
}

impl FingerFilter {
    /// Rewrites one frame's raw input (see the type's docs). `on_canvas`: a finger coming down
    /// there is the tool's. `under`: what egui's last pass found under the pointer (asked the frame
    /// after a finger came down off the canvas, where the pointer was moved to it).
    pub fn filter(&mut self, raw: &mut egui::RawInput, on_canvas: impl Fn(Pos2) -> bool, under: impl FnOnce() -> Under) {
        let now = raw.time.or(self.frame_time.map(|t| t + 1.0 / 60.0)).unwrap_or(0.0);
        self.frame_time = Some(now);
        self.scroll = Vec2::ZERO;
        let touched = raw.events.iter().any(|e| matches!(e, Event::Touch { .. }));
        self.expect_touch |= touched;
        let finger_down = raw.events.iter().any(|e| matches!(e, Event::Touch { phase: TouchPhase::Start, .. }));
        // Last frame's held press: dropped for the finger it was, else a mouse's, given now.
        let mut replayed = false;
        if let Some((pos, modifiers)) = self.held.take()
            && !finger_down
        {
            let press = Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers };
            raw.events.splice(0..0, [Event::PointerMoved(pos), press]);
            replayed = true;
        }
        // On a touch screen a primary press alone in its frame (no touch, no release) waits a frame
        // for its finger.
        let primary = |events: &[Event], pressed: bool| {
            events.iter().position(|e| matches!(e, Event::PointerButton { button: egui::PointerButton::Primary, pressed: p, .. } if *p == pressed))
        };
        if self.expect_touch
            && !touched
            && !replayed
            && let Some(i) = primary(&raw.events, true)
            && primary(&raw.events, false).is_none()
            && let Some(Event::PointerButton { pos, modifiers, .. }) = raw.events.get(i).cloned()
            && pos.is_finite()
        {
            raw.events.remove(i);
            self.held = Some((pos, modifiers));
        }
        if !touched && self.phase == Phase::Idle {
            // Mouse, touchpad, wheel and keyboard frames: as they came.
            self.note_passed(&raw.events);
            return;
        }
        // The keys held with the finger, as the emulated pointer events carry them.
        let mods =
            raw.events.iter().find_map(|e| if let Event::PointerButton { modifiers, .. } = e { Some(*modifiers) } else { None }).unwrap_or_default();
        let button = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods };
        let mut pre: Vec<Event> = Vec::new();
        let mut post: Vec<Event> = Vec::new();
        // Last frame moved the pointer under the finger: anything but a field or a list gets its
        // press now (a frame late) and egui's own touch handling.
        if self.phase == Phase::Pending && self.under.is_none() {
            match under() {
                Under::Other => {
                    self.phase = Phase::Native;
                    pre.extend([Event::PointerMoved(self.start), button(self.start, true)]);
                }
                u => self.under = Some(u),
            }
        }
        if self.phase == Phase::Pending && self.under.is_some() && now - self.start_time >= LONG_PRESS {
            self.phase = Phase::Held;
            pre.extend([Event::PointerMoved(self.start), button(self.start, true)]);
        }
        // A finger that came down and lifted within this frame is still this frame's to rewrite.
        let mut owned = self.phase.owned();
        for event in &raw.events {
            let Event::Touch { id, phase, pos, .. } = *event else { continue };
            if !pos.is_finite() {
                continue;
            }
            match phase {
                TouchPhase::Start => {
                    if !self.down.contains(&id) {
                        self.down.push(id);
                    }
                    if self.phase == Phase::Idle {
                        self.finger = Some(id);
                        (self.start, self.last, self.start_time) = (pos, pos, now);
                        self.under = None;
                        if on_canvas(pos) {
                            self.phase = Phase::Native;
                        } else {
                            // A press that went through before egui knew it was a finger: let go
                            // of it where it was, so the finger's drag can't move or select from it.
                            if let Some(at) = self.passed_down.take() {
                                pre.push(button(at, false));
                            }
                            self.phase = Phase::Pending;
                            owned = true;
                            pre.push(Event::PointerMoved(pos));
                        }
                    } else if self.down.len() >= 2 && self.phase.owned() {
                        if self.phase == Phase::Held {
                            post.push(button(self.last, false));
                        }
                        self.phase = Phase::Multi;
                    }
                }
                TouchPhase::Move if Some(id) == self.finger => match self.phase {
                    Phase::Pending if (pos - self.start).length() > SLOP => {
                        let d = pos - self.start;
                        match self.under {
                            // Sideways on a list's widget: the widget's drag, from where it began.
                            Some(Under::List) if d.x.abs() > d.y.abs() => {
                                self.phase = Phase::Native;
                                pre.extend([Event::PointerMoved(self.start), button(self.start, true)]);
                            }
                            Some(_) => {
                                self.phase = Phase::Scrolling;
                                self.scroll += d;
                            }
                            // Not known yet: the next frame decides, from where the finger is then.
                            None => {}
                        }
                        self.last = pos;
                    }
                    Phase::Scrolling => {
                        self.scroll += pos - self.last;
                        self.last = pos;
                    }
                    _ => self.last = pos,
                },
                TouchPhase::Move => {}
                TouchPhase::End | TouchPhase::Cancel => {
                    self.down.retain(|d| *d != id);
                    if Some(id) == self.finger {
                        self.lift(phase == TouchPhase::End, pos, &mut post, &button);
                    }
                }
            }
        }
        // While it scrolls, the pointer stays where the finger came down: the list under it keeps
        // scrolling however far the finger goes.
        let still = matches!(self.phase, Phase::Scrolling | Phase::Multi);
        let mut gone = false;
        let mut out: Vec<Event> = Vec::with_capacity(raw.events.len());
        for event in raw.events.drain(..) {
            match &event {
                Event::PointerButton { button: egui::PointerButton::Primary, .. } if owned => {}
                Event::PointerGone if owned => gone = true,
                Event::PointerMoved(_) if owned && still => {}
                _ => out.push(event),
            }
        }
        if self.down.is_empty() {
            if gone {
                post.push(Event::PointerGone);
            }
            self.phase = Phase::Idle;
            self.finger = None;
        }
        pre.append(&mut out);
        pre.append(&mut post);
        self.note_passed(&pre);
        raw.events = pre;
    }

    /// Keeps [`Self::passed_down`] up to date with the primary presses and releases going to egui.
    fn note_passed(&mut self, events: &[Event]) {
        for e in events {
            if let Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } = e {
                self.passed_down = pressed.then_some(*pos);
            }
        }
    }

    /// The device's main pointer is a finger (a phone or tablet: `(pointer: coarse)` on the web):
    /// even its first press waits until its touch says what it is.
    pub fn expect_touch(&mut self) {
        self.expect_touch = true;
    }

    /// The gesture's finger lifted (`ended`; else the system cancelled it) at `pos`.
    fn lift(&mut self, ended: bool, pos: Pos2, post: &mut Vec<Event>, button: &impl Fn(Pos2, bool) -> Event) {
        match self.phase {
            // A tap: a click where the finger came down.
            Phase::Pending if ended => post.extend([Event::PointerMoved(self.start), button(self.start, true), button(self.start, false)]),
            Phase::Held => post.push(button(if pos.is_finite() { pos } else { self.last }, false)),
            _ => {}
        }
        self.finger = None;
        if !matches!(self.phase, Phase::Multi | Phase::Native) {
            self.phase = if self.down.is_empty() { Phase::Idle } else { Phase::Multi };
        }
    }

    /// This frame's finger scroll (the wheel's convention), taken once: it goes into egui's
    /// `smooth_scroll_delta`, which the scroll area under the pointer takes.
    pub fn take_scroll(&mut self) -> Vec2 {
        let d = std::mem::take(&mut self.scroll);
        if d.is_finite() { d } else { Vec2::ZERO }
    }

    /// How soon the app must run again with no new input: a resting finger becomes a long press,
    /// or what is under a finger is decided on the next frame.
    pub fn repaint_after(&self) -> Option<std::time::Duration> {
        if self.held.is_some() {
            return Some(std::time::Duration::ZERO);
        }
        match self.phase {
            Phase::Pending if self.under.is_none() => Some(std::time::Duration::ZERO),
            Phase::Pending => {
                let left = LONG_PRESS - (self.frame_time.unwrap_or(self.start_time) - self.start_time);
                Some(std::time::Duration::from_secs_f64(left.clamp(0.0, LONG_PRESS)))
            }
            _ => None,
        }
    }
}

/// What egui's last pass found under the pointer ([`Under`]): a text field among the widgets it
/// marked hovered (one keeps a text edit's state); else a widget that takes drags inside a scroll
/// area that scrolls by drag (on a touch screen, egui gives such an area a drag-only widget over
/// its contents, below what it holds, while its contents overflow it); else anything.
pub fn under_finger(ctx: &egui::Context) -> Under {
    let (hovered, contains): (Vec<egui::Id>, Vec<egui::Id>) =
        ctx.interaction_snapshot(|s| (s.hovered.iter().copied().collect(), s.contains_pointer.iter().copied().collect()));
    if hovered.iter().any(|id| egui::TextEdit::load_state(ctx, *id).is_some()) {
        return Under::Field;
    }
    let Some(top) = hovered.iter().filter_map(|id| ctx.read_response(*id)).find(|r| r.sense.senses_drag() && r.sense != egui::Sense::DRAG) else {
        return Under::Other;
    };
    // The scroll area's own widget, on the same layer; not the layer's (a window that moves when
    // dragged by its body).
    let list = contains
        .iter()
        .filter(|id| **id != top.id)
        .filter_map(|id| ctx.read_response(*id))
        .any(|r| r.sense == egui::Sense::DRAG && r.layer_id == top.layer_id && r.id != r.layer_id.id);
    if list { Under::List } else { Under::Other }
}

/// The fingers on the drawing and the gesture they make, fed each frame's touches by
/// `canvas::show`.
///
/// A second finger joining one that came down on the drawing makes a two-finger gesture: it pans
/// and pinch-zooms the view (`canvas::handle_scroll_zoom`), and until every finger has lifted
/// nothing the first finger's press began counts: no point is placed, no selection window opened
/// or closed, no grip, tile or object clicked. The emulated pointer is still pressed meanwhile (the
/// browser releases it at each finger's `pointerup`), so this is what tells the canvas to look
/// away.
#[derive(Clone, Debug, Default)]
pub struct CanvasFingers {
    down: Vec<TouchId>,
    /// The gesture's first finger came down on the drawing.
    mine: bool,
    /// A second finger came down in this gesture.
    gesture: bool,
}

/// What a frame's touches made of the drawing's gesture ([`CanvasFingers::feed`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FingerFrame {
    /// A two-finger gesture is on (or ended) in this frame: the pointer's clicks and drags don't
    /// count, the fingers pan and zoom.
    pub gesture: bool,
    /// It began in this frame: what the first finger began is dropped.
    pub began: bool,
}

impl CanvasFingers {
    /// Follow this frame's touch events; `on_canvas`: a point is on the drawing (where a gesture
    /// must begin to be the drawing's).
    pub fn feed(&mut self, events: &[Event], on_canvas: impl Fn(Pos2) -> bool) -> FingerFrame {
        let mut began = false;
        let mut active = self.gesture;
        for event in events {
            let Event::Touch { id, phase, pos, .. } = *event else { continue };
            match phase {
                TouchPhase::Start => {
                    if self.down.is_empty() {
                        self.mine = pos.is_finite() && on_canvas(pos);
                        self.gesture = false;
                    }
                    if !self.down.contains(&id) {
                        self.down.push(id);
                    }
                    if self.down.len() >= 2 && self.mine && !self.gesture {
                        self.gesture = true;
                        began = true;
                    }
                    active |= self.gesture;
                }
                TouchPhase::Move => {}
                TouchPhase::End | TouchPhase::Cancel => self.down.retain(|d| *d != id),
            }
        }
        // The frame the last finger lifts still belongs to the gesture (its release is in it).
        if self.down.is_empty() {
            self.gesture = false;
        }
        FingerFrame { gesture: active, began }
    }

    /// The drawing's two-finger gesture is on: the fingers pan and zoom the view.
    pub fn pinching(&self) -> bool {
        self.gesture
    }

    /// A finger is down on the drawing (a finger, not the mouse, drives the pointer there).
    pub fn finger_down(&self) -> bool {
        self.mine && !self.down.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// One frame at `t` through the filter, off the canvas, over `under`.
    fn run(f: &mut FingerFilter, t: f64, under: Under, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, time: Some(t), ..Default::default() };
        f.filter(&mut raw, |_| false, || under);
        raw.events
    }

    fn presses(events: &[Event]) -> Vec<bool> {
        events.iter().filter_map(|e| if let Event::PointerButton { pressed, .. } = e { Some(*pressed) } else { None }).collect()
    }

    #[test]
    fn on_a_list_a_drag_scrolls_a_tap_clicks_and_sideways_goes_to_the_widget() {
        let p = Pos2::new(100.0, 300.0);
        let mut f = FingerFilter::default();
        assert!(presses(&run(&mut f, 0.0, Under::List, vec![contact(1, TouchPhase::Start, p), press(p, true)])).is_empty(), "the press waits");
        assert!(f.repaint_after().is_some());
        let mut total = Vec2::ZERO;
        for i in 1..=5 {
            let at = p - egui::vec2(1.0, 20.0 * i as f32);
            let out = run(&mut f, f64::from(i) / 60.0, Under::List, vec![contact(1, TouchPhase::Move, at), Event::PointerMoved(at)]);
            assert!(presses(&out).is_empty() && !out.contains(&Event::PointerMoved(at)), "no press, the pointer stays: {out:?}");
            total += f.take_scroll();
        }
        assert_eq!(total, egui::vec2(-1.0, -100.0), "1:1 with the finger");
        let end = p - egui::vec2(1.0, 100.0);
        let out = run(&mut f, 0.2, Under::List, vec![contact(1, TouchPhase::End, end), press(end, false), Event::PointerGone]);
        assert_eq!((presses(&out), out.last()), (vec![], Some(&Event::PointerGone)));
        assert!(f.repaint_after().is_none());

        // A tap: a click where the finger came down.
        run(&mut f, 1.0, Under::List, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        let q = p + egui::vec2(3.0, 2.0);
        run(&mut f, 1.02, Under::List, vec![contact(1, TouchPhase::Move, q), Event::PointerMoved(q)]);
        let out = run(&mut f, 1.05, Under::List, vec![contact(1, TouchPhase::End, q), press(q, false), Event::PointerGone]);
        assert_eq!(presses(&out), [true, false]);
        assert!(out.iter().any(|e| matches!(e, Event::PointerButton { pos, .. } if *pos == p)));
        assert_eq!(f.take_scroll(), Vec2::ZERO);

        // Sideways (a slider, a scrub field): the widget's drag, pressed where the finger landed.
        run(&mut f, 2.0, Under::List, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        run(&mut f, 2.02, Under::List, vec![]);
        let r = p + egui::vec2(30.0, 4.0);
        let out = run(&mut f, 2.04, Under::List, vec![contact(1, TouchPhase::Move, r), Event::PointerMoved(r)]);
        assert_eq!(presses(&out), [true]);
        assert_eq!(out.first(), Some(&Event::PointerMoved(p)));
        assert!(out.contains(&Event::PointerMoved(r)), "and the drag reaches egui");
        assert_eq!(f.take_scroll(), Vec2::ZERO);
        let out = run(&mut f, 2.06, Under::List, vec![contact(1, TouchPhase::End, r), press(r, false), Event::PointerGone]);
        assert_eq!(presses(&out), [false]);
    }

    #[test]
    fn a_long_press_lets_the_press_through_and_a_second_finger_ends_it() {
        let p = Pos2::new(50.0, 60.0);
        let mut f = FingerFilter::default();
        run(&mut f, 0.0, Under::Field, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        assert!(presses(&run(&mut f, 0.3, Under::Field, vec![])).is_empty());
        assert_eq!(presses(&run(&mut f, 0.5, Under::Field, vec![])), [true], "held: the press goes through");
        let q = p + egui::vec2(0.0, 80.0);
        let out = run(&mut f, 0.55, Under::Field, vec![contact(1, TouchPhase::Move, q), Event::PointerMoved(q)]);
        assert!(out.contains(&Event::PointerMoved(q)), "the drag reaches egui (it selects)");
        assert_eq!(f.take_scroll(), Vec2::ZERO, "and doesn't scroll");
        let out = run(&mut f, 0.6, Under::Field, vec![contact(2, TouchPhase::Start, q + egui::vec2(90.0, 0.0))]);
        assert_eq!(presses(&out), [false], "a second finger ends the drag");
        let out =
            run(&mut f, 0.7, Under::Field, vec![contact(1, TouchPhase::End, q), press(q, false), contact(2, TouchPhase::End, q), Event::PointerGone]);
        assert!(presses(&out).is_empty());
    }

    #[test]
    fn elsewhere_on_the_canvas_or_for_a_mouse_events_pass() {
        let p = Pos2::new(10.0, 10.0);
        // Not a field or a list: the press goes through a frame late, where the finger landed.
        let mut f = FingerFilter::default();
        let out = run(&mut f, 0.0, Under::Other, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        assert_eq!((out.first(), presses(&out)), (Some(&Event::PointerMoved(p)), vec![]));
        assert_eq!(presses(&run(&mut f, 0.02, Under::Other, vec![])), [true]);
        assert_eq!(presses(&run(&mut f, 0.04, Under::Other, vec![contact(1, TouchPhase::End, p), press(p, false)])), [false]);
        // On the canvas: as it came, the same frame.
        let events = vec![contact(1, TouchPhase::Start, p), Event::PointerMoved(p), press(p, true)];
        let mut raw = egui::RawInput { events: events.clone(), time: Some(1.0), ..Default::default() };
        f.filter(&mut raw, |_| true, || Under::List);
        assert_eq!(raw.events, events);
        let end = vec![contact(1, TouchPhase::End, p), press(p, false), Event::PointerGone];
        assert_eq!(run(&mut f, 1.1, Under::List, end.clone()), end);
        // A mouse: as it came.
        let mouse = vec![Event::PointerMoved(p), press(p, true), Event::PointerMoved(p + egui::vec2(40.0, 0.0)), press(p, false)];
        assert_eq!(run(&mut f, 3.0, Under::List, mouse.clone()), mouse);
    }

    /// A browser sends a finger's `pointerdown` a frame before its `touchstart`: on a touch screen
    /// that lone press waits a frame, and is dropped for the finger (whose own press comes with its
    /// touch) or given to egui then (a mouse's on a tablet). A click in one frame goes at once.
    #[test]
    fn a_lone_press_on_a_touch_screen_waits_a_frame_for_its_finger() {
        let p = Pos2::new(40.0, 40.0);
        let mut f = FingerFilter::default();
        f.expect_touch();
        // A finger off the canvas: the early press never reaches egui, nor its own.
        assert!(presses(&run(&mut f, 0.0, Under::List, vec![press(p, true)])).is_empty());
        assert_eq!(f.repaint_after(), Some(std::time::Duration::ZERO));
        assert!(presses(&run(&mut f, 0.02, Under::List, vec![press(p, true), contact(1, TouchPhase::Start, p)])).is_empty());
        assert!(presses(&run(&mut f, 0.04, Under::List, vec![press(p, false)])).is_empty(), "pointerup");
        let out = run(&mut f, 0.06, Under::List, vec![press(p, false), Event::PointerGone, contact(1, TouchPhase::End, p)]);
        assert_eq!(presses(&out), [true, false], "a tap: one click");
        // A finger on the canvas: its touch's own press goes through, once.
        let mut raw = egui::RawInput { events: vec![press(p, true)], time: Some(1.0), ..Default::default() };
        f.filter(&mut raw, |_| true, || Under::Other);
        assert!(presses(&raw.events).is_empty());
        let mut raw = egui::RawInput { events: vec![press(p, true), contact(1, TouchPhase::Start, p)], time: Some(1.02), ..Default::default() };
        f.filter(&mut raw, |_| true, || Under::Other);
        assert_eq!(presses(&raw.events), [true]);
        // A second finger's `pointerdown` presses nothing either.
        let q = p + egui::vec2(80.0, 0.0);
        assert!(presses(&run(&mut f, 1.04, Under::Other, vec![press(q, true)])).is_empty());
        assert!(presses(&run(&mut f, 1.06, Under::Other, vec![contact(2, TouchPhase::Start, q)])).is_empty());
        run(&mut f, 1.08, Under::Other, vec![contact(2, TouchPhase::End, q), contact(1, TouchPhase::End, p), press(p, false)]);
        // A mouse on the tablet: its press goes a frame late, as it was; a click at once.
        assert!(presses(&run(&mut f, 2.0, Under::List, vec![Event::PointerMoved(p), press(p, true)])).is_empty());
        assert_eq!(run(&mut f, 2.02, Under::List, vec![]), [Event::PointerMoved(p), press(p, true)]);
        let click = vec![press(p, false), press(p, true), press(p, false)];
        assert_eq!(run(&mut f, 2.04, Under::List, click.clone()), click);
        // Without a touch screen nothing waits.
        let mut desk = FingerFilter::default();
        assert_eq!(run(&mut desk, 0.0, Under::List, vec![press(p, true)]), [press(p, true)]);
    }

    /// A multi-line text field in a scroll area, run through the filter as the app runs it
    /// (`CadApp::raw_input_hook`, then the finger's scroll handed to egui): a finger drag
    /// scrolls and leaves the field alone, a tap focuses it, a long press then a drag selects.
    #[test]
    fn a_finger_drag_in_a_text_field_scrolls_it_a_tap_focuses_and_a_long_press_selects() {
        let ctx = egui::Context::default();
        let field = egui::Id::unique("long field");
        let mut text = "A line of a long note\n".repeat(200);
        let mut t = 0.0;
        let mut frame = |f: &mut FingerFilter, dt: f64, events: Vec<Event>| {
            t += dt;
            let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
            let mut raw = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            f.filter(&mut raw, |_| false, || under_finger(&ctx));
            let mut offset = 0.0_f32;
            let mut selected: usize = 0;
            ctx.run_ui(raw, |ui| {
                let d = f.take_scroll();
                ui.ctx().input_mut(|i| i.smooth_scroll_delta += d);
                let out = egui::ScrollArea::vertical().show(ui, |ui| egui::TextEdit::multiline(&mut text).id(field).show(ui));
                offset = out.state.clamped_offset().y;
                selected = out.inner.cursor_range.map_or(0, |r| {
                    let s = r.as_sorted_char_range();
                    (s.end - s.start).into()
                });
            })
            .drop_without_applying_deltas();
            (offset, selected)
        };
        let mut f = FingerFilter::default();
        frame(&mut f, 0.02, vec![contact(9, TouchPhase::Cancel, Pos2::new(-5.0, -5.0))]);
        frame(&mut f, 0.02, Vec::new());
        let p = Pos2::new(150.0, 250.0);
        frame(&mut f, 0.02, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        let mut offset = 0.0;
        for i in 1..=10 {
            let at = p - egui::vec2(0.0, 15.0 * i as f32);
            offset = frame(&mut f, 0.02, vec![contact(1, TouchPhase::Move, at), Event::PointerMoved(at)]).0;
        }
        assert!((offset - 150.0).abs() < 1.0, "the field scrolled with the finger: {offset}");
        let end = p - egui::vec2(0.0, 150.0);
        frame(&mut f, 0.02, vec![contact(1, TouchPhase::End, end), press(end, false), Event::PointerGone]);
        frame(&mut f, 0.02, Vec::new());
        assert!(!ctx.memory(|m| m.has_focus(field)), "a scroll doesn't focus or select in the field");
        // A tap focuses it.
        frame(&mut f, 0.02, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        frame(&mut f, 0.02, Vec::new());
        frame(&mut f, 0.02, vec![contact(1, TouchPhase::End, p), press(p, false), Event::PointerGone]);
        let (still, none) = frame(&mut f, 0.02, Vec::new());
        assert!(ctx.memory(|m| m.has_focus(field)), "a tap focuses the field");
        assert!((still - offset).abs() < 1.0 && none == 0, "and doesn't scroll or select");
        // A long press, then a drag, selects.
        frame(&mut f, 0.02, vec![contact(1, TouchPhase::Start, p), press(p, true)]);
        frame(&mut f, 0.02, Vec::new());
        frame(&mut f, 0.5, Vec::new());
        let q = p + egui::vec2(60.0, 40.0);
        frame(&mut f, 0.02, vec![contact(1, TouchPhase::Move, q), Event::PointerMoved(q)]);
        let (_, picked) = frame(&mut f, 0.02, vec![contact(1, TouchPhase::End, q), press(q, false), Event::PointerGone]);
        assert!(picked > 0, "a long press then a drag selects");
    }
}
