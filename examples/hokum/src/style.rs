//! What can be customised, its presets, and the settings file both processes read
//! (`$XDG_CONFIG_HOME/hokum/settings.json`). Unknown or missing fields take their defaults, so
//! older and newer files load.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corners {
    Sharp,
    Rounded,
    Soft,
    Round,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Density {
    Compact,
    Comfortable,
    Spacious,
}

/// How a card says which app sent it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mark {
    Letter,
    Dot,
    Stripe,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Depth {
    Flat,
    Soft,
    Deep,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Width {
    Narrow,
    Medium,
    Wide,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Style {
    pub corners: Corners,
    pub density: Density,
    pub mark: Mark,
    pub depth: Depth,
    pub width: Width,
    pub border: bool,
}

impl Style {
    pub fn radius(&self) -> f32 {
        match self.corners {
            Corners::Sharp => 4.0,
            Corners::Rounded => 10.0,
            Corners::Soft => 16.0,
            Corners::Round => 24.0,
        }
    }

    /// The badge follows the card's corners at its own size.
    pub fn badge_radius(&self) -> f32 {
        match self.corners {
            Corners::Sharp => 4.0,
            Corners::Rounded => 6.0,
            Corners::Soft => 7.0,
            Corners::Round => 10.0,
        }
    }

    /// Card padding, the gap inside a card, and the gap between cards.
    pub fn spacing(&self) -> (f32, f32, f32) {
        match self.density {
            Density::Compact => (10.0, 4.0, 8.0),
            Density::Comfortable => (12.0, 6.0, 10.0),
            Density::Spacious => (16.0, 8.0, 12.0),
        }
    }

    pub fn card_width(&self) -> f32 {
        match self.width {
            Width::Narrow => 320.0,
            Width::Medium => 360.0,
            Width::Wide => 420.0,
        }
    }
}

/// Four looks to start from; changing any one control makes the style custom.
pub const PRESETS: [(&str, Style); 4] = [
    (
        "Soft",
        Style {
            corners: Corners::Soft,
            density: Density::Comfortable,
            mark: Mark::Letter,
            depth: Depth::Soft,
            width: Width::Medium,
            border: true,
        },
    ),
    (
        "Crisp",
        Style {
            corners: Corners::Sharp,
            density: Density::Compact,
            mark: Mark::Dot,
            depth: Depth::Flat,
            width: Width::Narrow,
            border: true,
        },
    ),
    (
        "Capsule",
        Style {
            corners: Corners::Round,
            density: Density::Spacious,
            mark: Mark::Letter,
            depth: Depth::Deep,
            width: Width::Wide,
            border: false,
        },
    ),
    (
        "Ribbon",
        Style {
            corners: Corners::Rounded,
            density: Density::Comfortable,
            mark: Mark::Stripe,
            depth: Depth::Soft,
            width: Width::Medium,
            border: true,
        },
    ),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl Position {
    pub fn right(self) -> bool {
        matches!(self, Position::TopRight | Position::BottomRight)
    }
    pub fn bottom(self) -> bool {
        matches!(self, Position::BottomRight | Position::BottomLeft)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Linger {
    Short,
    Medium,
    Long,
}

impl Linger {
    pub fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Linger::Short => 4,
            Linger::Medium => 6,
            Linger::Long => 10,
        })
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub style: Style,
    pub position: Position,
    pub appearance: Appearance,
    /// How long ordinary notifications stay when the app doesn't say.
    pub linger: Linger,
    pub max_shown: usize,
    pub pause_on_hover: bool,
    /// Only critical notifications show.
    pub do_not_disturb: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            style: PRESETS[0].1,
            position: Position::TopRight,
            appearance: Appearance::System,
            linger: Linger::Medium,
            max_shown: 5,
            pause_on_hover: true,
            do_not_disturb: false,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        PRESETS[0].1
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("hokum").join("settings.json")
}

impl Settings {
    /// The saved settings, or the defaults when there are none (or they don't parse).
    pub fn load() -> Settings {
        let p = path();
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!("hokum: ignoring {}: {e}", p.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    /// Writes atomically: a temp file renamed over the old one.
    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(std::io::Error::other)? + "\n")?;
        std::fs::rename(tmp, p)
    }

    pub fn preset(&self) -> Option<usize> {
        PRESETS.iter().position(|(_, s)| *s == self.style)
    }
}
