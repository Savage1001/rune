//! `org.freedesktop.Notifications` on the session bus. Calls arrive on zbus's thread and reach
//! the UI through `AppHandle::update`; what the UI decides (closed, action invoked) comes back
//! on a channel and goes out as signals from a thread of its own.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use rune::AppHandle;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::OwnedValue;

use crate::card::{Note, Timeout};
use crate::daemon::{Out, Reason, Ui};
use crate::style::Settings;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// Hokum's own interface on the same connection: the settings window calls `Reload`.
pub const DAEMON_PATH: &str = "/app/hokum/Daemon";
pub const DAEMON_IFACE: &str = "app.hokum.Daemon";

struct Server {
    ui: AppHandle<Ui>,
    next: AtomicU32,
}

fn hint<T: TryFrom<OwnedValue>>(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<T> {
    hints.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| T::try_from(v).ok())
}

/// Bodies may carry the spec's small markup subset; cards show plain text.
fn plain(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut tag = false;
    for ch in body.chars() {
        match ch {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let out = out.trim();
    // Very long bodies read as a preview.
    match out.char_indices().nth(280) {
        Some((i, _)) => format!("{}…", out[..i].trim_end()),
        None => out.to_owned(),
    }
}

/// Slack titles every message `[workspace] in #channel` (or `[workspace] from Name`) and puts
/// `Sender: message` in the body. Cards read better as sender, message, and the channel next
/// to the app's name.
fn slack(summary: &str, body: &str) -> Option<(String, String, String)> {
    let rest = summary.strip_prefix('[')?.split_once("] ")?.1;
    if let Some(channel) = rest.strip_prefix("in ") {
        let (sender, message) = body.split_once(": ").filter(|(s, _)| !s.is_empty() && s.len() <= 80)?;
        return Some((channel.to_owned(), sender.to_owned(), message.to_owned()));
    }
    let sender = rest.strip_prefix("from ")?;
    Some(("Direct message".to_owned(), sender.to_owned(), body.to_owned()))
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["actions", "body", "body-markup", "persistence", "x-canonical-private-synchronous"]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 { replaces_id } else { self.next.fetch_add(1, Ordering::Relaxed) };
        let app = Some(app_name)
            .filter(|a| !a.is_empty())
            .or_else(|| hint::<String>(&hints, "desktop-entry"))
            .unwrap_or_else(|| "Notification".into());
        let (summary, body) = (plain(&summary), plain(&body));
        let slacky = app.eq_ignore_ascii_case("slack") || hint::<String>(&hints, "desktop-entry").as_deref() == Some("slack");
        let mut n = match slacky.then(|| slack(&summary, &body)).flatten() {
            Some((context, sender, message)) => {
                let mut n = Note::new(id, app, sender, message);
                n.context = Some(context);
                n
            }
            None => Note::new(id, app, summary, body),
        };
        n.urgency = hint::<u8>(&hints, "urgency").unwrap_or(1);
        n.value = hint::<i32>(&hints, "value").map(|v| (v as f32 / 100.0).clamp(0.0, 1.0));
        n.tag = hint::<String>(&hints, "x-dunst-stack-tag")
            .or_else(|| hint::<String>(&hints, "x-canonical-private-synchronous"))
            .filter(|t| !t.is_empty());
        n.resident = hint::<bool>(&hints, "resident").unwrap_or(false);
        for pair in actions.chunks(2) {
            let (key, label) = (pair[0].clone(), pair.get(1).cloned().unwrap_or_default());
            if key == "default" {
                n.has_default = true;
            } else if !label.is_empty() {
                n.actions.push((key, label));
            }
        }
        n.timeout = match expire_timeout {
            0 => Timeout::Never,
            t if t > 0 => Timeout::After(Duration::from_millis(t as u64)),
            _ => Timeout::Default,
        };
        self.ui.update(move |s, _| s.upsert(n));
        id
    }

    fn close_notification(&self, id: u32) {
        self.ui.update(move |s, _| s.close(id, Reason::Closed));
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("Hokum", "anomalyco", env!("CARGO_PKG_VERSION"), "1.2")
    }
}

struct Daemon {
    ui: AppHandle<Ui>,
}

#[zbus::interface(name = "app.hokum.Daemon")]
impl Daemon {
    /// Re-read the settings file and restyle live.
    fn reload(&self) {
        let settings = Settings::load();
        self.ui.update(move |s, _| s.apply(settings));
    }
}

/// Follows the desktop's colour scheme (`org.freedesktop.appearance color-scheme`: 1 dark,
/// 2 light, 0 no preference, which reads as dark like the rest of Hokum).
fn follow_color_scheme(conn: &zbus::blocking::Connection, ui: AppHandle<Ui>) {
    let Ok(proxy) = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    ) else {
        return;
    };
    // Portals answer `v`, and some wrap it twice.
    fn scheme(v: &zbus::zvariant::Value<'_>) -> Option<u32> {
        match v {
            zbus::zvariant::Value::U32(x) => Some(*x),
            zbus::zvariant::Value::Value(inner) => scheme(inner),
            _ => None,
        }
    }
    let scheme = |v: &OwnedValue| scheme(v);
    let apply = |ui: &AppHandle<Ui>, scheme: u32| ui.update(move |s, _| s.system_dark = scheme != 2);
    if let Ok(v) = proxy.call::<_, _, OwnedValue>("ReadOne", &("org.freedesktop.appearance", "color-scheme"))
        && let Some(sc) = scheme(&v)
    {
        apply(&ui, sc);
    }
    std::thread::spawn(move || {
        let Ok(signals) = proxy.receive_signal("SettingChanged") else { return };
        for m in signals {
            if let Ok((ns, key, v)) = m.body().deserialize::<(String, String, OwnedValue)>()
                && ns == "org.freedesktop.appearance"
                && key == "color-scheme"
                && let Some(sc) = scheme(&v)
            {
                apply(&ui, sc);
            }
        }
    });
}

/// Owns the name (taking it from the running daemon with `replace`) and returns where the UI
/// sends what should become signals.
pub fn serve(ui: AppHandle<Ui>, replace: bool) -> zbus::Result<Sender<Out>> {
    let server = Server { ui: ui.clone(), next: AtomicU32::new(1) };
    let conn = zbus::blocking::connection::Builder::session()?
        .serve_at(PATH, server)?
        .serve_at(DAEMON_PATH, Daemon { ui: ui.clone() })?
        .build()?;
    follow_color_scheme(&conn, ui.clone());
    let mut flags = RequestNameFlags::AllowReplacement | RequestNameFlags::DoNotQueue;
    if replace {
        flags |= RequestNameFlags::ReplaceExisting;
    }
    match conn.request_name_with_flags(NAME, flags)? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => {}
        _ => {
            return Err(zbus::Error::Failure(format!(
                "{NAME} is owned by another daemon{}",
                if replace { " that won't give it up (stop it first)" } else { "; pass --replace to take it" }
            )));
        }
    }
    // Losing the name to a newer daemon quits this one.
    let lost = conn.clone();
    std::thread::spawn(move || {
        let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(&lost) else { return };
        let Ok(mut signals) = proxy.receive_name_lost() else { return };
        if signals.any(|s| s.args().is_ok_and(|a| a.name == NAME)) {
            ui.quit();
        }
    });
    let (tx, rx) = mpsc::channel::<Out>();
    std::thread::spawn(move || {
        for out in rx {
            let _ = match out {
                Out::Closed(id, reason) => conn.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, reason as u32)),
                Out::Action(id, key) => conn.emit_signal(None::<&str>, PATH, NAME, "ActionInvoked", &(id, key)),
            };
        }
    });
    Ok(tx)
}
