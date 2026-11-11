//! Fingers on a touch screen (phones and tablets, in a browser or natively).
//!
//! A touch arrives from eframe as [`egui::Event::Touch`] plus emulated primary-button pointer
//! events, so left alone a one-finger drag on the page selects text and the document can't be
//! scrolled by finger. [`TouchGesture::filter`] runs in the raw input hook, before egui sees the
//! frame, and holds the emulated press back while it finds out what the finger is doing:
//! - it moves more than [`SLOP`]: the view scrolls with it, 1:1, and coasts after it lifts
//!   ([`crate::scroll::Momentum`]); the press never reaches egui, so nothing is selected, moved
//!   or dragged;
//! - it lifts first: a tap, given to egui as a click where the finger came down (caret, tips,
//!   buttons; two taps make a double-click);
//! - it rests for [`LONG_PRESS`]: the press goes through then, and a drag selects, as on iOS;
//! - a second finger comes down: a pinch zooms the page and the fingers pan it (the canvas reads
//!   [`egui::InputState::multi_touch`]), and whatever the first finger started stops.
//!
//! This applies on the document ([`Target::Canvas`], the rulers and page gaps too) and in text
//! fields ([`Target::Field`], whose scroll goes to the scroll area around them). Everything else
//! (buttons, lists, panes, sliders, scroll bars) keeps egui's own touch handling: its scroll areas
//! already scroll when dragged by finger. Mouse, pen-less touchpad and wheel input never contain
//! [`egui::Event::Touch`] and pass through untouched.

use egui::{Event, Pos2, TouchId, TouchPhase, Vec2};

use crate::scroll::Momentum;

/// Finger travel (points) that makes a press a scroll rather than a tap or a long press.
pub const SLOP: f32 = 8.0;
/// Seconds a finger rests before its press goes through and a drag selects.
pub const LONG_PRESS: f64 = 0.45;
/// Finger travel (points) along the swipe that turns a Read Mode column.
const TURN: f32 = 48.0;

/// What a finger that came down is acting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The document canvas, with its rulers and the space around the pages.
    Canvas,
    /// A text field (its scroll area scrolls).
    Field,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// Down, not yet moved past [`SLOP`] or held for [`LONG_PRESS`].
    Pending,
    Scrolling,
    /// Held, then the press went through: egui sees a drag.
    Selecting,
    /// Two fingers or more.
    Multi,
    /// Not a canvas or text field: egui's own handling (the press went through).
    Native,
}

impl Phase {
    /// The emulated pointer press/release of this phase is held back.
    fn owned(self) -> bool {
        matches!(self, Phase::Pending | Phase::Scrolling | Phase::Selecting | Phase::Multi)
    }
}

/// How the fingers on the screen are read. Kept by the app across frames.
#[derive(Debug, Default)]
pub struct TouchGesture {
    phase: Phase,
    /// None while [`Phase::Pending`] waits a frame to see what egui found under the finger.
    target: Option<Target>,
    /// The finger that started the gesture.
    finger: Option<TouchId>,
    down: Vec<TouchId>,
    start: Pos2,
    start_time: f64,
    last: Pos2,
    frame_time: Option<f64>,
    /// This frame's scroll for `target`, in the wheel's convention (positive = content moves
    /// down/right): the finger's own movement.
    scroll: Vec2,
    scroll_target: Option<Target>,
    momentum: Momentum,
    /// The gesture already turned a Read Mode column.
    turned: bool,
    /// The emulated `PointerGone` of a lifted finger waits until a text field's coast ends: the
    /// field's scroll area only scrolls under the pointer.
    gone_pending: bool,
    /// The last primary press egui saw came from a finger.
    press_was_touch: bool,
    /// Presses may be a finger's: the device's main pointer is coarse (a phone or tablet,
    /// [`TouchGesture::expect_touch`]) or a finger has been seen.
    expect_touch: bool,
    /// A primary press from a frame with no finger in it, held for a frame: a browser sends a
    /// finger's `pointerdown` (and runs a frame for it) before its `touchstart`. A finger coming
    /// down next frame drops it; otherwise it was a mouse's and goes through then.
    held: Option<(Pos2, egui::Modifiers)>,
    /// Where a primary press went through to egui with no release since (a finger's first press
    /// on a device not known to have a touch screen): released when a finger takes over, so egui
    /// isn't left holding a button that a drag would select with.
    passed_down: Option<Pos2>,
}

impl TouchGesture {
    /// Rewrites one frame's raw input (see the module docs). `on_canvas`: a finger coming down
    /// there is on the document. `field_hovered`: egui's last pass found a text field under the
    /// pointer (asked the frame after the finger came down, where the pointer was moved to it).
    pub fn filter(&mut self, raw: &mut egui::RawInput, on_canvas: impl Fn(Pos2) -> bool, field_hovered: impl FnOnce() -> bool) {
        let now = raw.time.or(self.frame_time.map(|t| t + 1.0 / 60.0)).unwrap_or(0.0);
        let dt = self.frame_time.map_or(0.0, |t| (now - t).clamp(0.0, 1.0) as f32);
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
        let gliding = self.momentum.is_active() || self.gone_pending;
        if !touched && self.phase == Phase::Idle {
            let primary = |pressed: bool| {
                raw.events
                    .iter()
                    .position(|e| matches!(e, Event::PointerButton { button: egui::PointerButton::Primary, pressed: p, .. } if *p == pressed))
            };
            // On a touch screen a press alone (no release in its frame) waits a frame for its
            // finger, also while the last swipe still glides (the next one comes down mid-glide).
            if self.expect_touch
                && !replayed
                && let Some(i) = primary(true)
                && primary(false).is_none()
                && let Some(Event::PointerButton { pos, modifiers, .. }) = raw.events.get(i).cloned()
                && pos.is_finite()
            {
                raw.events.remove(i);
                self.held = Some((pos, modifiers));
            }
        }
        if !touched && self.phase == Phase::Idle && !gliding {
            // Mouse, touchpad and keyboard frames: as they came, apart from noting their presses.
            self.note_passed(&raw.events);
            if raw.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })) {
                self.press_was_touch = false;
            }
            return;
        }
        // The keys held with the finger, as the emulated pointer events carry them.
        let mods =
            raw.events.iter().find_map(|e| if let Event::PointerButton { modifiers, .. } = e { Some(*modifiers) } else { None }).unwrap_or_default();
        let button = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods };
        let mut pre: Vec<Event> = Vec::new();
        let mut post: Vec<Event> = Vec::new();
        // A finger already up: its coast goes on.
        if self.phase == Phase::Idle && self.momentum.is_active() {
            self.scroll += self.momentum.tick(dt);
            if !self.momentum.is_active() && std::mem::take(&mut self.gone_pending) {
                post.push(Event::PointerGone);
            }
        }
        // Last frame moved the pointer under a finger of unknown target: a text field scrolls,
        // anything else gets its press now (a frame late) and egui's own touch handling.
        if self.phase == Phase::Pending && self.target.is_none() {
            if field_hovered() {
                self.target = Some(Target::Field);
            } else {
                self.phase = Phase::Native;
                pre.extend([Event::PointerMoved(self.start), button(self.start, true)]);
                self.press_was_touch = true;
            }
        }
        if self.phase == Phase::Pending && self.target.is_some() && now - self.start_time >= LONG_PRESS {
            self.phase = Phase::Selecting;
            pre.extend([Event::PointerMoved(self.start), button(self.start, true)]);
            self.press_was_touch = true;
        }
        let owned_before = self.phase.owned();
        // A finger that came down and lifted within this frame is still this frame's to rewrite.
        let mut owned = owned_before;
        let mut moved = Vec2::ZERO;
        let mut out: Vec<Event> = Vec::with_capacity(raw.events.len() + 4);
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
                        // A press that went through before egui knew it was a finger: let go of
                        // it where it was, so the finger's drag can't select from it.
                        if let Some(at) = self.passed_down.take() {
                            pre.push(button(at, false));
                        }
                        self.begin(id, pos, now, on_canvas(pos));
                        owned = true;
                        out.push(Event::PointerMoved(pos));
                    } else if self.down.len() >= 2 && self.phase.owned() {
                        if self.phase == Phase::Selecting {
                            post.push(button(self.last, false));
                        }
                        self.phase = Phase::Multi;
                        self.momentum.cancel();
                    }
                }
                TouchPhase::Move if Some(id) == self.finger => match self.phase {
                    Phase::Pending if self.target.is_some() && (pos - self.start).length() > SLOP => {
                        self.phase = Phase::Scrolling;
                        moved += pos - self.start;
                        self.last = pos;
                    }
                    Phase::Scrolling => {
                        moved += pos - self.last;
                        self.last = pos;
                    }
                    Phase::Pending => {}
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
        if moved != Vec2::ZERO {
            self.scroll += moved;
            self.momentum.feed(moved, dt);
        }
        self.scroll_target = if self.scroll != Vec2::ZERO { self.target } else { None };
        let multi = self.phase == Phase::Multi;
        let mut gone = false;
        for event in raw.events.drain(..) {
            match &event {
                Event::PointerButton { button: egui::PointerButton::Primary, .. } if owned => continue,
                Event::PointerGone if owned => gone = true,
                Event::PointerMoved(_) if owned && multi => {}
                Event::PointerButton { pressed: true, .. } => {
                    self.press_was_touch = touched || !self.down.is_empty();
                    out.push(event);
                }
                _ => out.push(event),
            }
        }
        if self.down.is_empty() {
            if gone && self.momentum.is_active() && self.target == Some(Target::Field) {
                self.gone_pending = true;
            } else if gone {
                post.push(Event::PointerGone);
            }
            if self.phase != Phase::Idle {
                self.phase = Phase::Idle;
                self.finger = None;
            }
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
    /// even its first press is held back until its touch says what it is.
    pub fn expect_touch(&mut self) {
        self.expect_touch = true;
    }

    fn begin(&mut self, id: TouchId, pos: Pos2, now: f64, on_canvas: bool) {
        self.phase = Phase::Pending;
        self.target = on_canvas.then_some(Target::Canvas);
        self.finger = Some(id);
        self.start = pos;
        self.last = pos;
        self.start_time = now;
        self.turned = false;
        self.gone_pending = false;
        self.momentum.cancel();
    }

    /// The gesture's finger lifted (`ended`; else the system cancelled it) at `pos`.
    fn lift(&mut self, ended: bool, pos: Pos2, post: &mut Vec<Event>, button: &impl Fn(Pos2, bool) -> Event) {
        match self.phase {
            // A tap: a click where the finger came down.
            Phase::Pending if ended => {
                post.extend([Event::PointerMoved(self.start), button(self.start, true), button(self.start, false)]);
                self.press_was_touch = true;
            }
            Phase::Scrolling if ended => self.momentum.release(),
            Phase::Scrolling => self.momentum.cancel(),
            Phase::Selecting => post.push(button(if pos.is_finite() { pos } else { self.last }, false)),
            _ => {}
        }
        self.finger = None;
        if !matches!(self.phase, Phase::Multi | Phase::Native) {
            self.phase = if self.down.is_empty() { Phase::Idle } else { Phase::Multi };
        }
    }

    /// This frame's finger scroll for `target` (the wheel's convention), taken once.
    pub fn take_scroll(&mut self, target: Target) -> Vec2 {
        if self.scroll_target == Some(target) {
            self.scroll_target = None;
            std::mem::take(&mut self.scroll)
        } else {
            Vec2::ZERO
        }
    }

    /// Two fingers or more are on the document: pinch zooms it and the fingers pan it.
    pub fn pinching_canvas(&self) -> bool {
        self.phase == Phase::Multi && self.target == Some(Target::Canvas)
    }

    /// The document scrolled into its end: the coast stops.
    pub fn hit_edge(&mut self) {
        self.momentum.cancel();
    }

    /// Read Mode: a swipe along either axis turns one column per gesture, forward for a swipe
    /// left or up. 0 when this frame turns nothing.
    pub fn swipe_turn(&mut self) -> i32 {
        self.momentum.cancel();
        let _ = self.take_scroll(Target::Canvas);
        if self.phase != Phase::Scrolling || self.target != Some(Target::Canvas) || self.turned {
            return 0;
        }
        let travel = self.last - self.start;
        let along = if travel.x.abs() >= travel.y.abs() { travel.x } else { travel.y };
        if along.abs() < TURN {
            return 0;
        }
        self.turned = true;
        if along < 0.0 { 1 } else { -1 }
    }

    /// The last primary press egui saw was a finger's (a touch screen has no hover).
    pub fn press_was_touch(&self) -> bool {
        self.press_was_touch
    }

    /// How soon the app must run again with no new input: a resting finger becomes a long press,
    /// a coast goes on, or a finger's target is decided on the next frame.
    pub fn repaint_after(&self) -> Option<std::time::Duration> {
        if self.held.is_some() {
            return Some(std::time::Duration::ZERO);
        }
        match self.phase {
            Phase::Pending if self.target.is_none() => Some(std::time::Duration::ZERO),
            Phase::Pending => {
                let left = LONG_PRESS - (self.frame_time.unwrap_or(self.start_time) - self.start_time);
                Some(std::time::Duration::from_secs_f64(left.clamp(0.0, LONG_PRESS)))
            }
            _ if self.momentum.is_active() || self.gone_pending => Some(std::time::Duration::ZERO),
            _ => None,
        }
    }
}

/// egui's last pass found a text field under the pointer: one of the widgets it marked hovered
/// keeps a text edit's state.
pub fn field_hovered(ctx: &egui::Context) -> bool {
    let hovered: Vec<egui::Id> = ctx.interaction_snapshot(|s| s.hovered.iter().copied().collect());
    hovered.into_iter().any(|id| egui::TextEdit::load_state(ctx, id).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// One frame at `t` through the filter, over the canvas everywhere.
    fn run(g: &mut TouchGesture, t: f64, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, time: Some(t), ..Default::default() };
        g.filter(&mut raw, |_| true, || false);
        raw.events
    }

    fn presses(events: &[Event]) -> Vec<bool> {
        events.iter().filter_map(|e| if let Event::PointerButton { pressed, .. } = e { Some(*pressed) } else { None }).collect()
    }

    #[test]
    fn a_drag_scrolls_and_holds_the_press_back() {
        let mut g = TouchGesture::default();
        let p = egui::pos2(100.0, 300.0);
        let out = run(&mut g, 0.0, vec![press(p, true), touch(1, TouchPhase::Start, p)]);
        assert!(presses(&out).is_empty(), "the press waits: {out:?}");
        let mut total = Vec2::ZERO;
        for i in 1..=10 {
            let at = p - egui::vec2(0.0, 12.0 * i as f32);
            let out = run(&mut g, f64::from(i) / 60.0, vec![Event::PointerMoved(at), touch(1, TouchPhase::Move, at)]);
            assert!(presses(&out).is_empty());
            total += g.take_scroll(Target::Canvas);
        }
        assert_eq!(total, egui::vec2(0.0, -120.0), "1:1 with the finger");
        let end = p - egui::vec2(0.0, 120.0);
        let out = run(&mut g, 11.0 / 60.0, vec![press(end, false), Event::PointerGone, touch(1, TouchPhase::End, end)]);
        assert!(presses(&out).is_empty(), "no release either: {out:?}");
        assert!(out.contains(&Event::PointerGone));
        // It coasts on, slowing down.
        let first = {
            run(&mut g, 12.0 / 60.0, Vec::new());
            g.take_scroll(Target::Canvas)
        };
        assert!(first.y < -1.0, "{first:?}");
        let mut frames = 0;
        while g.repaint_after().is_some() && frames < 1000 {
            frames += 1;
            run(&mut g, f64::from(12 + frames) / 60.0, Vec::new());
        }
        assert!(frames < 1000, "the coast stops");
        // A mouse press after it waits a frame for a finger (this is a touch screen), then goes
        // through as it was; a press and release in one frame (a click) at once.
        let mouse = vec![Event::PointerMoved(p), press(p, true)];
        assert_eq!(run(&mut g, 30.0, mouse.clone()), [Event::PointerMoved(p)]);
        assert_eq!(run(&mut g, 30.02, Vec::new()), [Event::PointerMoved(p), press(p, true)]);
        assert!(!g.press_was_touch());
        let click = vec![press(p, false), press(p, true), press(p, false)];
        assert_eq!(run(&mut g, 30.04, click[1..].to_vec()), click[1..]);
        // On a device that has shown no finger the mouse is never held.
        let mut desk = TouchGesture::default();
        assert_eq!(run(&mut desk, 0.0, mouse.clone()), mouse);
    }

    #[test]
    fn a_tap_is_a_click_where_the_finger_came_down() {
        let mut g = TouchGesture::default();
        let p = egui::pos2(50.0, 60.0);
        run(&mut g, 0.0, vec![touch(1, TouchPhase::Start, p), press(p, true)]);
        let q = p + egui::vec2(3.0, 2.0);
        run(&mut g, 0.05, vec![touch(1, TouchPhase::Move, q), Event::PointerMoved(q)]);
        let out = run(&mut g, 0.1, vec![touch(1, TouchPhase::End, q), press(q, false), Event::PointerGone]);
        assert_eq!(presses(&out), [true, false]);
        assert!(out.iter().any(|e| matches!(e, Event::PointerButton { pos, .. } if *pos == p)));
        assert_eq!(out.last(), Some(&Event::PointerGone), "the hover ends after the click");
        assert!(g.press_was_touch());
        assert_eq!(g.take_scroll(Target::Canvas), Vec2::ZERO);
    }

    #[test]
    fn a_long_press_lets_the_press_through_and_a_second_finger_ends_it() {
        let mut g = TouchGesture::default();
        let p = egui::pos2(50.0, 60.0);
        run(&mut g, 0.0, vec![touch(1, TouchPhase::Start, p), press(p, true)]);
        assert!(g.repaint_after().is_some_and(|d| d.as_secs_f64() <= LONG_PRESS));
        assert!(presses(&run(&mut g, 0.3, Vec::new())).is_empty());
        assert_eq!(presses(&run(&mut g, 0.5, Vec::new())), [true], "held: the press goes through");
        let q = p + egui::vec2(80.0, 0.0);
        let out = run(&mut g, 0.55, vec![touch(1, TouchPhase::Move, q), Event::PointerMoved(q)]);
        assert!(out.contains(&Event::PointerMoved(q)), "the drag reaches egui");
        assert_eq!(g.take_scroll(Target::Canvas), Vec2::ZERO, "and doesn't scroll");
        let out = run(&mut g, 0.6, vec![touch(2, TouchPhase::Start, q + egui::vec2(0.0, 90.0)), press(q, true)]);
        assert_eq!(presses(&out), [false], "a second finger releases the selection drag");
        assert!(g.pinching_canvas());
        let out = run(&mut g, 0.7, vec![touch(1, TouchPhase::End, q), press(q, false), touch(2, TouchPhase::End, q), Event::PointerGone]);
        assert!(presses(&out).is_empty());
        assert!(!g.pinching_canvas());
    }

    #[test]
    fn elsewhere_the_press_goes_through_a_frame_late_and_text_fields_scroll() {
        let p = egui::pos2(10.0, 10.0);
        let mut g = TouchGesture::default();
        let mut raw = egui::RawInput { events: vec![touch(1, TouchPhase::Start, p), press(p, true)], time: Some(0.0), ..Default::default() };
        g.filter(&mut raw, |_| false, || false);
        assert_eq!(raw.events.first(), Some(&Event::PointerMoved(p)), "the pointer goes to the finger");
        assert!(presses(&raw.events).is_empty(), "the press waits for egui to find what is under it");
        let mut raw = egui::RawInput { time: Some(0.02), ..Default::default() };
        g.filter(&mut raw, |_| false, || false);
        assert_eq!(presses(&raw.events), [true], "not a text field: egui's own handling");
        let mut raw = egui::RawInput { events: vec![touch(1, TouchPhase::End, p), press(p, false)], time: Some(0.04), ..Default::default() };
        g.filter(&mut raw, |_| false, || false);
        assert_eq!(presses(&raw.events), [false]);

        let mut g = TouchGesture::default();
        let mut raw = egui::RawInput { events: vec![touch(1, TouchPhase::Start, p), press(p, true)], time: Some(1.0), ..Default::default() };
        g.filter(&mut raw, |_| false, || false);
        let q = p + egui::vec2(0.0, 30.0);
        let mut raw = egui::RawInput { events: vec![touch(1, TouchPhase::Move, q), Event::PointerMoved(q)], time: Some(1.02), ..Default::default() };
        g.filter(&mut raw, |_| false, || true);
        assert!(presses(&raw.events).is_empty());
        assert_eq!(g.take_scroll(Target::Canvas), Vec2::ZERO);
        assert_eq!(g.take_scroll(Target::Field), egui::vec2(0.0, 30.0));
    }

    /// A multi-line text field in a scroll area, run through the filter as the app runs it
    /// (`WordApp::raw_input_hook`, then `window_ui` handing the field's scroll to egui): a finger
    /// drag scrolls and leaves the field alone, a tap focuses it.
    #[test]
    fn a_finger_drag_in_a_text_field_scrolls_it_and_a_tap_focuses_it() {
        let ctx = egui::Context::default();
        let field = egui::Id::unique("long field");
        let mut text = "A line of a long note\n".repeat(200);
        let mut t = 0.0;
        let mut frame = |g: &mut TouchGesture, events: Vec<Event>| {
            t += 1.0 / 60.0;
            let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
            let mut raw = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            g.filter(&mut raw, |_| false, || field_hovered(&ctx));
            let mut offset = 0.0_f32;
            ctx.run_ui(raw, |ui| {
                let d = g.take_scroll(Target::Field);
                ui.ctx().input_mut(|i| i.smooth_scroll_delta += d);
                let out = egui::ScrollArea::vertical().show(ui, |ui| ui.add(egui::TextEdit::multiline(&mut text).id(field)));
                offset = out.state.clamped_offset().y;
            })
            .drop_without_applying_deltas();
            offset
        };
        let mut g = TouchGesture::default();
        frame(&mut g, Vec::new());
        let p = egui::pos2(150.0, 250.0);
        frame(&mut g, vec![press(p, true), touch(1, TouchPhase::Start, p)]);
        let mut offset = 0.0_f32;
        for i in 1..=10 {
            let at = p - egui::vec2(0.0, 15.0 * i as f32);
            offset = frame(&mut g, vec![Event::PointerMoved(at), touch(1, TouchPhase::Move, at)]);
        }
        assert!((offset - 150.0).abs() < 1.0, "the field scrolled with the finger: {offset}");
        let end = p - egui::vec2(0.0, 150.0);
        frame(&mut g, vec![press(end, false), Event::PointerGone, touch(1, TouchPhase::End, end)]);
        assert!(!ctx.memory(|m| m.has_focus(field)), "a scroll doesn't focus or select in the field");
        let glide = frame(&mut g, Vec::new());
        assert!(glide > offset, "and glides on: {offset} -> {glide}");
        for _ in 0..300 {
            frame(&mut g, Vec::new());
        }
        assert!(g.repaint_after().is_none());
        frame(&mut g, vec![press(p, true), touch(1, TouchPhase::Start, p)]);
        frame(&mut g, Vec::new());
        frame(&mut g, vec![press(p, false), Event::PointerGone, touch(1, TouchPhase::End, p)]);
        frame(&mut g, Vec::new());
        assert!(ctx.memory(|m| m.has_focus(field)), "a tap focuses the field");
    }

    #[test]
    fn read_mode_swipes_turn_once_per_gesture() {
        let mut g = TouchGesture::default();
        let p = egui::pos2(300.0, 300.0);
        run(&mut g, 0.0, vec![touch(1, TouchPhase::Start, p)]);
        let mut turns = 0;
        for i in 1..=8 {
            run(&mut g, f64::from(i) / 60.0, vec![touch(1, TouchPhase::Move, p - egui::vec2(15.0 * i as f32, 0.0))]);
            turns += g.swipe_turn();
        }
        assert_eq!(turns, 1);
        run(&mut g, 0.2, vec![touch(1, TouchPhase::End, p - egui::vec2(120.0, 0.0))]);
        assert_eq!(g.swipe_turn(), 0);
        assert!(g.repaint_after().is_none(), "no coast in Read Mode");
        run(&mut g, 0.3, vec![touch(1, TouchPhase::Start, p)]);
        run(&mut g, 0.32, vec![touch(1, TouchPhase::Move, p + egui::vec2(0.0, 60.0))]);
        assert_eq!(g.swipe_turn(), -1, "down turns back");
    }

    #[test]
    fn junk_positions_are_ignored() {
        let mut g = TouchGesture::default();
        let out = run(&mut g, 0.0, vec![touch(1, TouchPhase::Start, egui::pos2(f32::NAN, 1.0))]);
        assert!(out.is_empty() || !out.iter().any(|e| matches!(e, Event::PointerButton { .. })));
        assert_eq!(g.take_scroll(Target::Canvas), Vec2::ZERO);
    }
}
