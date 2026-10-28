//! One notification and the card that draws it, shared by the overlay and the settings preview.

use std::time::{Duration, Instant};

use rune::{Color, Cursor, Edge, El, Shadow, Spring, TextStyle, Transition, col, el, icon, oklch, row, text};
use rune_kit::{Button, ButtonSize, ButtonVariant, Kit, Progress};
use rune_theme_opencode::icons;

use crate::style::{Depth, Mark, Style};

/// How long a notification stays when the app leaves it to the daemon.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Timeout {
    Default,
    Never,
    After(Duration),
}

pub struct Note {
    pub id: u32,
    pub app: String,
    /// Where in the app it came from (a Slack channel), shown after the app's name.
    pub context: Option<String>,
    pub summary: String,
    pub body: String,
    /// (key, label) without the default action.
    pub actions: Vec<(String, String)>,
    pub has_default: bool,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    pub value: Option<f32>,
    pub tag: Option<String>,
    pub resident: bool,
    pub timeout: Timeout,
    pub time: String,
    pub deadline: Option<Instant>,
    /// Time left while timers are paused.
    pub left: Option<Duration>,
}

impl Note {
    pub fn new(id: u32, app: impl Into<String>, summary: impl Into<String>, body: impl Into<String>) -> Note {
        Note {
            id,
            app: app.into(),
            context: None,
            summary: summary.into(),
            body: body.into(),
            actions: Vec::new(),
            has_default: false,
            urgency: 1,
            value: None,
            tag: None,
            resident: false,
            timeout: Timeout::Default,
            time: clock(),
            deadline: None,
            left: None,
        }
    }
}

/// What a card's handlers do in whichever state draws it.
pub trait Cards: 'static {
    fn dismiss(&mut self, id: u32);
    fn invoke(&mut self, id: u32, key: &str);
    /// A click on the card itself.
    fn activate(&mut self, id: u32);
    fn hover(&mut self, on: bool);
}

pub fn clock() -> String {
    // SAFETY: localtime_r only writes the struct we pass.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

/// A stable colour per app, so the same sender always reads the same at a glance.
pub fn tint(app: &str) -> Color {
    let h = app.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    oklch(0.64, 0.13, (h % 360) as f32)
}

/// The card for `n` in `style`. `from_right` picks the edge it slides in from and leaves by.
pub fn card<S: Cards>(n: &Note, style: &Style, k: &Kit, from_right: bool) -> El {
    let c = &k.c;
    let id = n.id;
    let critical = n.urgency >= 2;
    let accent = if critical { c.danger.icon } else { tint(&n.app) };
    let (pad, gap, _) = style.spacing();
    let indent = match style.mark {
        Mark::Letter => 28.0,
        Mark::Dot => 16.0,
        Mark::Stripe | Mark::None => 0.0,
    };

    let mut head = row().key("head").w_full().gap(8.0).center_y();
    match style.mark {
        Mark::Letter => {
            let badge = el().key("badge").size(20.0, 20.0).radius(style.badge_radius()).center().fill(accent);
            let badge = if critical {
                badge.child(icon(&icons::WARNING_BOLD, 12.0).color(Color::WHITE))
            } else {
                let letter = n.app.chars().find(|ch| ch.is_alphanumeric()).unwrap_or('•').to_uppercase().to_string();
                badge.child(text(letter).font(TextStyle::new(11.0, 650.0).lh(12.0)).color(Color::WHITE))
            };
            head = head.child(badge);
        }
        Mark::Dot => head = head.child(el().key("dot").size(8.0, 8.0).radius(4.0).fill(accent)),
        Mark::Stripe | Mark::None => {}
    }
    let close = row()
        .key("close")
        .role("button")
        .label("Dismiss")
        .size(22.0, 22.0)
        .center()
        .radius(style.badge_radius().min(6.0))
        .hover(c.hover)
        .press(c.pressed)
        .cursor(Cursor::Pointer)
        .on_hover(|s: &mut S| s.hover(true))
        .on_press(move |s: &mut S| s.dismiss(id))
        .child(icon(k.glyph.close, 14.0).color(c.icon_muted));
    head = head
        .child(text(match &n.context {
            Some(ctx) => format!("{} · {ctx}", n.app),
            None => n.app.clone(),
        }).key("app").font(k.ty.small_medium).color(c.text_muted).truncate().grow(1.0).shrink())
        .child(text(n.time.clone()).key("time").font(k.ty.small).color(c.text_faint))
        .child(close);

    let mut content = col().key("content").w_full().gap(2.0).pl(indent);
    if !n.summary.is_empty() {
        content = content.child(text(n.summary.clone()).key("summary").font(k.ty.ui_medium).color(c.text_base).wrap().w_full());
    }
    if !n.body.is_empty() {
        content = content.child(text(n.body.clone()).key("body").font(k.ty.ui).color(c.text_muted).wrap().w_full());
    }
    if let Some(v) = n.value {
        content = content.child(el().key("progress").w_full().pt(6.0).child(Progress::new(Some(v)).build(k)));
    }

    let stripe = style.mark == Mark::Stripe;
    let mut root = col()
        .key(format!("note-{id}"))
        .role("status")
        .label(n.summary.clone())
        .w_full()
        .padding(pad)
        .gap(gap)
        .radius(style.radius())
        .fill(c.layer_01)
        .hover(k.alpha(c.white, if k.dark { 0.02 } else { 0.0 }))
        .cursor(Cursor::Pointer)
        .on_hover(|s: &mut S| s.hover(true))
        .on_hover_end(|s: &mut S| s.hover(false))
        .on_press(move |s: &mut S| s.activate(id));
    if critical {
        root = root.border(1.0, c.danger.border);
    } else if style.border {
        root = root.border(1.0, c.border_muted);
    }
    match style.depth {
        Depth::Flat => {}
        Depth::Soft | Depth::Deep => {
            for sh in &k.elevation.overlay {
                root = root.box_shadow(*sh);
            }
        }
    }
    if style.depth == Depth::Deep {
        let a = if k.dark { 0.45 } else { 0.16 };
        root = root.box_shadow(Shadow { y: 14.0, blur: 36.0, color: Color::BLACK.alpha(a), ..Default::default() });
    }
    let mut body = col().key("main").w_full().gap(gap).child(head).child(content);
    if !n.actions.is_empty() {
        let buttons = n.actions.iter().enumerate().map(|(i, (key, label))| {
            let key = key.clone();
            Button::new(label.clone())
                .variant(if i == 0 { ButtonVariant::Neutral } else { ButtonVariant::Ghost })
                .size(ButtonSize::Small)
                .on_press(move |s: &mut S| s.invoke(id, &key))
                .build(k)
                .key(format!("action-{i}"))
                .on_hover(|s: &mut S| s.hover(true))
        });
        body = body.child(row().key("actions").pl(indent).pt(4.0).gap(6.0).flex_wrap().children(buttons));
    }
    root = if stripe {
        // The stripe stretches with the card's content, inset from its edge.
        root.pl(pad - 4.0).child(
            row()
                .key("ribbon")
                .w_full()
                .gap(9.0)
                .items_stretch()
                .child(el().key("stripe").w(3.0).radius(1.5).fill(accent))
                .child(body.grow(1.0).shrink().w(0.0)),
        )
    } else {
        root.child(body)
    };
    let dx = if from_right { 48.0 } else { -48.0 };
    let enter = Transition::offset(dx, 0.0).combined(Transition::opacity()).combined(Transition::scale(0.97));
    let exit = Transition::move_edge(if from_right { Edge::Trailing } else { Edge::Leading }).combined(Transition::opacity());
    root.transition(Transition::asymmetric(enter, exit)).motion(Spring::new(0.38, 0.12))
}

/// Notes that show off every part of a card, for previews and snapshots.
pub fn samples() -> Vec<Note> {
    let mut a = Note::new(1, "Slack", "Dax", "hokum looks sick. ship it before anyone sees the mako ones");
    a.context = Some("#abattoir".into());
    a.actions = vec![("reply".into(), "Reply".into()), ("later".into(), "Later".into())];
    a.has_default = true;
    let mut b = Note::new(2, "Volume", "Volume", "");
    b.value = Some(0.62);
    b.tag = Some("volume".into());
    let c = Note::new(3, "Firefox", "Download complete", "hokum-x86_64.tar.zst · 4.2 MB");
    let mut d = Note::new(4, "Battery", "Battery critically low", "4% remaining. Plug in now or the session will suspend.");
    d.urgency = 2;
    d.timeout = Timeout::Never;
    vec![a, d, b, c]
}
