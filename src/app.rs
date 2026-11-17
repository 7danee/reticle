//! Background mode: hidden control window that owns tray, hotkey, profile switching and the
//! overlay.
//!
//! Fully event driven. While idle the thread sleeps in `MsgWaitForMultipleObjects` and only
//! wakes for: window messages (tray, hotkey, editor), a foreground window change, or a change
//! in the config folder. The only timer is the right-button check, and it runs only while the
//! active profile hides the crosshair when aiming.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::SystemTime;

use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindCloseChangeNotification,
    FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, INFINITE, SetProcessWorkingSetSize};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_RBUTTON,
};
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    EVENT_SYSTEM_FOREGROUND, GetCursorPos, GetForegroundWindow, KillTimer, MF_CHECKED, MF_DEFAULT, MF_SEPARATOR,
    MF_STRING, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, QS_ALLINPUT,
    RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL, SetForegroundWindow, SetTimer, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_HOTKEY, WM_LBUTTONUP, WM_NULL, WM_QUIT, WM_RBUTTONUP,
    WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::config::{self, AimMode, Config, Look};
use crate::hotkey;
use crate::overlay::{self, Overlay};
use crate::process;
use crate::render;
use crate::tray::Tray;

/// Window class of the control window. The editor finds it to request a reload.
pub const CONTROL_CLASS: PCWSTR = w!("reticle_control");
/// Posted by the editor after it saved the config.
pub const WM_RELOAD: u32 = WM_APP + 2;

const WM_TRAY: u32 = WM_APP + 1;
const HOTKEY_TOGGLE: i32 = 1;
/// Right-button check while aim hiding is on. 15 ms is one scheduler tick; fast enough that the
/// crosshair is gone before any aim-down-sights animation finishes.
const TIMER_AIM: usize = 2;
const AIM_MS: u32 = 15;
/// One-shot checks for exclusive fullscreen after a program comes to the front.
const TIMER_FULLSCREEN: usize = 3;
const FULLSCREEN_CHECKS_MS: [u32; 2] = [3_000, 12_000];

const MENU_EDITOR: usize = 1;
const MENU_TOGGLE: usize = 2;
const MENU_CAPTURE: usize = 3;
const MENU_EDIT: usize = 4;
const MENU_FOLDER: usize = 5;
const MENU_QUIT: usize = 6;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f` on the app state. Returns `None` while the state is borrowed (re-entrant message).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

struct App {
    control: HWND,
    overlay: Overlay,
    tray: Tray,
    config: Config,
    config_stamp: Option<SystemTime>,
    hotkey_active: bool,
    /// Toggled with the hotkey or the tray menu.
    user_shown: bool,
    /// Hidden because the right mouse button aims.
    aim_hidden: bool,
    /// Right button state at the last aim check (for toggle mode).
    aim_button: bool,
    aim_timer: bool,
    /// Active game profile (`None` = Standard).
    profile: Option<usize>,
    /// Last foreground program other than reticle itself.
    foreground: Option<String>,
    /// reticle's own window (editor, tray menu) is in front: keep the last profile.
    own_in_front: bool,
    own_exe: String,
    tray_state: Option<(bool, String)>,
    /// Monitor of the last game, used when the monitor is set to automatic.
    game_monitor: Option<overlay::Monitor>,
    /// Pending exclusive-fullscreen checks for the current foreground program.
    fullscreen_checks_left: usize,
    /// Programs already warned about in this session (lowercase file names).
    fullscreen_warned: HashSet<String>,
}

/// Starts the settings window as its own process.
pub fn spawn_editor() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--editor").spawn();
    }
}

pub fn run() -> Result<()> {
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    let control = unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(control_proc),
            hInstance: instance,
            lpszClassName: CONTROL_CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CONTROL_CLASS,
            w!("reticle"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?
    };

    let first_start = !config::path().exists();
    let (config, load_error) = match config::load() {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e)),
    };
    let mut app = App {
        control,
        overlay: Overlay::create(instance)?,
        tray: Tray::new(control, WM_TRAY),
        config,
        config_stamp: config_stamp(),
        hotkey_active: false,
        user_shown: true,
        aim_hidden: false,
        aim_button: false,
        aim_timer: false,
        profile: None,
        foreground: None,
        own_in_front: false,
        own_exe: process::own_exe().unwrap_or_default().to_lowercase(),
        tray_state: None,
        game_monitor: None,
        fullscreen_checks_left: 0,
        fullscreen_warned: HashSet::new(),
    };
    app.tray.add();
    app.apply(None);
    app.on_foreground(unsafe { GetForegroundWindow() });
    app.sync_visibility();
    app.update_tray();
    if let Some(e) = load_error {
        app.tray.notify("Config fehlerhaft", &format!("{e}\nStandard-Crosshair ist aktiv."), true);
    } else if first_start {
        let key = match app.config.toggle_hotkey.trim() {
            "" => String::new(),
            k => format!(" {k} blendet es ein und aus."),
        };
        app.tray.notify(
            "reticle läuft",
            &format!("Das Crosshair sitzt in der Bildschirmmitte.{key} Klick aufs Symbol öffnet die Einstellungen."),
            false,
        );
    }
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    // Watches the config folder, so edits in a text editor apply without any polling.
    let watch = unsafe {
        FindFirstChangeNotificationW(
            &HSTRING::from(config::dir().as_os_str()),
            false,
            FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
        )
        .ok()
    };
    trim_memory();

    message_loop(watch);

    unsafe {
        if let Some(handle) = watch {
            let _ = FindCloseChangeNotification(handle);
        }
        if !hook.is_invalid() {
            let _ = UnhookWinEvent(hook);
        }
    }
    APP.with(|cell| cell.borrow_mut().take());
    Ok(())
}

/// Sleeps until a message arrives or the config folder changes.
fn message_loop(watch: Option<HANDLE>) {
    let handles: Vec<HANDLE> = watch.into_iter().collect();
    let mut msg = MSG::default();
    loop {
        let woke = unsafe {
            MsgWaitForMultipleObjects((!handles.is_empty()).then_some(handles.as_slice()), false, INFINITE, QS_ALLINPUT)
        };
        if !handles.is_empty() && woke == WAIT_OBJECT_0 {
            unsafe {
                let _ = FindNextChangeNotification(handles[0]);
            }
            with_app(App::check_config_file);
            continue;
        }
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// Hands start-up pages (font parsing, first render) back to Windows. They are not needed again.
fn trim_memory() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

fn config_stamp() -> Option<SystemTime> {
    std::fs::metadata(config::path()).and_then(|m| m.modified()).ok()
}

impl App {
    fn look(&self) -> Look<'_> {
        self.config.look(self.profile)
    }

    fn profile_name(&self) -> String {
        self.profile
            .and_then(|i| self.config.profiles.get(i))
            .map_or_else(|| "Standard".to_string(), |p| p.name.clone())
    }

    /// Renders the active look and moves the overlay into place.
    fn render_overlay(&mut self) {
        let look = self.look();
        let pixmap = render::render(look.crosshair);
        let (ox, oy) = look.offset;
        let monitor = match self.config.monitor {
            Some(index) => overlay::pick_monitor(Some(index)),
            // Automatic: where the last game was, otherwise the primary monitor.
            None => self.game_monitor.or_else(|| overlay::pick_monitor(None)),
        };
        if let Some(monitor) = monitor {
            let (cx, cy) = monitor.center();
            if let Err(e) = self.overlay.update(&pixmap, cx + ox, cy + oy) {
                self.tray.notify("Overlay-Fehler", &e.message(), true);
            }
        }
    }

    /// Shown unless hidden by hotkey, by "only in games", or while aiming.
    fn wanted(&self) -> bool {
        self.user_shown && (!self.config.only_in_games || self.profile.is_some())
    }

    fn sync_visibility(&mut self) {
        let visible = self.wanted() && !self.aim_hidden;
        if self.overlay.is_shown() != visible {
            self.overlay.set_shown(visible);
        }
    }

    /// Applies the config. Hotkey and capture flag are only touched when they changed
    /// compared to `previous`, so live edits from the editor stay cheap.
    fn apply(&mut self, previous: Option<&Config>) {
        let profile = self.foreground.as_deref().and_then(|exe| self.config.match_profile(exe));
        if profile != self.profile {
            self.profile = profile;
            self.aim_hidden = false;
        }
        self.render_overlay();
        if previous.is_none_or(|p| p.hide_from_capture != self.config.hide_from_capture)
            && let Err(e) = self.overlay.set_capture_excluded(self.config.hide_from_capture)
            && self.config.hide_from_capture
        {
            self.tray.notify("Stream-Schutz nicht möglich", &e.message(), true);
        }
        if previous.is_none_or(|p| p.toggle_hotkey != self.config.toggle_hotkey) {
            self.register_hotkey();
        }
        self.update_aim_timer();
        self.sync_visibility();
        self.update_tray();
    }

    fn on_foreground(&mut self, hwnd: HWND) {
        let exe = process::exe_of_window(hwnd);
        self.own_in_front = exe.as_ref().is_some_and(|e| e.to_lowercase() == self.own_exe);
        if !self.own_in_front {
            let profile = exe.as_deref().and_then(|e| self.config.match_profile(e));
            let moved = self.follow_game_monitor(hwnd, profile.is_some(), exe.as_deref());
            self.foreground = exe;
            let switched = profile != self.profile;
            if switched {
                self.profile = profile;
                self.aim_hidden = false;
                self.update_tray();
            }
            if switched || moved {
                self.render_overlay();
            }
            self.schedule_fullscreen_check();
        }
        self.update_aim_timer();
        self.sync_visibility();
        // A newly activated topmost window would cover us; this is the only place that can happen.
        self.overlay.raise();
    }

    /// With automatic monitor choice the crosshair moves to the monitor of a game: a program
    /// with a profile, or any window that fills its whole monitor (borderless or fullscreen).
    /// Other windows (browser on the second screen, desktop) leave it where it is.
    /// Returns true if the monitor changed.
    fn follow_game_monitor(&mut self, hwnd: HWND, has_profile: bool, exe: Option<&str>) -> bool {
        if self.config.monitor.is_some() {
            return false;
        }
        let Some(monitor) = overlay::monitor_of_window(hwnd) else { return false };
        let shell = exe.is_some_and(|e| e.eq_ignore_ascii_case("explorer.exe"));
        let game = has_profile || (!shell && overlay::fills_monitor(hwnd, &monitor));
        if !game || self.game_monitor == Some(monitor) {
            return false;
        }
        self.game_monitor = Some(monitor);
        true
    }

    /// A game in exclusive fullscreen hides every overlay. Some games switch a few seconds
    /// after getting focus, so the state is checked twice (after 3 s and 15 s), once per
    /// program and session. Nothing runs while no new program comes to the front.
    fn schedule_fullscreen_check(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.control), TIMER_FULLSCREEN);
        }
        self.fullscreen_checks_left = 0;
        let Some(exe) = self.foreground.as_deref() else { return };
        let exe = exe.to_lowercase();
        if !self.config.warn_fullscreen || exe == "explorer.exe" || self.fullscreen_warned.contains(&exe) {
            return;
        }
        self.fullscreen_checks_left = FULLSCREEN_CHECKS_MS.len();
        unsafe {
            SetTimer(Some(self.control), TIMER_FULLSCREEN, FULLSCREEN_CHECKS_MS[0], None);
        }
    }

    fn check_fullscreen(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.control), TIMER_FULLSCREEN);
        }
        let Some(exe) = self.foreground.clone() else { return };
        if self.own_in_front || self.fullscreen_checks_left == 0 || !self.config.warn_fullscreen {
            return;
        }
        let exclusive = unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN);
        if exclusive {
            self.fullscreen_checks_left = 0;
            self.fullscreen_warned.insert(exe.to_lowercase());
            self.tray.notify(
                "Exklusives Vollbild erkannt",
                &format!(
                    "{exe} läuft im exklusiven Vollbild und kann das Crosshair verdecken. \
                     Stell im Spiel „Randloses Fenster“ ein."
                ),
                true,
            );
            return;
        }
        self.fullscreen_checks_left -= 1;
        let next = FULLSCREEN_CHECKS_MS.len() - self.fullscreen_checks_left;
        if let Some(&delay) = FULLSCREEN_CHECKS_MS.get(next) {
            unsafe {
                SetTimer(Some(self.control), TIMER_FULLSCREEN, delay, None);
            }
        }
    }

    /// The right-button check only runs while it can matter.
    fn update_aim_timer(&mut self) {
        let wanted = self.look().aim != AimMode::Off && self.wanted() && !self.own_in_front;
        if wanted == self.aim_timer {
            return;
        }
        unsafe {
            if wanted {
                SetTimer(Some(self.control), TIMER_AIM, AIM_MS, None);
            } else {
                let _ = KillTimer(Some(self.control), TIMER_AIM);
            }
        }
        self.aim_timer = wanted;
        self.aim_button = false;
        self.aim_hidden = false;
    }

    fn check_aim(&mut self) {
        let down = unsafe { GetAsyncKeyState(VK_RBUTTON.0 as i32) } < 0;
        let pressed = down && !self.aim_button;
        self.aim_button = down;
        let hidden = match self.look().aim {
            AimMode::Off => false,
            AimMode::Hold => down,
            AimMode::Toggle if pressed => !self.aim_hidden,
            AimMode::Toggle => self.aim_hidden,
        };
        if hidden != self.aim_hidden {
            self.aim_hidden = hidden;
            self.sync_visibility();
        }
    }

    fn register_hotkey(&mut self) {
        unsafe {
            if self.hotkey_active {
                let _ = UnregisterHotKey(Some(self.control), HOTKEY_TOGGLE);
                self.hotkey_active = false;
            }
            let Ok(Some(key)) = hotkey::parse(&self.config.toggle_hotkey) else {
                return;
            };
            let modifiers = HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT;
            match RegisterHotKey(Some(self.control), HOTKEY_TOGGLE, modifiers, key.vk) {
                Ok(()) => self.hotkey_active = true,
                Err(_) => self.tray.notify(
                    "Hotkey belegt",
                    &format!("{} wird schon von einem anderen Programm benutzt.", self.config.toggle_hotkey),
                    true,
                ),
            }
        }
    }

    fn update_tray(&mut self) {
        let state = if self.user_shown { "sichtbar" } else { "ausgeblendet" };
        let key = self.config.toggle_hotkey.trim();
        let key = if key.is_empty() { String::new() } else { format!(" ({key})") };
        let tip = format!("reticle – {state}{key}\nProfil: {}", self.profile_name());
        let state = (self.user_shown, tip);
        if self.tray_state.as_ref() != Some(&state) {
            self.tray.set_state(state.0, &state.1);
            self.tray_state = Some(state);
        }
    }

    fn toggle(&mut self) {
        self.user_shown = !self.user_shown;
        self.update_aim_timer();
        self.sync_visibility();
        self.update_tray();
    }

    /// Reloads the config if the file really changed (the folder watch also fires for presets,
    /// images and the temp file of an atomic save).
    fn check_config_file(&mut self) {
        let stamp = config_stamp();
        if stamp != self.config_stamp {
            self.config_stamp = stamp;
            self.reload();
        }
    }

    fn reload(&mut self) {
        match config::load() {
            Ok(config) if config != self.config => {
                let previous = std::mem::replace(&mut self.config, config);
                self.apply(Some(&previous));
            }
            Ok(_) => {}
            Err(e) => {
                self.tray.notify("Config fehlerhaft", &format!("{e}\nDas letzte gültige Crosshair bleibt aktiv."), true)
            }
        }
    }

    fn toggle_capture(&mut self) {
        let previous = self.config.clone();
        self.config.hide_from_capture = !self.config.hide_from_capture;
        if let Err(e) = config::save(&self.config) {
            self.tray.notify("Speichern fehlgeschlagen", &e, true);
        }
        self.config_stamp = config_stamp();
        self.apply(Some(&previous));
    }
}

fn open(hwnd: HWND, target: &std::path::Path) {
    let target = HSTRING::from(target.as_os_str());
    unsafe {
        let result = ShellExecuteW(Some(hwnd), w!("open"), &target, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            // No program associated with .json: fall back to Notepad.
            ShellExecuteW(Some(hwnd), w!("open"), w!("notepad.exe"), &target, PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

fn show_menu(hwnd: HWND) {
    let Some((shown, capture, key)) =
        with_app(|a| (a.user_shown, a.config.hide_from_capture, a.config.toggle_hotkey.trim().to_string()))
    else {
        return;
    };
    let toggle_label = if key.is_empty() { "Crosshair anzeigen".to_string() } else { format!("Crosshair anzeigen\t{key}") };
    let checked = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
    let command = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING | MF_DEFAULT, MENU_EDITOR, w!("Einstellungen öffnen"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, checked(shown), MENU_TOGGLE, &HSTRING::from(toggle_label));
        let _ = AppendMenuW(menu, checked(capture), MENU_CAPTURE, w!("Im Stream verstecken"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_EDIT, w!("Config-Datei öffnen"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_FOLDER, w!("Config-Ordner öffnen"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Beenden"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as usize
    };
    match command {
        MENU_EDITOR => spawn_editor(),
        MENU_TOGGLE => {
            with_app(App::toggle);
        }
        MENU_CAPTURE => {
            with_app(App::toggle_capture);
        }
        MENU_EDIT => open(hwnd, &config::path()),
        MENU_FOLDER => open(hwnd, &config::dir()),
        MENU_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

unsafe extern "system" fn control_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == TIMER_FULLSCREEN => {
            with_app(App::check_fullscreen);
        }
        WM_TIMER if wparam.0 == TIMER_AIM => {
            with_app(App::check_aim);
        }
        WM_TRAY => match (lparam.0 & 0xFFFF) as u32 {
            WM_LBUTTONUP => spawn_editor(),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        },
        WM_RELOAD => {
            with_app(|a| {
                a.config_stamp = config_stamp();
                a.reload();
            });
        }
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_TOGGLE => {
            with_app(App::toggle);
        }
        WM_DISPLAYCHANGE => {
            // Monitor layout changed: forget the remembered game monitor, it may be gone.
            with_app(|a| {
                a.game_monitor = None;
                a.apply(None);
            });
        }
        WM_DESTROY => unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_AIM);
            PostQuitMessage(0);
        },
        m if m != 0 && m == TASKBAR_CREATED.get() => {
            with_app(|a| {
                a.tray.add();
                a.tray_state = None;
                a.update_tray();
            });
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_app(|a| a.on_foreground(hwnd));
}
