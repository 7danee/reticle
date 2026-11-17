//! Settings window (`reticle.exe --editor`), a separate process so it costs nothing once closed.
//! Compact layout: tabs on the left, preview on the right. Every change is written to the config
//! right away and the overlay is told to reload.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense,
    Stroke, TextureHandle, TextureOptions, Ui, Vec2,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
use windows::core::PCWSTR;

use crate::app::{CONTROL_CLASS, WM_RELOAD};
use crate::autostart;
use crate::clipboard;
use crate::config::{
    self, AimMode, ChevronLayer, CircleLayer, Config, CrossLayer, Crosshair, DotLayer, GAME_TEMPLATES, ImageLayer,
    LIMIT_ANGLE, LIMIT_CHEVRON, LIMIT_DOT, LIMIT_GAP, LIMIT_LENGTH, LIMIT_OFFSET, LIMIT_OUTLINE, LIMIT_RADIUS,
    LIMIT_SCALE, LIMIT_THICKNESS, Layer, Outline, Preset, Profile, Rgba, Theme,
};
use crate::hotkey;
use crate::logo;
use crate::overlay::{self, Monitor};
use crate::process;
use crate::render;
use crate::share;
use crate::theme::{self, Palette, heading};

pub const WINDOW_TITLE: &str = "reticle – Einstellungen";

/// Minimum time between two config writes while dragging a slider.
const SAVE_INTERVAL: Duration = Duration::from_millis(40);
const THUMB: f32 = 40.0;
const PREVIEW_HEIGHT: f32 = 150.0;

pub fn run() -> eframe::Result {
    let (config, load_error) = match config::load() {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e)),
    };
    let icon = logo::render(64, logo::ACCENT, true);
    let viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([340.0, 600.0])
        .with_min_inner_size([320.0, 420.0])
        .with_icon(Arc::new(egui::IconData {
            rgba: render::to_rgba_straight(&icon),
            width: icon.width(),
            height: icon.height(),
        }));
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "reticle",
        options,
        Box::new(move |cc| {
            theme::setup_fonts(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx, config.editor_theme);
            Ok(Box::new(Editor::new(config, load_error)))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Crosshair,
    Presets,
    Profile,
    Options,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Crosshair, Tab::Presets, Tab::Profile, Tab::Options];

    fn label(self) -> String {
        let (icon, text) = match self {
            Tab::Crosshair => (theme::I_CROSSHAIRS, "Form"),
            Tab::Presets => (theme::I_BOOKMARK, "Presets"),
            Tab::Profile => (theme::I_GAMEPAD, "Profil"),
            Tab::Options => (theme::I_GEAR, "Optionen"),
        };
        format!("{icon} {text}")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backdrop {
    Dark,
    Light,
    Checker,
    Grass,
    Sky,
}

impl Backdrop {
    const ALL: [Backdrop; 5] = [Backdrop::Dark, Backdrop::Light, Backdrop::Checker, Backdrop::Grass, Backdrop::Sky];

    fn label(self) -> &'static str {
        match self {
            Backdrop::Dark => "Dunkel",
            Backdrop::Light => "Hell",
            Backdrop::Checker => "Karo",
            Backdrop::Grass => "Wiese",
            Backdrop::Sky => "Himmel",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Backdrop::Dark => Color32::from_rgb(0x22, 0x23, 0x26),
            Backdrop::Light => Color32::from_rgb(0xE4, 0xE6, 0xEA),
            Backdrop::Checker => Color32::from_rgb(0x80, 0x80, 0x80),
            Backdrop::Grass => Color32::from_rgb(0x4B, 0x6B, 0x38),
            Backdrop::Sky => Color32::from_rgb(0x8F, 0xB4, 0xD9),
        }
    }
}

struct Editor {
    config: Config,
    /// Config as last written (or loaded); used to detect changes.
    saved: Config,
    tab: Tab,
    /// Profile being edited, `None` = Standard.
    profile: Option<usize>,
    selected: usize,
    dirty_since: Option<Instant>,
    last_save: Instant,
    own_stamp: Option<SystemTime>,
    last_check: Instant,
    message: Option<(String, bool)>,
    hotkey_text: String,
    program_text: String,
    running: Vec<String>,
    monitors: Vec<Monitor>,
    overlay_running: bool,
    autostart: bool,
    zoom: u32,
    backdrop: Backdrop,
    guides: bool,
    preview: Option<TextureHandle>,
    preview_size: u32,
    preview_for: Option<Crosshair>,
    presets: Vec<Preset>,
    builtin: Vec<Preset>,
    thumbs: Vec<TextureHandle>,
    preset_name: String,
    share_text: String,
    applied_theme: Theme,
}

impl Editor {
    fn new(config: Config, load_error: Option<String>) -> Self {
        let message = load_error.map(|e| (format!("{e} Die nächste Änderung überschreibt die Datei."), true));
        Self {
            hotkey_text: config.toggle_hotkey.clone(),
            saved: config.clone(),
            applied_theme: config.editor_theme,
            config,
            tab: Tab::Crosshair,
            profile: None,
            selected: 0,
            dirty_since: None,
            last_save: Instant::now() - SAVE_INTERVAL,
            own_stamp: config_stamp(),
            last_check: Instant::now(),
            message,
            program_text: String::new(),
            running: Vec::new(),
            monitors: overlay::monitors(),
            overlay_running: overlay_running(),
            autostart: autostart::is_enabled(),
            zoom: 5,
            backdrop: Backdrop::Dark,
            guides: false,
            preview: None,
            preview_size: 0,
            preview_for: None,
            presets: config::load_presets(),
            builtin: config::builtin_presets(),
            thumbs: Vec::new(),
            preset_name: String::new(),
            share_text: String::new(),
        }
    }

    fn palette(&self) -> Palette {
        theme::palette(self.config.editor_theme)
    }

    // --- access to the profile being edited ---------------------------------------------------

    fn crosshair(&self) -> &Crosshair {
        self.config.look(self.profile).crosshair
    }

    fn crosshair_mut(&mut self) -> &mut Crosshair {
        match self.profile.and_then(|i| self.config.profiles.get_mut(i)) {
            Some(p) => &mut p.crosshair,
            None => &mut self.config.crosshair,
        }
    }

    fn look_mut(&mut self) -> (&mut i32, &mut i32, &mut AimMode) {
        match self.profile.and_then(|i| self.config.profiles.get_mut(i)) {
            Some(p) => (&mut p.offset_x, &mut p.offset_y, &mut p.aim),
            None => (&mut self.config.offset_x, &mut self.config.offset_y, &mut self.config.aim),
        }
    }

    fn profile_name(&self) -> String {
        self.profile.and_then(|i| self.config.profiles.get(i)).map_or("Standard".into(), |p| p.name.clone())
    }

    fn select_profile(&mut self, profile: Option<usize>) {
        self.profile = profile.filter(|&i| i < self.config.profiles.len());
        self.selected = self.crosshair().layers.len().saturating_sub(1);
    }

    // --- persistence ----------------------------------------------------------------------------

    /// Writes pending changes (rate-limited) and keeps an eye on the file and the overlay.
    fn housekeeping(&mut self, ctx: &egui::Context) {
        if self.config != self.saved && self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
        if self.dirty_since.is_some() {
            if self.last_save.elapsed() >= SAVE_INTERVAL {
                self.save();
            } else {
                ctx.request_repaint_after(SAVE_INTERVAL);
            }
        }
        if self.last_check.elapsed() >= Duration::from_secs(1) {
            self.last_check = Instant::now();
            self.overlay_running = overlay_running();
            let stamp = config_stamp();
            if self.dirty_since.is_none() && stamp != self.own_stamp {
                // Changed outside the editor (text editor, tray menu).
                self.own_stamp = stamp;
                if let Ok(c) = config::load() {
                    self.hotkey_text = c.toggle_hotkey.clone();
                    self.saved = c.clone();
                    self.config = c;
                    self.select_profile(self.profile);
                }
            }
        }
        // No periodic repaint: an idle window draws nothing. The checks above run on the next
        // repaint, which any mouse movement or focus change triggers.
    }

    fn save(&mut self) {
        if let Err(e) = self.config.validate() {
            self.message = Some((e, true));
            self.dirty_since = None;
            self.saved = self.config.clone();
            return;
        }
        match config::save(&self.config) {
            Ok(()) => {
                self.saved = self.config.clone();
                self.own_stamp = config_stamp();
                if matches!(self.message, Some((_, true))) {
                    self.message = None;
                }
                notify_overlay();
            }
            Err(e) => self.message = Some((e, true)),
        }
        self.last_save = Instant::now();
        self.dirty_since = None;
    }

    fn refresh_preview(&mut self, ctx: &egui::Context) {
        if self.preview.is_some() && self.preview_for.as_ref() == Some(self.crosshair()) {
            return;
        }
        let crosshair = self.crosshair().clone();
        let pixmap = render::render(&crosshair);
        let image = ColorImage::from_rgba_premultiplied([pixmap.width() as usize, pixmap.height() as usize], pixmap.data());
        match &mut self.preview {
            Some(tex) => tex.set(image, TextureOptions::NEAREST),
            None => self.preview = Some(ctx.load_texture("preview", image, TextureOptions::NEAREST)),
        }
        self.preview_size = pixmap.width();
        self.preview_for = Some(crosshair);
    }

    fn refresh_thumbs(&mut self, ctx: &egui::Context) {
        if self.thumbs.len() == self.builtin.len() + self.presets.len() {
            return;
        }
        self.thumbs = self
            .builtin
            .iter()
            .chain(self.presets.iter())
            .enumerate()
            .map(|(i, p)| {
                let pm = render::render(&p.crosshair);
                let image = ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
                ctx.load_texture(format!("thumb-{i}"), image, TextureOptions::NEAREST)
            })
            .collect();
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != self.config.editor_theme {
            theme::apply(&ctx, self.config.editor_theme);
            self.applied_theme = self.config.editor_theme;
        }
        self.refresh_preview(&ctx);
        if self.tab == Tab::Presets {
            self.refresh_thumbs(&ctx);
        }
        let p = self.palette();

        egui::Panel::top("top")
            .frame(Frame::new().fill(p.panel).inner_margin(Margin::symmetric(10, 5)).stroke(Stroke::new(1.0, p.line)))
            .show(ui, |ui| self.top_bar(ui));

        if let Some((msg, error)) = self.message.clone() {
            egui::Panel::bottom("message")
                .frame(Frame::new().fill(p.panel).inner_margin(Margin::symmetric(10, 4)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (icon, color) = if error { (theme::I_WARN, p.danger) } else { (theme::I_CHECK, p.accent) };
                        ui.label(RichText::new(icon).size(10.5).color(color));
                        ui.label(RichText::new(msg).size(11.0).color(if error { p.danger } else { p.text }));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button(ui, theme::I_XMARK, p.muted).clicked() {
                                self.message = None;
                            }
                        });
                    });
                });
        }

        // One narrow column: preview on top, tabs below, the tab content scrolls.
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 8, right: 4, top: 8, bottom: 6 }))
            .show(ui, |ui| {
                ui.scope(|ui| {
                    ui.set_width(ui.available_width() - 4.0);
                    self.preview_area(ui);
                    ui.add_space(2.0);
                    self.tabs_row(ui);
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(ui.available_width() - 10.0);
                    match self.tab {
                        Tab::Crosshair => self.crosshair_tab(ui),
                        Tab::Presets => self.presets_tab(ui),
                        Tab::Profile => self.profile_tab(ui),
                        Tab::Options => self.options_tab(ui),
                    }
                });
            });

        self.housekeeping(&ctx);
    }
}

// -------------------------------------------------------------------------------------------------
// Top bar

impl Editor {
    fn top_bar(&mut self, ui: &mut Ui) {
        let p = self.palette();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new("r").font(FontId::new(17.0, heading())));
            ui.label(RichText::new(".").font(FontId::new(17.0, heading())).color(p.accent));
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(8.0);

            // Profile picker, as wide as the bar allows.
            let mut chosen = None;
            egui::ComboBox::from_id_salt("profile")
                .selected_text(format!("{}  {}", theme::I_GAMEPAD, truncate(&self.profile_name(), 22)))
                .width((ui.available_width() - 52.0).max(120.0))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.profile.is_none(), "Standard").clicked() {
                        chosen = Some(None);
                    }
                    for (i, prof) in self.config.profiles.iter().enumerate() {
                        if ui.selectable_label(self.profile == Some(i), &prof.name).clicked() {
                            chosen = Some(Some(i));
                        }
                    }
                });
            if let Some(c) = chosen {
                self.select_profile(c);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (icon, tip) = match self.config.editor_theme {
                    Theme::Dark => (theme::I_SUN, "Helles Design"),
                    Theme::Light => (theme::I_MOON, "Dunkles Design"),
                };
                if icon_button(ui, icon, p.muted).on_hover_text(tip).clicked() {
                    self.config.editor_theme = match self.config.editor_theme {
                        Theme::Dark => Theme::Light,
                        Theme::Light => Theme::Dark,
                    };
                }
                if self.overlay_running {
                    ui.label(RichText::new(theme::I_DOT).size(8.0).color(p.accent)).on_hover_text("Overlay läuft");
                } else if ui
                    .add(egui::Button::new(RichText::new(format!("{} Overlay", theme::I_PLAY)).color(p.danger)))
                    .on_hover_text("Overlay läuft nicht – starten")
                    .clicked()
                {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    self.last_check = Instant::now() - Duration::from_millis(700);
                }
            });
        });
    }
}

// -------------------------------------------------------------------------------------------------
// Tabs

impl Editor {
    /// Four equal tabs across the column.
    fn tabs_row(&mut self, ui: &mut Ui) {
        // Columns split the width exactly, so the row never pushes the window wider.
        let mut clicked = None;
        ui.columns(Tab::ALL.len(), |cols| {
            for (col, tab) in cols.iter_mut().zip(Tab::ALL) {
                col.spacing_mut().button_padding.x = 2.0;
                let width = col.available_width();
                let button = egui::Button::selectable(self.tab == tab, tab.label()).min_size(Vec2::new(width, 20.0));
                if col.add_sized([width, 20.0], button).clicked() {
                    clicked = Some(tab);
                }
            }
        });
        if let Some(tab) = clicked {
            self.tab = tab;
            if tab == Tab::Profile {
                self.running = process::running_programs();
            }
        }
    }

    fn crosshair_tab(&mut self, ui: &mut Ui) {
        let p = self.palette();
        let count = self.crosshair().layers.len();
        let mut action: Option<(usize, LayerAction)> = None;
        // Top of the list = topmost layer.
        for i in (0..count).rev() {
            let layer = &self.crosshair().layers[i];
            let selected = i == self.selected;
            let row = Frame::new()
                .fill(if selected { p.accent_dim } else { p.panel_hi })
                .stroke(Stroke::new(1.0, if selected { p.accent } else { p.line }))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Margin::symmetric(6, 1))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let visible = layer.visible();
                        let eye = if visible { theme::I_EYE } else { theme::I_EYE_SLASH };
                        if icon_button(ui, eye, if visible { p.text } else { p.muted })
                            .on_hover_text(if visible { "Ausblenden" } else { "Einblenden" })
                            .clicked()
                        {
                            action = Some((i, LayerAction::ToggleVisible));
                        }
                        ui.label(RichText::new(kind_icon(layer)).size(10.5).color(p.muted));
                        let name = RichText::new(layer_label(layer));
                        ui.label(if visible { name } else { name.color(p.muted) });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button(ui, theme::I_TRASH, p.muted).on_hover_text("Löschen").clicked() {
                                action = Some((i, LayerAction::Delete));
                            }
                            if i > 0 && icon_button(ui, theme::I_DOWN, p.muted).on_hover_text("Nach unten").clicked() {
                                action = Some((i, LayerAction::Down));
                            }
                            if i + 1 < count && icon_button(ui, theme::I_UP, p.muted).on_hover_text("Nach oben").clicked() {
                                action = Some((i, LayerAction::Up));
                            }
                        });
                    });
                });
            // A click elsewhere in the row selects it. Registering the row itself as a clickable
            // widget would sit on top of the buttons and swallow their clicks.
            let row_clicked =
                ui.rect_contains_pointer(row.response.rect) && ui.input(|inp| inp.pointer.primary_clicked());
            if action.is_none() && row_clicked {
                action = Some((i, LayerAction::Select));
            }
        }
        if let Some((i, a)) = action {
            self.layer_action(i, a);
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::I_PLUS).size(10.5).color(p.muted));
            let new_layers: [(&str, &str, MakeLayer); 5] = [
                (theme::I_PLUS, "Kreuz", || Layer::Cross(CrossLayer::default())),
                (theme::I_DOT, "Punkt", || Layer::Dot(DotLayer::default())),
                (theme::I_RING, "Kreis", || Layer::Circle(CircleLayer::default())),
                (theme::I_CHEVRON, "Chevron", || Layer::Chevron(ChevronLayer::default())),
                (theme::I_IMAGE, "Bild (PNG/SVG)", || Layer::Image(ImageLayer::default())),
            ];
            for (icon, label, make) in new_layers {
                if ui.small_button(icon).on_hover_text(format!("{label} hinzufügen")).clicked() {
                    let mut layer = make();
                    if let (Some(color), Some(current)) = (layer_color_mut(&mut layer), self.current_color()) {
                        *color = current;
                    }
                    let is_image = matches!(layer, Layer::Image(_));
                    self.crosshair_mut().layers.push(layer);
                    self.selected = self.crosshair().layers.len() - 1;
                    if is_image {
                        self.pick_image();
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.separator();
        self.properties(ui);
    }

    /// Color of the selected layer, reused for new layers.
    fn current_color(&self) -> Option<String> {
        self.crosshair().layers.get(self.selected).and_then(|l| l.color()).map(str::to_string)
    }

    fn layer_action(&mut self, i: usize, action: LayerAction) {
        let mut selected = self.selected;
        let layers = &mut self.crosshair_mut().layers;
        match action {
            LayerAction::Select => selected = i,
            LayerAction::ToggleVisible => {
                let v = layers[i].visible_mut();
                *v = !*v;
            }
            LayerAction::Up => {
                layers.swap(i, i + 1);
                selected = i + 1;
            }
            LayerAction::Down => {
                layers.swap(i, i - 1);
                selected = i - 1;
            }
            LayerAction::Delete => {
                layers.remove(i);
                if selected >= layers.len() {
                    selected = layers.len().saturating_sub(1);
                } else if selected > i {
                    selected -= 1;
                }
            }
        }
        self.selected = selected;
    }

    fn properties(&mut self, ui: &mut Ui) {
        let p = self.palette();
        let selected = self.selected;
        let Some(layer) = self.crosshair_mut().layers.get_mut(selected) else {
            ui.label(RichText::new("Keine Ebene. Füge oben eine hinzu.").color(p.muted));
            return;
        };
        let mut pick_image = false;
        egui::Grid::new("props").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| match layer {
            Layer::Cross(l) => {
                color_row(ui, "Farbe", &mut l.color);
                int_row(ui, "Dicke", &mut l.thickness, 1..=10, LIMIT_THICKNESS);
                let separate = l.vertical_length.is_some();
                int_row(ui, if separate { "Länge waagr." } else { "Länge" }, &mut l.length, 0..=40, LIMIT_LENGTH);
                if let Some(vertical) = &mut l.vertical_length {
                    int_row(ui, "Länge senkr.", vertical, 0..=40, LIMIT_LENGTH);
                }
                ui.label("");
                let mut split = separate;
                if ui.checkbox(&mut split, "waagerecht/senkrecht getrennt").changed() {
                    l.vertical_length = split.then_some(l.length);
                }
                ui.end_row();
                signed_row(ui, "Abstand", &mut l.gap, -10..=30, LIMIT_GAP);
                ui.label("T-Form");
                ui.checkbox(&mut l.t_style, "oben aus");
                ui.end_row();
                outline_rows(ui, &mut l.outline);
            }
            Layer::Dot(l) => {
                color_row(ui, "Farbe", &mut l.color);
                int_row(ui, "Größe", &mut l.size, 1..=16, LIMIT_DOT);
                outline_rows(ui, &mut l.outline);
            }
            Layer::Circle(l) => {
                color_row(ui, "Farbe", &mut l.color);
                int_row(ui, "Radius", &mut l.radius, 1..=60, LIMIT_RADIUS);
                int_row(ui, "Dicke", &mut l.thickness, 1..=8, LIMIT_THICKNESS);
                outline_rows(ui, &mut l.outline);
            }
            Layer::Chevron(l) => {
                color_row(ui, "Farbe", &mut l.color);
                int_row(ui, "Größe", &mut l.size, 2..=40, LIMIT_CHEVRON);
                int_row(ui, "Dicke", &mut l.thickness, 1..=8, (1, 16));
                int_row(ui, "Winkel", &mut l.angle, 20..=160, LIMIT_ANGLE);
                signed_row(ui, "Abstand", &mut l.gap, -20..=30, LIMIT_GAP);
                outline_rows(ui, &mut l.outline);
            }
            Layer::Image(l) => {
                ui.label("Datei");
                ui.horizontal(|ui| {
                    let name = if l.file.is_empty() { "keine".to_string() } else { truncate(&short_name(&l.file), 18) };
                    ui.label(RichText::new(name).color(p.muted));
                    if ui.small_button(theme::I_FOLDER).on_hover_text("Bild wählen").clicked() {
                        pick_image = true;
                    }
                });
                ui.end_row();
                if !l.file.is_empty() && !render::image_ok(&l.file) {
                    ui.label("");
                    ui.label(RichText::new("Bild nicht lesbar (PNG/SVG)").size(11.0).color(p.danger));
                    ui.end_row();
                }
                ui.label("Größe");
                ui.add(
                    egui::Slider::new(&mut l.scale, LIMIT_SCALE.0..=4.0)
                        .logarithmic(true)
                        .clamping(egui::SliderClamping::Edits)
                        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                        .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
                );
                l.scale = l.scale.clamp(LIMIT_SCALE.0, LIMIT_SCALE.1);
                ui.end_row();
                ui.label("Deckkraft");
                ui.add(
                    egui::Slider::new(&mut l.opacity, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                        .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
                );
                ui.end_row();
            }
        });
        if pick_image {
            self.pick_image();
        }
    }

    fn pick_image(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Bild für das Crosshair wählen")
            .add_filter("Bilder (PNG, SVG)", &["png", "svg"])
            .pick_file()
        else {
            return;
        };
        let selected = self.selected;
        match import_image(&path) {
            Ok(name) => {
                if let Some(Layer::Image(l)) = self.crosshair_mut().layers.get_mut(selected) {
                    l.file = name;
                }
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn presets_tab(&mut self, ui: &mut Ui) {
        let p = self.palette();
        ui.label(
            RichText::new(format!("Klick übernimmt ins Profil „{}“. Rechtsklick löscht eigene.", self.profile_name()))
                .size(11.0)
                .color(p.muted),
        );
        let mut apply: Option<Crosshair> = None;
        let mut delete: Option<usize> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            let builtin = self.builtin.len();
            for (i, preset) in self.builtin.iter().chain(self.presets.iter()).enumerate() {
                let Some(tex) = self.thumbs.get(i) else { continue };
                let own = i >= builtin;
                let response = preset_tile(ui, &p, tex, &preset.name, own);
                if response.clicked() {
                    apply = Some(preset.crosshair.clone());
                }
                if own {
                    response.context_menu(|ui| {
                        if ui.button(format!("{}  Löschen", theme::I_TRASH)).clicked() {
                            delete = Some(i - builtin);
                            ui.close();
                        }
                    });
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.preset_name).hint_text("Name für aktuelles Crosshair").desired_width(190.0),
            );
            let name = self.preset_name.trim().to_string();
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ui.add_enabled(!name.is_empty(), egui::Button::new(format!("{} Speichern", theme::I_SAVE))).clicked();
            if (clicked || enter) && !name.is_empty() {
                let preset = Preset { name: name.clone(), crosshair: self.crosshair().clone() };
                match self.presets.iter_mut().find(|p| p.name == name) {
                    Some(existing) => *existing = preset,
                    None => self.presets.push(preset),
                }
                self.persist_presets();
                self.preset_name.clear();
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("Teilen").font(FontId::new(12.5, heading())));
        ui.horizontal(|ui| {
            if ui.button(format!("{} Code kopieren", theme::I_COPY)).on_hover_text("Code für Discord & Co.").clicked() {
                let (code, dropped) = share::encode(self.crosshair());
                ui.ctx().copy_text(code);
                let note = if dropped { " Bild-Ebenen bleiben auf deinem PC." } else { "" };
                self.message = Some((format!("Code kopiert.{note}"), false));
            }
            if ui
                .button(format!("{} Code einfügen", theme::I_PASTE))
                .on_hover_text("Code aus der Zwischenablage übernehmen")
                .clicked()
            {
                match clipboard::read_text().filter(|t| !t.trim().is_empty()) {
                    Some(text) => {
                        self.share_text = text.trim().to_string();
                        self.import_code();
                    }
                    None => self.message = Some(("In der Zwischenablage ist kein Text.".into(), true)),
                }
            }
            if ui.button(format!("{} PNG", theme::I_IMAGE)).on_hover_text("Crosshair als Bild speichern").clicked() {
                self.export_png();
            }
        });
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.share_text)
                    .hint_text("oder hier eintippen / Strg+V")
                    .desired_width(210.0),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ui
                .add_enabled(!self.share_text.trim().is_empty(), egui::Button::new("Übernehmen"))
                .clicked();
            if (clicked || enter) && !self.share_text.trim().is_empty() {
                self.import_code();
            }
        });
        ui.label(
            RichText::new("Spiel-Codes werden auf deine Bildschirmhöhe umgerechnet.").size(10.5).color(p.muted),
        );

        if let Some(crosshair) = apply {
            *self.crosshair_mut() = crosshair;
            self.selected = self.crosshair().layers.len().saturating_sub(1);
        }
        if let Some(i) = delete {
            self.presets.remove(i);
            self.persist_presets();
        }
    }

    /// Height of the monitor the overlay uses; game codes are scaled to it.
    fn screen_height(&self) -> u32 {
        overlay::pick_monitor(self.config.monitor).map_or(1080, |m| (m.rect.bottom - m.rect.top).max(240) as u32)
    }

    fn import_code(&mut self) {
        let height = self.screen_height();
        match share::decode(&self.share_text, height) {
            Ok((crosshair, source)) => {
                *self.crosshair_mut() = crosshair;
                self.selected = self.crosshair().layers.len().saturating_sub(1);
                self.share_text.clear();
                let scaled = if source == share::Source::Reticle { String::new() } else { format!(", für {height} px Bildhöhe") };
                self.message = Some((format!("{}-Crosshair übernommen{scaled}.", source.label()), false));
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn export_png(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Crosshair als PNG speichern")
            .set_file_name(format!("{}.png", file_stem(&self.profile_name())))
            .add_filter("PNG", &["png"])
            .save_file()
        else {
            return;
        };
        let result = render::render(self.crosshair()).save_png(&path);
        self.message = Some(match result {
            Ok(()) => (format!("Gespeichert: {}", short_name(&path.to_string_lossy())), false),
            Err(e) => (format!("PNG konnte nicht gespeichert werden: {e}"), true),
        });
    }

    fn persist_presets(&mut self) {
        if let Err(e) = config::save_presets(&self.presets) {
            self.message = Some((e, true));
        }
        self.thumbs.clear();
    }

    fn profile_tab(&mut self, ui: &mut Ui) {
        let p = self.palette();
        // New profile from a template.
        ui.horizontal(|ui| {
            ui.menu_button(format!("{} Neues Profil", theme::I_PLUS), |ui| {
                for (name, programs) in GAME_TEMPLATES {
                    if ui.button(*name).clicked() {
                        self.add_profile(name, programs);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Eigenes Spiel").clicked() {
                    self.add_profile("Neues Profil", &[]);
                    ui.close();
                }
            });
            if let Some(i) = self.profile
                && ui.button(format!("{} Löschen", theme::I_TRASH)).on_hover_text("Profil löschen").clicked()
            {
                self.config.profiles.remove(i);
                self.select_profile(None);
            }
        });
        ui.add_space(4.0);

        let Some(i) = self.profile else {
            ui.label(RichText::new("Standard gilt überall, wo kein Spielprofil passt.").size(11.0).color(p.muted));
            ui.add_space(4.0);
            self.look_rows(ui);
            return;
        };
        let mut remove_program = None;
        egui::Grid::new("profile").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
            let prof = &mut self.config.profiles[i];
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut prof.name).desired_width(180.0));
            ui.end_row();
            ui.label("Programme");
            ui.vertical(|ui| {
                if prof.programs.is_empty() {
                    ui.label(RichText::new("noch keins – Spiel starten und unten wählen").size(11.0).color(p.danger));
                }
                for (j, program) in prof.programs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(program).size(11.5));
                        if icon_button(ui, theme::I_XMARK, p.muted).on_hover_text("Entfernen").clicked() {
                            remove_program = Some(j);
                        }
                    });
                }
            });
            ui.end_row();
        });
        if let Some(j) = remove_program {
            self.config.profiles[i].programs.remove(j);
        }

        // Add a program by name or from the running ones.
        ui.horizontal(|ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut self.program_text).hint_text("z. B. game.exe").desired_width(120.0));
            let enter = field.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Enter));
            if (ui.small_button(theme::I_PLUS).on_hover_text("Programm hinzufügen").clicked() || enter)
                && !self.program_text.trim().is_empty()
            {
                let name = self.program_text.trim().to_string();
                self.add_program(i, name);
                self.program_text.clear();
            }
        });
        ui.horizontal(|ui| {
            let mut picked = None;
            egui::ComboBox::from_id_salt("running")
                .selected_text("Laufende Programme")
                .width(180.0)
                .show_ui(ui, |ui| {
                    if self.running.is_empty() {
                        ui.label("keine gefunden");
                    }
                    for exe in &self.running {
                        if ui.selectable_label(false, exe).clicked() {
                            picked = Some(exe.clone());
                        }
                    }
                });
            if let Some(exe) = picked {
                self.add_program(i, exe);
            }
            if icon_button(ui, theme::I_REFRESH, p.muted).on_hover_text("Liste aktualisieren").clicked() {
                self.running = process::running_programs();
            }
        });
        ui.label(RichText::new("* steht für beliebige Zeichen, z. B. FiveM_*GTAProcess.exe").size(10.5).color(p.muted));
        ui.add_space(4.0);
        ui.separator();
        self.look_rows(ui);
    }

    fn add_program(&mut self, profile: usize, exe: String) {
        let programs = &mut self.config.profiles[profile].programs;
        if !programs.iter().any(|p| p.eq_ignore_ascii_case(&exe)) {
            programs.push(exe);
        }
    }

    fn add_profile(&mut self, name: &str, programs: &[&str]) {
        self.config.profiles.push(Profile {
            name: name.to_string(),
            programs: programs.iter().map(|s| s.to_string()).collect(),
            crosshair: self.config.crosshair.clone(),
            ..Profile::default()
        });
        self.select_profile(Some(self.config.profiles.len() - 1));
        self.running = process::running_programs();
    }

    /// Aim behaviour and offset of the profile being edited.
    fn look_rows(&mut self, ui: &mut Ui) {
        let p = self.palette();
        let (ox, oy, aim) = self.look_mut();
        egui::Grid::new("look").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
            ui.label("Beim Zielen");
            egui::ComboBox::from_id_salt("aim").selected_text(aim.label()).width(170.0).show_ui(ui, |ui| {
                for mode in AimMode::ALL {
                    ui.selectable_value(aim, mode, mode.label());
                }
            });
            ui.end_row();
            signed_row(ui, "Versatz X", ox, -50..=50, LIMIT_OFFSET);
            signed_row(ui, "Versatz Y", oy, -50..=50, LIMIT_OFFSET);
            ui.label("");
            if ui.add_enabled(*ox != 0 || *oy != 0, egui::Button::new(format!("{} Zur Mitte", theme::I_RESET))).clicked() {
                *ox = 0;
                *oy = 0;
            }
            ui.end_row();
        });
        ui.label(RichText::new("Zielen = rechte Maustaste.").size(10.5).color(p.muted));
    }

    fn options_tab(&mut self, ui: &mut Ui) {
        let p = self.palette();
        egui::Grid::new("options").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Hotkey");
            ui.vertical(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.hotkey_text).hint_text("z. B. F8").desired_width(140.0),
                );
                let valid = hotkey::parse(&self.hotkey_text);
                // Only commit on Enter or leaving the field, so half-typed keys are never registered.
                if response.lost_focus() && valid.is_ok() {
                    self.config.toggle_hotkey = self.hotkey_text.trim().to_string();
                }
                match valid {
                    Err(e) => ui.label(RichText::new(e).size(10.5).color(p.danger)),
                    Ok(_) if response.has_focus() => ui.label(RichText::new("Enter übernimmt").size(10.5).color(p.muted)),
                    Ok(_) => ui.label(RichText::new("Crosshair ein/aus").size(10.5).color(p.muted)),
                };
            });
            ui.end_row();

            ui.label("Monitor");
            ui.horizontal(|ui| {
                let label = match self.config.monitor.and_then(|i| self.monitors.get(i).map(|m| (i, m))) {
                    Some((i, m)) => monitor_label(i, m),
                    None => "Automatisch".to_string(),
                };
                egui::ComboBox::from_id_salt("monitor").selected_text(label).width(150.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.config.monitor, None, "Automatisch")
                        .on_hover_text("Monitor des Spiels: Programm mit Profil oder Fenster, das den ganzen Bildschirm füllt. Sonst Hauptmonitor.");
                    for (i, mon) in self.monitors.iter().enumerate() {
                        ui.selectable_value(&mut self.config.monitor, Some(i), monitor_label(i, mon));
                    }
                });
                if icon_button(ui, theme::I_REFRESH, p.muted).on_hover_text("Monitore neu einlesen").clicked() {
                    self.monitors = overlay::monitors();
                }
            });
            ui.end_row();

            ui.label("Anzeigen");
            ui.checkbox(&mut self.config.only_in_games, "nur in Spielen mit Profil");
            ui.end_row();
            ui.label("Stream");
            ui.checkbox(&mut self.config.hide_from_capture, "in Aufnahmen verstecken");
            ui.end_row();
            ui.label("Autostart");
            let mut on = self.autostart;
            if ui.checkbox(&mut on, "mit Windows starten").changed() {
                match autostart::set(on) {
                    Ok(()) => self.autostart = autostart::is_enabled(),
                    Err(e) => self.message = Some((e, true)),
                }
            }
            ui.end_row();
            ui.label("Design");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.config.editor_theme, Theme::Dark, format!("{} Dunkel", theme::I_MOON));
                ui.selectable_value(&mut self.config.editor_theme, Theme::Light, format!("{} Hell", theme::I_SUN));
            });
            ui.end_row();
        });
    }

    fn preview_area(&mut self, ui: &mut Ui) {
        let p = self.palette();
        let size = Vec2::new(ui.available_width(), PREVIEW_HEIGHT);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect);
        let ppp = ui.ctx().pixels_per_point();
        draw_backdrop(&painter, rect, self.backdrop, 10.0);
        painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, p.line), egui::StrokeKind::Inside);

        // Snap the center to a physical pixel so one canvas pixel maps to exactly `zoom` screen pixels.
        let center = Pos2::new((rect.center().x * ppp).round() / ppp, (rect.center().y * ppp).round() / ppp);
        if let Some(tex) = &self.preview {
            let side = self.preview_size as f32 * self.zoom as f32 / ppp;
            painter.image(
                tex.id(),
                Rect::from_center_size(center, Vec2::splat(side)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.guides {
            let guide = Stroke::new(1.0 / ppp, Color32::from_rgb(0xFF, 0x2D, 0xAA));
            painter.line_segment([Pos2::new(center.x, rect.top()), Pos2::new(center.x, rect.bottom())], guide);
            painter.line_segment([Pos2::new(rect.left(), center.y), Pos2::new(rect.right(), center.y)], guide);
        }
        painter.text(
            rect.left_top() + Vec2::new(7.0, 5.0),
            egui::Align2::LEFT_TOP,
            self.profile_name(),
            FontId::proportional(10.5),
            match self.backdrop {
                Backdrop::Light | Backdrop::Sky => Color32::from_rgb(0x30, 0x33, 0x38),
                _ => Color32::from_rgb(0xB8, 0xBC, 0xC4),
            },
        );

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 70.0;
            ui.add(egui::Slider::new(&mut self.zoom, 1..=16).suffix("×")).on_hover_text("Zoom (1× = Originalgröße)");
            for b in Backdrop::ALL {
                let (r, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                draw_backdrop(ui.painter(), r.shrink(2.0), b, 4.0);
                let stroke = if self.backdrop == b { Stroke::new(2.0, p.accent) } else { Stroke::new(1.0, p.line) };
                ui.painter().rect_stroke(r.shrink(1.0), CornerRadius::same(3), stroke, egui::StrokeKind::Inside);
                if response.on_hover_text(b.label()).clicked() {
                    self.backdrop = b;
                }
            }
            ui.toggle_value(&mut self.guides, "+").on_hover_text("Mitte markieren");
        });
    }
}

#[derive(Clone, Copy)]
enum LayerAction {
    Select,
    ToggleVisible,
    Up,
    Down,
    Delete,
}

/// Constructor for a fresh layer of one kind.
type MakeLayer = fn() -> Layer;

// -------------------------------------------------------------------------------------------------
// Widgets

fn icon_button(ui: &mut Ui, icon: &str, color: Color32) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).size(10.5).color(color)).frame(false).min_size(Vec2::new(16.0, 16.0)))
}

fn preset_tile(ui: &mut Ui, p: &Palette, tex: &TextureHandle, name: &str, own: bool) -> egui::Response {
    let ppp = ui.ctx().pixels_per_point();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(THUMB + 10.0, THUMB + 16.0), Sense::click());
    let tile = Rect::from_min_size(rect.min + Vec2::new(5.0, 0.0), Vec2::splat(THUMB));
    let painter = ui.painter();
    let hovered = response.hovered();
    painter.rect_filled(tile, CornerRadius::same(6), Backdrop::Dark.color());
    let stroke = if hovered { Stroke::new(1.5, p.accent) } else { Stroke::new(1.0, p.line) };
    painter.rect_stroke(tile, CornerRadius::same(6), stroke, egui::StrokeKind::Inside);
    // Integer zoom when it fits, otherwise scaled down.
    let px = tex.size()[0] as f32;
    let room = (THUMB - 6.0) * ppp;
    let scale = if px * 2.0 <= room { (room / px).floor().min(3.0) } else { (room / px).min(1.0) };
    let side = px * scale / ppp;
    let center = Pos2::new((tile.center().x * ppp).round() / ppp, (tile.center().y * ppp).round() / ppp);
    painter.image(
        tex.id(),
        Rect::from_center_size(center, Vec2::splat(side)),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    if own {
        // Marks presets the user saved.
        painter.circle_filled(tile.right_top() + Vec2::new(-5.0, 5.0), 2.5, p.accent);
    }
    painter.text(
        Pos2::new(tile.center().x, tile.bottom() + 2.0),
        egui::Align2::CENTER_TOP,
        truncate(name, 9),
        FontId::proportional(10.0),
        if hovered { p.text } else { p.muted },
    );
    response.on_hover_text(name)
}

fn draw_backdrop(painter: &egui::Painter, rect: Rect, b: Backdrop, tile: f32) {
    let radius = CornerRadius::same(if tile > 6.0 { 6 } else { 2 });
    if b != Backdrop::Checker {
        painter.rect_filled(rect, radius, b.color());
        return;
    }
    painter.rect_filled(rect, radius, Color32::from_rgb(0x9A, 0x9A, 0x9A));
    let dark = Color32::from_rgb(0x6E, 0x6E, 0x6E);
    let painter = painter.with_clip_rect(rect.shrink(1.0));
    let (cols, rows) = ((rect.width() / tile).ceil() as i32, (rect.height() / tile).ceil() as i32);
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                let min = rect.min + Vec2::new(x as f32 * tile, y as f32 * tile);
                painter.rect_filled(Rect::from_min_size(min, Vec2::splat(tile)), 0.0, dark);
            }
        }
    }
}

/// Color picker plus hex field. Keeps the string untouched while the hex is being typed.
fn color_row(ui: &mut Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.horizontal(|ui| {
        let parsed = Rgba::parse(value).unwrap_or(Rgba { r: 0, g: 0, b: 0, a: 255 });
        let mut rgba = [parsed.r, parsed.g, parsed.b, parsed.a];
        if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
            *value = Rgba { r: rgba[0], g: rgba[1], b: rgba[2], a: rgba[3] }.to_hex();
        }
        let mut text = value.clone();
        if ui.add(egui::TextEdit::singleline(&mut text).desired_width(76.0)).changed() {
            let t = text.trim();
            let normalized = if t.starts_with('#') { t.to_string() } else { format!("#{t}") };
            if Rgba::parse(&normalized).is_some() {
                *value = normalized.to_uppercase();
            }
        }
    });
    ui.end_row();
}

/// Slider with a practical range; typing a value allows going up to the hard limit.
fn int_row(ui: &mut Ui, label: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>, limit: (u32, u32)) {
    ui.label(label);
    ui.add(egui::Slider::new(value, range).clamping(egui::SliderClamping::Edits));
    *value = (*value).clamp(limit.0, limit.1);
    ui.end_row();
}

fn signed_row(ui: &mut Ui, label: &str, value: &mut i32, range: std::ops::RangeInclusive<i32>, limit: (i32, i32)) {
    ui.label(label);
    ui.add(egui::Slider::new(value, range).clamping(egui::SliderClamping::Edits));
    *value = (*value).clamp(limit.0, limit.1);
    ui.end_row();
}

fn outline_rows(ui: &mut Ui, outline: &mut Outline) {
    ui.label("Kontur");
    ui.horizontal(|ui| {
        ui.checkbox(&mut outline.enabled, "");
        if outline.enabled {
            ui.add(egui::Slider::new(&mut outline.thickness, 1..=4).clamping(egui::SliderClamping::Edits))
                .on_hover_text("Konturbreite");
            outline.thickness = outline.thickness.clamp(LIMIT_OUTLINE.0, LIMIT_OUTLINE.1);
        }
    });
    ui.end_row();
    if outline.enabled {
        color_row(ui, "Konturfarbe", &mut outline.color);
    }
}

fn kind_icon(layer: &Layer) -> &'static str {
    match layer {
        Layer::Cross(_) => theme::I_PLUS,
        Layer::Dot(_) => theme::I_DOT,
        Layer::Circle(_) => theme::I_RING,
        Layer::Chevron(_) => theme::I_CHEVRON,
        Layer::Image(_) => theme::I_IMAGE,
    }
}

fn layer_label(layer: &Layer) -> String {
    match layer {
        Layer::Image(l) if !l.file.is_empty() => format!("Bild · {}", truncate(&short_name(&l.file), 16)),
        other => other.kind_name().to_string(),
    }
}

fn layer_color_mut(layer: &mut Layer) -> Option<&mut String> {
    match layer {
        Layer::Cross(l) => Some(&mut l.color),
        Layer::Dot(l) => Some(&mut l.color),
        Layer::Circle(l) => Some(&mut l.color),
        Layer::Chevron(l) => Some(&mut l.color),
        Layer::Image(_) => None,
    }
}

fn monitor_label(i: usize, m: &Monitor) -> String {
    let (w, h) = (m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
    let main = if m.primary { " · Haupt" } else { "" };
    format!("{} · {w}×{h}{main}", i + 1)
}

fn short_name(file: &str) -> String {
    Path::new(file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
}

// -------------------------------------------------------------------------------------------------
// Files and processes

fn config_stamp() -> Option<SystemTime> {
    std::fs::metadata(config::path()).and_then(|m| m.modified()).ok()
}

/// Copies an image into `%APPDATA%\reticle\images\` and returns the stored file name.
fn import_image(source: &Path) -> Result<String, String> {
    let dir = config::images_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Bildordner konnte nicht angelegt werden: {e}"))?;
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let stem: String = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bild".into())
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let mut name = format!("{stem}.{ext}");
    let mut n = 2;
    while dir.join(&name).exists() {
        if same_file(&dir.join(&name), source) {
            return Ok(name);
        }
        name = format!("{stem}-{n}.{ext}");
        n += 1;
    }
    std::fs::copy(source, dir.join(&name)).map_err(|e| format!("Bild konnte nicht kopiert werden: {e}"))?;
    if !render::image_ok(&name) {
        let _ = std::fs::remove_file(dir.join(&name));
        return Err("Das Bild konnte nicht gelesen werden. Erlaubt sind PNG und SVG.".into());
    }
    Ok(name)
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

fn control_window() -> Option<windows::Win32::Foundation::HWND> {
    unsafe { FindWindowW(CONTROL_CLASS, PCWSTR::null()).ok() }
}

fn overlay_running() -> bool {
    control_window().is_some()
}

fn notify_overlay() {
    if let Some(hwnd) = control_window() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_RELOAD, WPARAM(0), LPARAM(0));
        }
    }
}

/// File name from a profile name: letters, digits, dashes.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if stem.is_empty() { "crosshair".into() } else { format!("crosshair-{stem}") }
}
