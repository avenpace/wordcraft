//! Fingers on the document (`crate::touch`), through the raw input hook as eframe runs it, with
//! the events a browser's touch handlers send: the emulated pointer press, moves and release
//! next to each [`egui::Event::Touch`].

use egui::{Event, Pos2, TouchId, TouchPhase, vec2};
use serde_json::json;
use wordcraft_doc::Pos;

use crate::{Services, WordApp};

struct Phone {
    ctx: egui::Context,
    t: f64,
}

impl Phone {
    fn new(a: &mut WordApp) -> Self {
        let mut p = Phone { ctx: egui::Context::default(), t: 0.0 };
        for _ in 0..4 {
            p.frame(a, Vec::new());
        }
        p
    }

    /// One 1280 by 800 frame 1/60 s after the last, through the raw input hook, logic and UI.
    fn frame(&mut self, a: &mut WordApp, events: Vec<Event>) {
        self.t += 1.0 / 60.0;
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
        let mut raw = egui::RawInput { events, time: Some(self.t), screen_rect: Some(screen), ..Default::default() };
        a.raw_input_hook(&self.ctx, &mut raw);
        self.ctx
            .run_ui(raw, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
    }

    fn idle(&mut self, a: &mut WordApp, frames: usize) {
        for _ in 0..frames {
            self.frame(a, Vec::new());
        }
    }

    fn down(&mut self, a: &mut WordApp, id: u64, at: Pos2) {
        self.frame(a, vec![button(at, true), touch(id, TouchPhase::Start, at)]);
    }

    fn drag(&mut self, a: &mut WordApp, id: u64, from: Pos2, to: Pos2, steps: usize) {
        for i in 1..=steps {
            let at = from + (to - from) * (i as f32 / steps as f32);
            self.frame(a, vec![Event::PointerMoved(at), touch(id, TouchPhase::Move, at)]);
        }
    }

    fn up(&mut self, a: &mut WordApp, id: u64, at: Pos2) {
        self.frame(a, vec![button(at, false), Event::PointerGone, touch(id, TouchPhase::End, at)]);
    }
}

fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch { device_id: egui::TouchDeviceId(7), id: TouchId(id), phase, pos, force: None }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// A document of several pages, caret at its start.
fn long_doc() -> WordApp {
    let text: Vec<String> = (0..160).map(|i| format!("The quick brown fox jumps over the lazy dog, line {i}")).collect();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&text.join("\n"))), Services::default());
    a.run("caret.docStart", json!({})).unwrap();
    a
}

/// Where `pos` is on screen: halfway down its caret.
fn screen_of(a: &mut WordApp, pos: &Pos) -> Pos2 {
    let c = a.session.layout().caret(pos).unwrap();
    crate::canvas::page_to_screen(a, c.page, c.x, c.top + c.height / 2.0).unwrap()
}

/// A finger dragged up the page scrolls it down with the finger, 1:1, and on after it lifts; the
/// caret stays where it was and nothing is selected.
#[test]
fn a_finger_drag_scrolls_the_document_and_selects_nothing() {
    let mut a = long_doc();
    let mut p = Phone::new(&mut a);
    let sel = a.session.sel.clone();
    let start = a.canvas.scroll_offset.y;
    let from = a.canvas.canvas_rect.unwrap().center();
    let to = from - vec2(0.0, 300.0);
    p.down(&mut a, 1, from);
    p.drag(&mut a, 1, from, to, 15);
    assert!((a.canvas.scroll_offset.y - start - 300.0).abs() < 1.0, "follows the finger: {} -> {}", start, a.canvas.scroll_offset.y);
    assert_eq!(a.session.sel, sel, "no selection, no caret move");
    assert!(!a.canvas.drag_selecting());
    p.up(&mut a, 1, to);
    let lifted = a.canvas.scroll_offset.y;
    p.idle(&mut a, 3);
    assert!(a.canvas.scroll_offset.y > lifted + 10.0, "it glides on after the finger lifts: {lifted} -> {}", a.canvas.scroll_offset.y);
    let gliding = a.canvas.scroll_offset.y;
    p.idle(&mut a, 240);
    let settled = a.canvas.scroll_offset.y;
    assert!(settled > gliding, "and slows to a stop");
    p.idle(&mut a, 5);
    assert_eq!(a.canvas.scroll_offset.y, settled, "stopped");
    assert_eq!(a.session.sel, sel);
    // Sideways: the page is narrower than the window, so it doesn't move that way.
    assert_eq!(a.canvas.scroll_offset.x, 0.0);
}

/// A tap places the caret where the finger came down, without scrolling.
#[test]
fn a_tap_places_the_caret() {
    let mut a = long_doc();
    let mut p = Phone::new(&mut a);
    let target = Pos::body(2, 10);
    let at = screen_of(&mut a, &target);
    let scroll = a.canvas.scroll_offset;
    p.down(&mut a, 1, at);
    assert_eq!(a.session.sel.focus, Pos::body(0, 0), "nothing happens while the finger is down");
    p.drag(&mut a, 1, at, at + vec2(2.0, 1.0), 2);
    p.up(&mut a, 1, at + vec2(2.0, 1.0));
    p.idle(&mut a, 2);
    assert!(a.session.sel.is_collapsed());
    assert_eq!(a.session.sel.focus, target);
    assert_eq!(a.canvas.scroll_offset, scroll);
    assert!(a.canvas.press_was_touch, "the press is known to be a finger's (tips on tracked changes stay up)");
}

/// Resting the finger, then dragging, selects from where it came down, as on iOS.
#[test]
fn a_long_press_then_drag_selects() {
    let mut a = long_doc();
    let mut p = Phone::new(&mut a);
    let (from_pos, to_pos) = (Pos::body(3, 4), Pos::body(3, 30));
    let (from, to) = (screen_of(&mut a, &from_pos), screen_of(&mut a, &to_pos));
    let scroll = a.canvas.scroll_offset;
    p.down(&mut a, 1, from);
    p.idle(&mut a, 32);
    p.drag(&mut a, 1, from, to, 6);
    p.up(&mut a, 1, to);
    p.idle(&mut a, 2);
    assert_eq!(a.session.sel.ordered(), (from_pos, to_pos), "selected from the press to the finger");
    assert_eq!(a.canvas.scroll_offset, scroll, "a selecting drag doesn't scroll");
}

/// Two fingers moving apart zoom in about the point between them; moving together zoom out.
#[test]
fn a_pinch_zooms_about_its_centre() {
    let mut a = long_doc();
    let mut p = Phone::new(&mut a);
    // On the page's centre line: a page narrower than the window stays centred, so only a point
    // on that line can stay put sideways.
    let c = a.canvas.canvas_rect.unwrap().center() + vec2(0.0, 40.0);
    let zoom = a.session.view.zoom;
    let under = |a: &mut WordApp| {
        let layout = a.session.layout();
        crate::canvas::page_at(&a.canvas.page_rects, &layout, a.canvas.scale, c).unwrap()
    };
    let before = under(&mut a);
    let (f1, f2) = (c - vec2(0.0, 60.0), c + vec2(0.0, 60.0));
    p.down(&mut a, 1, f1);
    p.frame(&mut a, vec![button(f1, true), touch(2, TouchPhase::Start, f2)]);
    let sel = a.session.sel.clone();
    for i in 1..=10 {
        let s = 60.0 + 6.0 * i as f32;
        let (g1, g2) = (c - vec2(0.0, s), c + vec2(0.0, s));
        p.frame(&mut a, vec![Event::PointerMoved(g1), touch(1, TouchPhase::Move, g1), touch(2, TouchPhase::Move, g2)]);
    }
    let after = under(&mut a);
    assert!(a.session.view.zoom > zoom * 1.5, "zoomed in: {zoom} -> {}", a.session.view.zoom);
    assert_eq!(after.0, before.0, "the same page under the fingers");
    assert!((after.1 - before.1).abs() < 6.0 && (after.2 - before.2).abs() < 6.0, "anchored at the pinch centre: {before:?} -> {after:?}");
    let (g1, g2) = (c - vec2(0.0, 120.0), c + vec2(0.0, 120.0));
    p.frame(&mut a, vec![button(g1, false), Event::PointerGone, touch(1, TouchPhase::End, g1)]);
    p.frame(&mut a, vec![touch(2, TouchPhase::End, g2)]);
    p.idle(&mut a, 3);
    assert_eq!(a.session.sel, sel, "the pinch selects nothing");
}

/// The mouse is unchanged: a press and drag selects at once, and the page stays put.
#[test]
fn a_mouse_drag_still_selects() {
    let mut a = long_doc();
    let mut p = Phone::new(&mut a);
    let (from_pos, to_pos) = (Pos::body(3, 4), Pos::body(3, 30));
    let (from, to) = (screen_of(&mut a, &from_pos), screen_of(&mut a, &to_pos));
    let scroll = a.canvas.scroll_offset;
    p.frame(&mut a, vec![Event::PointerMoved(from)]);
    p.frame(&mut a, vec![button(from, true)]);
    for i in 1..=6 {
        p.frame(&mut a, vec![Event::PointerMoved(from + (to - from) * (i as f32 / 6.0))]);
    }
    p.frame(&mut a, vec![button(to, false)]);
    p.idle(&mut a, 2);
    assert_eq!(a.session.sel.ordered(), (from_pos, to_pos));
    assert_eq!(a.canvas.scroll_offset, scroll);
    assert!(!a.canvas.press_was_touch);
}

/// Read Mode: a swipe left turns to the next column, a swipe down back.
#[test]
fn read_mode_swipes_turn_columns() {
    let mut a = long_doc();
    let mut p = Phone::new(&mut a);
    a.run("view.readMode", json!({})).unwrap();
    p.idle(&mut a, 4);
    assert!(a.canvas.cols > 2, "several columns: {}", a.canvas.cols);
    // Columns stand 18 points apart (`canvas::GAP`).
    let step = a.session.view.read_width * a.canvas.scale + 18.0;
    let sel = a.session.sel.clone();
    let c = a.canvas.canvas_rect.unwrap().center();
    p.down(&mut a, 1, c);
    p.drag(&mut a, 1, c, c - vec2(150.0, 10.0), 6);
    p.up(&mut a, 1, c - vec2(150.0, 10.0));
    p.idle(&mut a, 40);
    assert!((a.canvas.scroll_offset.x - step).abs() < 1.0, "one column on: {} of {step}", a.canvas.scroll_offset.x);
    p.down(&mut a, 1, c);
    p.drag(&mut a, 1, c, c + vec2(0.0, 120.0), 6);
    p.up(&mut a, 1, c + vec2(0.0, 120.0));
    p.idle(&mut a, 40);
    assert!(a.canvas.scroll_offset.x.abs() < 1.0, "and back: {}", a.canvas.scroll_offset.x);
    assert_eq!(a.session.sel, sel);
}
