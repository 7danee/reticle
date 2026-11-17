//! Configuration model, stored as JSON in `%APPDATA%\reticle\config.json`.
//!
//! A crosshair is a stack of layers (first = bottom). Files from v0.1 with the flat
//! crosshair fields are migrated on load.

use serde::{Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};

use crate::hotkey;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Hotkey that shows/hides the crosshair, e.g. "F8" or "Ctrl+Alt+H". Empty = none.
    pub toggle_hotkey: String,
    /// Monitor index (0-based, enumeration order). `null` = automatic: the monitor of the
    /// last game (profile match or window filling its monitor), otherwise the primary one.
    pub monitor: Option<usize>,
    /// Shift in pixels from the screen center. Positive = right / down.
    pub offset_x: i32,
    pub offset_y: i32,
    /// Hide the overlay from screenshots, recordings and streams.
    pub hide_from_capture: bool,
    /// Show the crosshair only while a program with a profile is in the foreground.
    pub only_in_games: bool,
    /// Notify once per game when it runs in exclusive fullscreen, where overlays are hidden.
    pub warn_fullscreen: bool,
    pub editor_theme: Theme,
    /// Behaviour of the default profile when aiming (right mouse button).
    pub aim: AimMode,
    /// Crosshair of the default profile ("Standard").
    pub crosshair: Crosshair,
    /// Game profiles. The first one whose program matches the foreground window wins.
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            toggle_hotkey: "F8".into(),
            monitor: None,
            offset_x: 0,
            offset_y: 0,
            hide_from_capture: false,
            only_in_games: false,
            warn_fullscreen: true,
            editor_theme: Theme::Dark,
            aim: AimMode::Off,
            crosshair: Crosshair::default(),
            profiles: Vec::new(),
        }
    }
}

/// What happens while the right mouse button aims down sights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AimMode {
    #[default]
    Off,
    /// Hidden while the button is held.
    Hold,
    /// Each press toggles hidden/shown.
    Toggle,
}

impl AimMode {
    pub const ALL: [AimMode; 3] = [AimMode::Off, AimMode::Hold, AimMode::Toggle];

    pub fn label(self) -> &'static str {
        match self {
            AimMode::Off => "Immer zeigen",
            AimMode::Hold => "Ausblenden, solange gehalten",
            AimMode::Toggle => "Ein/aus je Klick",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    /// Program file names, case-insensitive; `*` and `?` work as wildcards.
    pub programs: Vec<String>,
    pub offset_x: i32,
    pub offset_y: i32,
    pub aim: AimMode,
    pub crosshair: Crosshair,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Neues Profil".into(),
            programs: Vec::new(),
            offset_x: 0,
            offset_y: 0,
            aim: AimMode::Off,
            crosshair: Crosshair::default(),
        }
    }
}

/// Everything that can differ per profile, borrowed from the config.
pub struct Look<'a> {
    pub crosshair: &'a Crosshair,
    pub offset: (i32, i32),
    pub aim: AimMode,
}

/// Known games for the "add profile" menu: (name, program patterns).
pub const GAME_TEMPLATES: &[(&str, &[&str])] = &[
    ("Counter-Strike 2", &["cs2.exe"]),
    ("Valorant", &["VALORANT-Win64-Shipping.exe"]),
    ("PUBG", &["TslGame.exe"]),
    ("GTA V", &["GTA5.exe", "GTA5_Enhanced.exe"]),
    ("FiveM", &["FiveM_*GTAProcess.exe"]),
    ("ROB RUN", &[]),
];

/// Case-insensitive match with `*` (any run) and `?` (one character).
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.trim().to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len() && !p.is_empty()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Crosshair {
    pub layers: Vec<Layer>,
}

impl Default for Crosshair {
    fn default() -> Self {
        Self { layers: vec![Layer::Cross(CrossLayer::default())] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Layer {
    Cross(CrossLayer),
    Dot(DotLayer),
    Circle(CircleLayer),
    Chevron(ChevronLayer),
    Image(ImageLayer),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Outline {
    pub enabled: bool,
    pub thickness: u32,
    /// "#RRGGBB" or "#RRGGBBAA".
    pub color: String,
}

impl Outline {
    fn on() -> Self {
        Self { enabled: true, ..Self::default() }
    }
}

impl Default for Outline {
    fn default() -> Self {
        Self { enabled: false, thickness: 1, color: "#000000".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrossLayer {
    pub visible: bool,
    pub color: String,
    pub thickness: u32,
    /// Length of the horizontal arms, and of the vertical ones unless `vertical_length` is set.
    pub length: u32,
    /// Separate length for the vertical arms. `None` = same as `length`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_length: Option<u32>,
    /// Distance between center and arms. May be negative.
    pub gap: i32,
    /// Hides the top arm.
    pub t_style: bool,
    pub outline: Outline,
}

impl CrossLayer {
    pub fn vertical(&self) -> u32 {
        self.vertical_length.unwrap_or(self.length)
    }
}

impl Default for CrossLayer {
    fn default() -> Self {
        Self {
            visible: true,
            color: DEFAULT_COLOR.into(),
            thickness: 2,
            length: 6,
            vertical_length: None,
            gap: 3,
            t_style: false,
            outline: Outline::on(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DotLayer {
    pub visible: bool,
    pub color: String,
    pub size: u32,
    pub outline: Outline,
}

impl Default for DotLayer {
    fn default() -> Self {
        Self { visible: true, color: DEFAULT_COLOR.into(), size: 2, outline: Outline::on() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CircleLayer {
    pub visible: bool,
    pub color: String,
    /// Distance from center to the middle of the ring.
    pub radius: u32,
    pub thickness: u32,
    pub outline: Outline,
}

impl Default for CircleLayer {
    fn default() -> Self {
        Self { visible: true, color: DEFAULT_COLOR.into(), radius: 12, thickness: 2, outline: Outline::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChevronLayer {
    pub visible: bool,
    pub color: String,
    /// Length of each arm.
    pub size: u32,
    pub thickness: u32,
    /// Opening angle in degrees.
    pub angle: u32,
    /// Distance of the tip below the center. May be negative.
    pub gap: i32,
    pub outline: Outline,
}

impl Default for ChevronLayer {
    fn default() -> Self {
        Self {
            visible: true,
            color: DEFAULT_COLOR.into(),
            size: 8,
            thickness: 2,
            angle: 90,
            gap: 0,
            outline: Outline::on(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageLayer {
    pub visible: bool,
    /// File name inside `%APPDATA%\reticle\images\` or an absolute path. PNG or SVG.
    pub file: String,
    pub scale: f32,
    pub opacity: f32,
}

impl Default for ImageLayer {
    fn default() -> Self {
        Self { visible: true, file: String::new(), scale: 1.0, opacity: 1.0 }
    }
}

pub const DEFAULT_COLOR: &str = "#00FF66";

pub const LIMIT_THICKNESS: (u32, u32) = (1, 32);
pub const LIMIT_LENGTH: (u32, u32) = (0, 200);
pub const LIMIT_GAP: (i32, i32) = (-50, 100);
pub const LIMIT_DOT: (u32, u32) = (1, 32);
pub const LIMIT_OUTLINE: (u32, u32) = (1, 8);
pub const LIMIT_RADIUS: (u32, u32) = (1, 200);
pub const LIMIT_CHEVRON: (u32, u32) = (2, 100);
pub const LIMIT_ANGLE: (u32, u32) = (20, 160);
pub const LIMIT_SCALE: (f32, f32) = (0.05, 8.0);
pub const LIMIT_OFFSET: (i32, i32) = (-2000, 2000);

impl Layer {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Layer::Cross(_) => "Kreuz",
            Layer::Dot(_) => "Punkt",
            Layer::Circle(_) => "Kreis",
            Layer::Chevron(_) => "Chevron",
            Layer::Image(_) => "Bild",
        }
    }

    pub fn visible(&self) -> bool {
        match self {
            Layer::Cross(l) => l.visible,
            Layer::Dot(l) => l.visible,
            Layer::Circle(l) => l.visible,
            Layer::Chevron(l) => l.visible,
            Layer::Image(l) => l.visible,
        }
    }

    pub fn visible_mut(&mut self) -> &mut bool {
        match self {
            Layer::Cross(l) => &mut l.visible,
            Layer::Dot(l) => &mut l.visible,
            Layer::Circle(l) => &mut l.visible,
            Layer::Chevron(l) => &mut l.visible,
            Layer::Image(l) => &mut l.visible,
        }
    }

    /// Main color, if the layer has one.
    pub fn color(&self) -> Option<&str> {
        match self {
            Layer::Cross(l) => Some(&l.color),
            Layer::Dot(l) => Some(&l.color),
            Layer::Circle(l) => Some(&l.color),
            Layer::Chevron(l) => Some(&l.color),
            Layer::Image(_) => None,
        }
    }

    fn colors(&self) -> Vec<(&str, &'static str)> {
        match self {
            Layer::Cross(l) => vec![(&l.color, "Farbe"), (&l.outline.color, "Konturfarbe")],
            Layer::Dot(l) => vec![(&l.color, "Farbe"), (&l.outline.color, "Konturfarbe")],
            Layer::Circle(l) => vec![(&l.color, "Farbe"), (&l.outline.color, "Konturfarbe")],
            Layer::Chevron(l) => vec![(&l.color, "Farbe"), (&l.outline.color, "Konturfarbe")],
            Layer::Image(_) => vec![],
        }
    }
}

impl Crosshair {
    pub fn validate(&self) -> Result<(), String> {
        for (i, layer) in self.layers.iter().enumerate() {
            for (color, label) in layer.colors() {
                Rgba::parse(color).ok_or_else(|| {
                    format!(
                        "Ebene {} ({}): {label} \"{color}\" ist ungültig (Format #RRGGBB oder #RRGGBBAA)",
                        i + 1,
                        layer.kind_name()
                    )
                })?;
            }
        }
        Ok(())
    }
}

impl Config {
    /// Checks everything that cannot be clamped silently. Returns a German message for the user.
    pub fn validate(&self) -> Result<(), String> {
        self.crosshair.validate()?;
        for p in &self.profiles {
            p.crosshair.validate().map_err(|e| format!("Profil \"{}\": {e}", p.name))?;
        }
        hotkey::parse(&self.toggle_hotkey).map_err(|e| format!("Hotkey \"{}\": {e}", self.toggle_hotkey))?;
        Ok(())
    }

    /// Index of the first profile whose program list matches `program` (a file name).
    pub fn match_profile(&self, program: &str) -> Option<usize> {
        self.profiles.iter().position(|p| p.programs.iter().any(|pat| wildcard_match(pat, program)))
    }

    /// Settings of a profile, or of the default when `None` or out of range.
    pub fn look(&self, profile: Option<usize>) -> Look<'_> {
        let clamp = |x: i32, y: i32| {
            (x.clamp(LIMIT_OFFSET.0, LIMIT_OFFSET.1), y.clamp(LIMIT_OFFSET.0, LIMIT_OFFSET.1))
        };
        match profile.and_then(|i| self.profiles.get(i)) {
            Some(p) => Look { crosshair: &p.crosshair, offset: clamp(p.offset_x, p.offset_y), aim: p.aim },
            None => Look { crosshair: &self.crosshair, offset: clamp(self.offset_x, self.offset_y), aim: self.aim },
        }
    }
}

/// v0.1 stored a single cross with an optional dot directly in `crosshair`.
#[derive(Deserialize)]
#[serde(default)]
struct LegacyCrosshair {
    color: String,
    thickness: u32,
    length: u32,
    gap: i32,
    t_style: bool,
    dot: bool,
    dot_size: u32,
    outline: bool,
    outline_thickness: u32,
    outline_color: String,
}

impl Default for LegacyCrosshair {
    fn default() -> Self {
        let cross = CrossLayer::default();
        Self {
            color: cross.color,
            thickness: cross.thickness,
            length: cross.length,
            gap: cross.gap,
            t_style: false,
            dot: false,
            dot_size: 2,
            outline: true,
            outline_thickness: 1,
            outline_color: "#000000".into(),
        }
    }
}

impl From<LegacyCrosshair> for Crosshair {
    fn from(l: LegacyCrosshair) -> Self {
        let outline = Outline { enabled: l.outline, thickness: l.outline_thickness, color: l.outline_color };
        let mut layers = vec![Layer::Cross(CrossLayer {
            visible: true,
            color: l.color.clone(),
            thickness: l.thickness,
            length: l.length,
            vertical_length: None,
            gap: l.gap,
            t_style: l.t_style,
            outline: outline.clone(),
        })];
        if l.dot {
            layers.push(Layer::Dot(DotLayer { visible: true, color: l.color, size: l.dot_size, outline }));
        }
        Self { layers }
    }
}

impl<'de> Deserialize<'de> for Crosshair {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        struct Layers {
            layers: Vec<Layer>,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("layers").is_some() {
            let parsed: Layers = serde_json::from_value(value).map_err(D::Error::custom)?;
            Ok(Self { layers: parsed.layers })
        } else {
            let legacy: LegacyCrosshair = serde_json::from_value(value).map_err(D::Error::custom)?;
            Ok(legacy.into())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.trim().strip_prefix('#')?;
        if !hex.is_ascii() || (hex.len() != 6 && hex.len() != 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if hex.len() == 8 { byte(6)? } else { 255 },
        })
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub crosshair: Crosshair,
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("reticle")
}

pub fn path() -> PathBuf {
    dir().join("config.json")
}

pub fn images_dir() -> PathBuf {
    dir().join("images")
}

pub fn presets_path() -> PathBuf {
    dir().join("presets.json")
}

/// Resolves an image layer's file name.
pub fn image_path(file: &str) -> PathBuf {
    let p = Path::new(file);
    if p.is_absolute() { p.to_path_buf() } else { images_dir().join(p) }
}

pub fn parse(text: &str) -> Result<Config, String> {
    let config: Config =
        serde_json::from_str(text).map_err(|e| format!("Config ist kein gültiges JSON: {e}"))?;
    config.validate()?;
    Ok(config)
}

/// Loads the config. A missing file is created with defaults.
pub fn load() -> Result<Config, String> {
    let path = path();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(text.trim_start_matches('\u{feff}')),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            save(&config)?;
            Ok(config)
        }
        Err(e) => Err(format!("Config konnte nicht gelesen werden: {e}")),
    }
}

pub fn save(config: &Config) -> Result<(), String> {
    write_json(&path(), config)
}

/// Writes via a temp file and rename, so readers never see half a file.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| format!("Ordner konnte nicht angelegt werden: {e}"))?;
    let text = serde_json::to_string_pretty(value).expect("value serializes") + "\n";
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Datei konnte nicht gespeichert werden: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Datei konnte nicht gespeichert werden: {e}"))
}

pub fn load_presets() -> Vec<Preset> {
    std::fs::read_to_string(presets_path())
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save_presets(presets: &[Preset]) -> Result<(), String> {
    write_json(&presets_path(), &presets)
}

/// Presets that ship with reticle.
pub fn builtin_presets() -> Vec<Preset> {
    let white = "#FFFFFF".to_string();
    let cross = |thickness, length, gap, color: &str| CrossLayer {
        thickness,
        length,
        gap,
        color: color.into(),
        ..CrossLayer::default()
    };
    vec![
        Preset { name: "Klassisch".into(), crosshair: Crosshair::default() },
        Preset {
            name: "Dünn".into(),
            crosshair: Crosshair { layers: vec![Layer::Cross(cross(1, 5, 2, "#00FFFF"))] },
        },
        Preset {
            name: "Punkt".into(),
            crosshair: Crosshair { layers: vec![Layer::Dot(DotLayer { size: 4, ..DotLayer::default() })] },
        },
        Preset {
            name: "T-Form".into(),
            crosshair: Crosshair {
                layers: vec![Layer::Cross(CrossLayer { t_style: true, ..cross(2, 7, 2, "#FFFF00") })],
            },
        },
        Preset {
            name: "Kreis".into(),
            crosshair: Crosshair {
                layers: vec![
                    Layer::Circle(CircleLayer { color: white.clone(), radius: 14, thickness: 2, ..CircleLayer::default() }),
                    Layer::Dot(DotLayer { color: white.clone(), size: 2, ..DotLayer::default() }),
                ],
            },
        },
        Preset {
            name: "Chevron".into(),
            crosshair: Crosshair {
                layers: vec![Layer::Chevron(ChevronLayer { color: "#FF3355".into(), ..ChevronLayer::default() })],
            },
        },
        Preset {
            name: "Kreuz + Punkt".into(),
            crosshair: Crosshair {
                layers: vec![
                    Layer::Cross(cross(2, 5, 4, &white)),
                    Layer::Dot(DotLayer { color: white.clone(), size: 2, ..DotLayer::default() }),
                ],
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors() {
        assert_eq!(Rgba::parse("#00FF66"), Some(Rgba { r: 0, g: 255, b: 0x66, a: 255 }));
        assert_eq!(Rgba::parse("#00ff6680"), Some(Rgba { r: 0, g: 255, b: 0x66, a: 0x80 }));
        assert_eq!(Rgba::parse("00FF66"), None);
        assert_eq!(Rgba::parse("#00FF6"), None);
        assert_eq!(Rgba::parse("#GGFF66"), None);
        assert_eq!(Rgba::parse("#ääFF"), None);
        assert_eq!(Rgba { r: 1, g: 2, b: 3, a: 255 }.to_hex(), "#010203");
        assert_eq!(Rgba { r: 1, g: 2, b: 3, a: 4 }.to_hex(), "#01020304");
    }

    #[test]
    fn default_round_trips() {
        let text = serde_json::to_string_pretty(&Config::default()).unwrap();
        assert_eq!(parse(&text).unwrap(), Config::default());
    }

    #[test]
    fn all_layer_types_round_trip() {
        let config = Config {
            crosshair: Crosshair {
                layers: vec![
                    Layer::Cross(CrossLayer::default()),
                    Layer::Dot(DotLayer::default()),
                    Layer::Circle(CircleLayer::default()),
                    Layer::Chevron(ChevronLayer::default()),
                    Layer::Image(ImageLayer { file: "x.png".into(), ..ImageLayer::default() }),
                ],
            },
            ..Config::default()
        };
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(parse(&text).unwrap(), config);
    }

    #[test]
    fn migrates_v01_config() {
        let config = parse(
            r##"{ "toggle_hotkey": "F9", "crosshair": { "color": "#FF0000", "gap": -2, "dot": true, "dot_size": 3, "outline": false } }"##,
        )
        .unwrap();
        assert_eq!(config.toggle_hotkey, "F9");
        let [Layer::Cross(cross), Layer::Dot(dot)] = config.crosshair.layers.as_slice() else {
            panic!("expected cross + dot, got {:?}", config.crosshair.layers);
        };
        assert_eq!((cross.color.as_str(), cross.gap, cross.outline.enabled), ("#FF0000", -2, false));
        assert_eq!((dot.color.as_str(), dot.size), ("#FF0000", 3));
    }

    #[test]
    fn partial_layer_uses_defaults() {
        let config = parse(r#"{ "crosshair": { "layers": [ { "type": "circle", "radius": 30 } ] } }"#).unwrap();
        let [Layer::Circle(c)] = config.crosshair.layers.as_slice() else { panic!() };
        assert_eq!((c.radius, c.thickness), (30, 2));
        assert_eq!(config.editor_theme, Theme::Dark);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(parse(r#"{ "crosshair": { "layers": [ { "type": "dot", "color": "green" } ] } }"#).is_err());
        assert!(parse(r#"{ "crosshair": { "layers": [ { "type": "star" } ] } }"#).is_err());
        assert!(parse(r#"{ "toggle_hotkey": "Ctrl+Banana" }"#).is_err());
        assert!(parse("{ nope").is_err());
        let err = parse(r##"{ "crosshair": { "layers": [ {"type":"dot"}, { "type": "cross", "outline": { "color": "#12" } } ] } }"##)
            .unwrap_err();
        assert!(err.contains("Ebene 2") && err.contains("Konturfarbe"), "{err}");
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("cs2.exe", "CS2.EXE"));
        assert!(wildcard_match("FiveM_*GTAProcess.exe", "FiveM_GTAProcess.exe"));
        assert!(wildcard_match("FiveM_*GTAProcess.exe", "FiveM_b3258_GTAProcess.exe"));
        assert!(wildcard_match("game?.exe", "game2.exe"));
        assert!(!wildcard_match("game?.exe", "game.exe"));
        assert!(!wildcard_match("cs2.exe", "cs2.exe.bak"));
        assert!(!wildcard_match("", "cs2.exe"));
        assert!(wildcard_match("*", "anything.exe"));
    }

    #[test]
    fn profiles_match_and_fall_back() {
        let mut config = Config { offset_x: 3, ..Config::default() };
        config.profiles.push(Profile {
            name: "CS2".into(),
            programs: vec!["cs2.exe".into()],
            offset_y: -2,
            aim: AimMode::Hold,
            ..Profile::default()
        });
        config.profiles.push(Profile { name: "Alle".into(), programs: vec!["*".into()], ..Profile::default() });
        assert_eq!(config.match_profile("cs2.exe"), Some(0));
        assert_eq!(config.match_profile("notepad.exe"), Some(1));
        let look = config.look(Some(0));
        assert_eq!((look.offset, look.aim), ((0, -2), AimMode::Hold));
        assert_eq!(config.look(None).offset, (3, 0));
        assert_eq!(config.look(Some(9)).offset, (3, 0));
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(parse(&text).unwrap(), config);
    }

    #[test]
    fn profile_errors_name_the_profile() {
        let err = parse(r#"{ "profiles": [ { "name": "CS2", "crosshair": { "layers": [ { "type": "dot", "color": "x" } ] } } ] }"#)
            .unwrap_err();
        assert!(err.contains("CS2"), "{err}");
    }

    #[test]
    fn builtin_presets_are_valid() {
        for p in builtin_presets() {
            p.crosshair.validate().unwrap();
        }
    }
}
