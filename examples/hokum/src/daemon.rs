//! `hokum service`: owns `org.freedesktop.Notifications` and draws the cards on a click-through
//! layer surface. It maps the surface for the first card (so it opens on the output focused
//! then) and unmaps it once the last card has left. `app.hokum.Daemon.Reload` re-reads the
//! settings file live.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use rune::{App, AppHandle, AppSpec, Cx, El, Layer, LayerAnchor, Tx, col, el, row, text};
use rune::{Cursor, Transition};
use rune_kit::Kit;

use crate::card::{Cards, Note, Timeout, card, samples};
use crate::style::{Appearance, Settings};

/// The gap around the stack, which leaves room for shadows.
pub const GUTTER: f32 = 18.0;
/// Long enough for the last card's exit to finish before its surface unmaps.
const UNMAP_AFTER: Duration = Duration::from_millis(900);

/// Why a notification closed (the spec's reasons).
#[derive(Clone, Copy)]
pub enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

pub enum Out {
    Closed(u32, Reason),
    Action(u32, String),
}

pub struct Ui {
    /// Newest first.
    notes: Vec<Note>,
    paused: bool,
    pub settings: Settings,
    /// The desktop's colour scheme (`Appearance::System`).
    pub system_dark: bool,
    out: Sender<Out>,
    timer: Sender<Option<Instant>>,
    /// Maps the surface for the first card; None when headless.
    handle: Option<AppHandle<Ui>>,
    mapped: bool,
    unmap_at: Option<Instant>,
}

impl Ui {
    fn new(settings: Settings, out: Sender<Out>, timer: Sender<Option<Instant>>, handle: Option<AppHandle<Ui>>) -> Ui {
        Ui {
            notes: Vec::new(),
            paused: false,
            settings,
            system_dark: true,
            out,
            timer,
            handle,
            mapped: false,
            unmap_at: None,
        }
    }

    pub fn dark(&self) -> bool {
        match self.settings.appearance {
            Appearance::System => self.system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// New settings from the file: the surface moves if it must, and cards restyle in place.
    pub fn apply(&mut self, settings: Settings) {
        if let Some(h) = &self.handle {
            h.set_layer(layer(&settings));
        }
        if !settings.pause_on_hover {
            self.hover(false);
        }
        self.settings = settings;
    }

    /// Shows `n`, replacing the card with the same id or stack tag in place.
    pub fn upsert(&mut self, mut n: Note) {
        if self.settings.do_not_disturb && n.urgency < 2 {
            let _ = self.out.send(Out::Closed(n.id, Reason::Expired));
            return;
        }
        let base = self.settings.linger.duration();
        let timeout = match n.timeout {
            Timeout::Never => None,
            Timeout::After(d) => Some(d),
            Timeout::Default if n.urgency >= 2 => None,
            Timeout::Default if n.urgency == 0 => Some(base * 2 / 3),
            // Cards with buttons stay long enough to reach them.
            Timeout::Default if !n.actions.is_empty() => Some(base.max(Duration::from_secs(10))),
            Timeout::Default => Some(base),
        };
        match (self.paused, timeout) {
            (_, None) => {}
            (true, Some(t)) => n.left = Some(t),
            (false, Some(t)) => n.deadline = Some(Instant::now() + t),
        }
        let same = |o: &Note| o.id == n.id || (n.tag.is_some() && o.tag == n.tag && o.app == n.app);
        if let Some(i) = self.notes.iter().position(same) {
            let old = self.notes[i].id;
            if old != n.id {
                let _ = self.out.send(Out::Closed(old, Reason::Closed));
            }
            self.notes[i] = n;
        } else {
            self.notes.insert(0, n);
        }
        if !self.mapped
            && let Some(h) = &self.handle
        {
            h.open();
            self.mapped = true;
        }
        self.reschedule();
    }

    pub fn close(&mut self, id: u32, reason: Reason) {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.notes.len() != before {
            let _ = self.out.send(Out::Closed(id, reason));
            self.reschedule();
        }
    }

    fn clear(&mut self) {
        for n in self.notes.drain(..) {
            let _ = self.out.send(Out::Closed(n.id, Reason::Dismissed));
        }
        self.paused = false;
        self.reschedule();
    }

    fn expire(&mut self, now: Instant, tx: &mut Tx) {
        let gone: Vec<u32> = self.notes.iter().filter(|n| n.deadline.is_some_and(|d| d <= now)).map(|n| n.id).collect();
        for id in gone {
            self.close(id, Reason::Expired);
        }
        if self.notes.is_empty() && self.mapped && self.unmap_at.is_some_and(|t| t <= now) {
            tx.close_window();
            self.mapped = false;
            self.paused = false;
        }
        self.reschedule();
    }

    fn reschedule(&mut self) {
        self.unmap_at = match (self.notes.is_empty() && self.mapped, self.unmap_at) {
            (false, _) => None,
            (true, Some(t)) => Some(t),
            (true, None) => Some(Instant::now() + UNMAP_AFTER),
        };
        let _ = self.timer.send(self.notes.iter().filter_map(|n| n.deadline).chain(self.unmap_at).min());
    }
}

impl Cards for Ui {
    fn dismiss(&mut self, id: u32) {
        self.close(id, Reason::Dismissed);
        // The pointer may be over the card that just went; let timers run until it moves.
        self.hover(false);
    }

    fn invoke(&mut self, id: u32, key: &str) {
        let resident = self.notes.iter().find(|n| n.id == id).is_some_and(|n| n.resident);
        let _ = self.out.send(Out::Action(id, key.to_owned()));
        if !resident {
            self.dismiss(id);
        }
    }

    fn activate(&mut self, id: u32) {
        let default = self.notes.iter().find(|n| n.id == id).is_some_and(|n| n.has_default);
        if default { self.invoke(id, "default") } else { self.dismiss(id) }
    }

    /// Over a card: pause every timer. Off them: resume where they were.
    fn hover(&mut self, on: bool) {
        let on = on && self.settings.pause_on_hover;
        if on == self.paused {
            return;
        }
        self.paused = on;
        let now = Instant::now();
        for n in &mut self.notes {
            if on {
                n.left = n.deadline.take().map(|d| d.saturating_duration_since(now));
            } else {
                n.deadline = n.left.take().map(|l| now + l.max(Duration::from_millis(1200)));
            }
        }
        self.reschedule();
    }
}

/// Where the surface sits for `s`: a full-height strip on the chosen side.
pub fn layer(s: &Settings) -> Layer {
    let right = s.settings_right();
    Layer {
        namespace: "hokum",
        anchor: LayerAnchor { top: true, bottom: true, right, left: !right },
        size: (s.style.card_width() + GUTTER * 2.0, 0.0),
        ..Layer::default()
    }
}

trait Side {
    fn settings_right(&self) -> bool;
}

impl Side for Settings {
    fn settings_right(&self) -> bool {
        self.position.right()
    }
}

/// The stack of cards for `notes` in a corner of its parent, as both processes draw it.
pub fn stack<S: Cards>(notes: &[Note], s: &Settings, k: &Kit, more: Option<El>) -> El {
    let (_, _, between) = s.style.spacing();
    let right = s.position.right();
    let bottom = s.position.bottom();
    let shown = notes.iter().take(s.max_shown.max(1)).map(|n| card::<S>(n, &s.style, k, right));
    let cards: Vec<El> = if bottom { shown.rev().collect() } else { shown.collect() };
    let (l, r) = if right { (None, Some(GUTTER)) } else { (Some(GUTTER), None) };
    let (t, b) = if bottom { (None, Some(GUTTER)) } else { (Some(GUTTER), None) };
    let mut col = col().key("stack").pin(l, t, r, b).w(s.style.card_width()).gap(between);
    let more = more.map(|m| {
        let row = row().key("more-row").w_full().child(m);
        if right { row.justify_end() } else { row }
    });
    if bottom {
        col = col.children(more).children(cards);
    } else {
        col = col.children(cards).children(more);
    }
    col
}

fn more(hidden: usize, k: &Kit) -> El {
    let c = &k.c;
    row()
        .key("more")
        .role("button")
        .label("Clear all")
        .h(28.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(999.0)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .hover(c.hover)
        .press(c.pressed)
        .cursor(Cursor::Pointer)
        .on_hover(|s: &mut Ui| s.hover(true))
        .on_hover_end(|s: &mut Ui| s.hover(false))
        .on_press(|s: &mut Ui| s.clear())
        .child(text(format!("{hidden} more")).key("count").font(k.ty.small_medium).color(c.text_muted))
        .child(text("Clear all").key("clear").font(k.ty.small_medium).color(c.text_base))
        .transition(Transition::opacity().combined(Transition::scale(0.9)))
}

fn view(s: &Ui, _: &mut Cx<Ui>) -> El {
    let k = rune_theme_opencode::kit(s.dark());
    let hidden = s.notes.len().saturating_sub(s.settings.max_shown.max(1));
    let pill = (hidden > 0).then(|| more(hidden, k));
    el().key("root").w_full().h_full().child(stack::<Ui>(&s.notes, &s.settings, k, pill))
}

fn demo(s: &mut Ui) {
    for n in samples().into_iter().rev() {
        s.upsert(n);
    }
}

fn preset(s: &mut Ui, i: usize) {
    s.settings.style = crate::style::PRESETS[i].1;
    demo(s);
}

fn overflow(s: &mut Ui) {
    demo(s);
    for i in 0..3 {
        s.upsert(Note::new(10 + i, "GitHub", format!("Review requested #{}", 412 + i), "anomalyco/rune: layer-shell shell"));
    }
}

fn idle() -> Ui {
    Ui::new(Settings::load(), mpsc::channel().0, mpsc::channel().0, None)
}

/// Sleeps until the earliest deadline, then expires what's due; each reschedule replaces it.
fn timers(handle: AppHandle<Ui>) -> Sender<Option<Instant>> {
    let (tx, rx) = mpsc::channel::<Option<Instant>>();
    std::thread::spawn(move || {
        let mut next: Option<Instant> = None;
        loop {
            let msg = match next {
                Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(t) => next = t,
                Err(RecvTimeoutError::Timeout) => {
                    next = None;
                    handle.update(|s, tx: &mut Tx| s.expire(Instant::now(), tx));
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    tx
}

pub fn run(headless: bool, replace: bool) {
    let settings = Settings::load();
    let size = (settings.style.card_width() + GUTTER * 2.0, 900.0);
    let spec = AppSpec {
        title: "Hokum",
        size,
        init: idle,
        view,
        scenes: &[
            ("demo", demo),
            ("overflow", overflow),
            ("soft", |s| preset(s, 0)),
            ("crisp", |s| preset(s, 1)),
            ("capsule", |s| preset(s, 2)),
            ("ribbon", |s| preset(s, 3)),
        ],
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        setup: None,
        poll: None,
        on_key: None,
    };
    let app = App::new(spec).layer(layer(&settings)).windowless();
    if headless {
        return app.run();
    }
    let handle = app.handle();
    let timer = timers(handle.clone());
    let out = match crate::bus::serve(handle.clone(), replace) {
        Ok(out) => out,
        Err(e) => {
            eprintln!("hokum: {e}");
            std::process::exit(1);
        }
    };
    app.init(move || Ui::new(settings, out, timer, Some(handle))).run();
}
