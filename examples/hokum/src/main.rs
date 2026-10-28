//! Hokum (hokum.app): a freedesktop notification daemon drawn with Rune.
//!
//! ```sh
//! hokum                     # the settings window (one at a time)
//! hokum service [--replace] # the daemon: owns org.freedesktop.Notifications (systemd runs this)
//! hokum --version
//! hokum --snap /tmp/settings.png --scene hokum            # headless settings, no D-Bus
//! hokum service --snap /tmp/cards.png --scene demo        # headless cards, no D-Bus
//! ```
//!
//! Two processes, because a Rune app has one surface: the daemon draws the cards on a
//! click-through layer surface, and the settings window is an ordinary window. They share
//! `$XDG_CONFIG_HOME/hokum/settings.json`; the window saves it and calls
//! `app.hokum.Daemon.Reload` so the overlay restyles live.

mod bus;
mod card;
mod daemon;
mod settings;
mod style;

const HELP: &str = "usage: hokum [service [--replace]] [--version] [--help]
  hokum                  open the settings window
  hokum service          run the notification daemon (what hokum.service starts)
    --replace            take org.freedesktop.Notifications from another daemon
  --version              print the version
Rune's tooling flags (--snap, --scene, --size, --theme, ...) render headless.";

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--version" || a == "-V") {
        println!("hokum {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    let headless = argv.iter().any(|a| {
        matches!(a.as_str(), "--snap" | "--tree" | "--trace" | "--frames" | "--strip" | "--list-scenes" | "--first-pixels")
    });
    if argv.first().is_some_and(|a| a == "service") {
        daemon::run(headless, argv.iter().any(|a| a == "--replace"));
    } else {
        settings::run(headless);
    }
}
