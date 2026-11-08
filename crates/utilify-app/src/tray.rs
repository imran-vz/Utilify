//! The menu-bar status item: a live icon (rates, graph, storage bar, and
//! optionally a temperature and fan speed), a tooltip with the numbers, and
//! a menu with the preferences, keep awake and fans.
//!
//! Left click toggles the dashboard; right click opens the menu. The native
//! tray is main-thread-only and lives in [`NATIVE`]; its event handlers run
//! outside GPUI, so they forward [`TrayEvent`]s through a channel.

use crate::stats::Stats;
use crate::system::{self, RunningApp};
use std::cell::RefCell;
use std::time::{Duration, Instant};
use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use utilify_core::awake::{self, KeepAwake, Mode};
use utilify_core::fans::controller::{FanStatus, HelperStatus};
use utilify_core::fans::service::FanSnapshot;
use utilify_core::fans::{Active, Control, FanSettings};
use utilify_core::format;
use utilify_core::icon::{IconRenderer, Readout, TrayContent};
use utilify_core::monitor::Monitor;
use utilify_core::sensors;
use utilify_core::settings::{INTERVALS_MS, Settings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left click on the status item.
    Clicked,
    Menu(MenuAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    OpenDashboard,
    /// The data workbench window.
    OpenWorkbench,
    /// "Other Time/Until…" and "Keep Awake Settings…": the dashboard's
    /// Awake tab.
    OpenAwakeTab,
    SetInterval(u64),
    ToggleRates,
    ToggleTemp,
    ToggleFanRpm,
    /// "Show Fans…": the dashboard's Fans tab.
    OpenFansTab,
    FanPreset(PresetChoice),
    StartAwake(Mode),
    EndAwake,
    ToggleAllowDisplaySleep,
    ToggleAllowClosedDisplaySleep,
    ToggleScreenSaver,
    ToggleAwakeOnLaunch,
    ToggleNotifyEnd,
    ToggleLaunchAtLogin,
    Quit,
}

/// A preset in the Fans menu. User presets go by their position in
/// [`FanSettings::presets`]; the menu is rebuilt when that list changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetChoice {
    Automatic,
    FullBlast,
    User(usize),
}

impl PresetChoice {
    /// The [`Active`] it selects, if the preset still exists.
    pub fn active(self, settings: &FanSettings) -> Option<Active> {
        Some(match self {
            PresetChoice::Automatic => Active::Automatic,
            PresetChoice::FullBlast => Active::FullBlast,
            PresetChoice::User(i) => Active::Preset(settings.presets.get(i)?.name.clone()),
        })
    }
}

impl MenuAction {
    pub fn id(self) -> String {
        match self {
            MenuAction::OpenDashboard => "dashboard".into(),
            MenuAction::OpenWorkbench => "workbench".into(),
            MenuAction::OpenAwakeTab => "awake:tab".into(),
            MenuAction::SetInterval(ms) => format!("interval:{ms}"),
            MenuAction::ToggleRates => "show_rates".into(),
            MenuAction::ToggleTemp => "show_temp".into(),
            MenuAction::ToggleFanRpm => "show_fan_rpm".into(),
            MenuAction::OpenFansTab => "fans:tab".into(),
            MenuAction::FanPreset(p) => match p {
                PresetChoice::Automatic => "fans:preset:auto".into(),
                PresetChoice::FullBlast => "fans:preset:full".into(),
                PresetChoice::User(i) => format!("fans:preset:user:{i}"),
            },
            MenuAction::StartAwake(mode) => format!(
                "awake:start:{}",
                match mode {
                    Mode::Indefinite => "indefinite".into(),
                    Mode::For(d) => format!("for:{}", d.as_secs()),
                    Mode::Until(m) => format!("until:{m}"),
                    Mode::WhileApp(pid) => format!("app:{pid}"),
                    Mode::WhileFileDownloading => "file".into(),
                    Mode::WhileNetworkBusy => "network".into(),
                }
            ),
            MenuAction::EndAwake => "awake:end".into(),
            MenuAction::ToggleAllowDisplaySleep => "awake:allow_display".into(),
            MenuAction::ToggleAllowClosedDisplaySleep => "awake:allow_closed".into(),
            MenuAction::ToggleScreenSaver => "awake:screen_saver".into(),
            MenuAction::ToggleAwakeOnLaunch => "awake:on_launch".into(),
            MenuAction::ToggleNotifyEnd => "awake:notify_end".into(),
            MenuAction::ToggleLaunchAtLogin => "login".into(),
            MenuAction::Quit => "quit".into(),
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "dashboard" => MenuAction::OpenDashboard,
            "workbench" => MenuAction::OpenWorkbench,
            "awake:tab" => MenuAction::OpenAwakeTab,
            "show_rates" => MenuAction::ToggleRates,
            "show_temp" => MenuAction::ToggleTemp,
            "show_fan_rpm" => MenuAction::ToggleFanRpm,
            "fans:tab" => MenuAction::OpenFansTab,
            "fans:preset:auto" => MenuAction::FanPreset(PresetChoice::Automatic),
            "fans:preset:full" => MenuAction::FanPreset(PresetChoice::FullBlast),
            "awake:end" => MenuAction::EndAwake,
            "awake:allow_display" => MenuAction::ToggleAllowDisplaySleep,
            "awake:allow_closed" => MenuAction::ToggleAllowClosedDisplaySleep,
            "awake:screen_saver" => MenuAction::ToggleScreenSaver,
            "awake:on_launch" => MenuAction::ToggleAwakeOnLaunch,
            "awake:notify_end" => MenuAction::ToggleNotifyEnd,
            "login" => MenuAction::ToggleLaunchAtLogin,
            "quit" => MenuAction::Quit,
            _ => {
                if let Some(i) = id.strip_prefix("fans:preset:user:") {
                    MenuAction::FanPreset(PresetChoice::User(i.parse().ok()?))
                } else if let Some(start) = id.strip_prefix("awake:start:") {
                    let (kind, arg) = start.split_once(':').unwrap_or((start, ""));
                    MenuAction::StartAwake(match kind {
                        "indefinite" => Mode::Indefinite,
                        "for" => Mode::For(Duration::from_secs(arg.parse().ok()?)),
                        "until" => Mode::Until(arg.parse().ok()?),
                        "app" => Mode::WhileApp(arg.parse().ok()?),
                        "file" => Mode::WhileFileDownloading,
                        "network" => Mode::WhileNetworkBusy,
                        _ => return None,
                    })
                } else {
                    MenuAction::SetInterval(id.strip_prefix("interval:")?.parse().ok()?)
                }
            }
        })
    }
}

/// How a fan is driven, for the Fans menu: `Auto`, `Constant 3000 rpm`,
/// `CPU Core Average 50–85°C`.
pub fn control_label(control: &Control, sensors: &[sensors::Sensor]) -> String {
    match control {
        Control::Constant(rpm) => format!("Constant {rpm:.0} rpm"),
        c => c.describe(|id| sensors::find(sensors, id).map(|s| s.name.clone())),
    }
}

/// `Left side: 2317 rpm · Auto`.
pub fn fan_line(fan: &FanStatus, sensors: &[sensors::Sensor]) -> String {
    format!(
        "{}: {:.0} rpm · {}",
        fan.name,
        fan.info.current,
        control_label(&fan.control, sensors)
    )
}

/// Why fan control isn't available, in a few words; `None` when it is.
pub fn helper_note(status: &HelperStatus) -> Option<&'static str> {
    match status {
        HelperStatus::Ready => None,
        HelperStatus::Unknown => Some("Checking the fan helper…"),
        HelperStatus::Unsupported => Some("Fan control needs Apple Silicon"),
        HelperStatus::NotInstalled => Some("Fan helper not installed"),
        HelperStatus::NotRunning => Some("Fan helper not running"),
        HelperStatus::PermissionDenied => Some("Fan control needs an admin user"),
        HelperStatus::Broken(_) => Some("Lost the fan helper"),
    }
}

/// The menu-bar readout the settings ask for, from the latest fan snapshot.
pub fn readout(settings: &Settings, fans: &FanSnapshot) -> Readout {
    let celsius = || {
        sensors::find(&fans.sensors, &settings.menu_temp_sensor)
            .map(|s| s.celsius)
            // Saved before a sensor rename, or not on this Mac.
            .or_else(|| sensors::cpu_average(&fans.sensors))
    };
    // A Mac without fans has nothing to show.
    let fanless = fans.read_at.is_some() && fans.fans.is_empty();
    Readout {
        celsius: settings.show_temp.then(celsius),
        rpm: (settings.show_fan_rpm && !fanless).then(|| fans.fastest_rpm()),
    }
}

/// Hover text for the status item: current rates, free space, and the
/// keep-awake session when one runs.
pub fn tooltip(m: &Monitor, awake: Option<&str>) -> String {
    let r = m.rates();
    let rates = format!("↓ {}    ↑ {}", format::rate(r.down), format::rate(r.up));
    let disk = match m.disk() {
        Some(d) => format!(
            "Disk: {} free of {} ({} used)",
            format::bytes(d.available),
            format::bytes(d.total),
            format::percent(d.used_fraction())
        ),
        None => "Disk: unavailable".into(),
    };
    match awake {
        Some(a) => format!("{rates}\n{disk}\nKeeping awake: {a}"),
        None => format!("{rates}\n{disk}"),
    }
}

pub fn interval_label(ms: u64) -> String {
    utilify_core::format::interval(ms)
}

/// The first line under "Keep Awake", like Amphetamine's session details.
pub fn session_line(k: &KeepAwake, now: Instant) -> String {
    let Some(s) = k.session() else {
        return "No Active Session".into();
    };
    match s.mode {
        Mode::Indefinite => "Indeterminate time remaining".into(),
        Mode::For(_) | Mode::Until(_) => {
            let left = s.remaining(now).unwrap_or_default();
            format!(
                "{} remaining",
                format::countdown(left.as_secs_f64().ceil() as u64)
            )
        }
        _ => s.status(now),
    }
}

pub fn screen_saver_label(settings: &Settings) -> String {
    format!(
        "Allow Screen Saver After {} of Inactivity",
        format::span_long(settings.awake_screen_saver_after().as_secs())
    )
}

/// The running-app list and login state are system queries; refresh them
/// this often rather than every tick.
const SYSTEM_REFRESH: Duration = Duration::from_secs(3);

struct Native {
    tray: tray_icon::TrayIcon,
    intervals: Vec<(u64, CheckMenuItem)>,
    show_rates: CheckMenuItem,
    show_temp: CheckMenuItem,
    show_fan_rpm: CheckMenuItem,
    fans_menu: Submenu,
    /// What the Fans menu was built for: fan count, user presets, and the
    /// helper note. It's rebuilt when that changes, else updated in place.
    fans_shape: Option<(usize, Vec<String>, Option<&'static str>)>,
    fan_lines: Vec<MenuItem>,
    presets: Vec<(PresetChoice, CheckMenuItem)>,
    last_fans_title: Option<String>,
    awake_menu: Submenu,
    session_line: MenuItem,
    activation_line: MenuItem,
    activation_shown: bool,
    allow_display: CheckMenuItem,
    allow_closed: CheckMenuItem,
    screen_saver: CheckMenuItem,
    end_session: MenuItem,
    apps_menu: Submenu,
    apps: Vec<RunningApp>,
    on_launch: CheckMenuItem,
    notify_end: CheckMenuItem,
    login: CheckMenuItem,
    system_checked: Option<Instant>,
    renderer: IconRenderer,
    last_icon: Option<utilify_core::icon::Bitmap>,
    last_tooltip: Option<String>,
    last_awake_title: Option<String>,
}

thread_local! {
    static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) };
}

const AWAKE_TITLE: &str = "Keep Awake";
const FANS_TITLE: &str = "Fans";

fn cmd(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(Modifiers::SUPER), code))
}

fn item(action: MenuAction, text: &str, accelerator: Option<Accelerator>) -> MenuItem {
    MenuItem::with_id(action.id(), text, true, accelerator)
}

fn check(action: MenuAction, text: &str, on: bool) -> CheckMenuItem {
    CheckMenuItem::with_id(action.id(), text, true, on, None)
}

/// A disabled line of text.
fn label(text: &str) -> MenuItem {
    MenuItem::new(text, false, None)
}

/// Create the status item. Must run on the main thread.
pub fn build(settings: &Settings, events: smol::channel::Sender<TrayEvent>) -> anyhow::Result<()> {
    let intervals: Vec<(u64, CheckMenuItem)> = INTERVALS_MS
        .iter()
        .map(|&ms| {
            let item = check(
                MenuAction::SetInterval(ms),
                &interval_label(ms),
                ms == settings.interval_ms,
            );
            (ms, item)
        })
        .collect();
    let refresh = Submenu::new("Refresh Every", true);
    for (_, item) in &intervals {
        refresh.append(item)?;
    }
    let show_rates = check(
        MenuAction::ToggleRates,
        "Show Speeds in Menu Bar",
        settings.show_rates,
    );
    let show_temp = check(
        MenuAction::ToggleTemp,
        "Show Temperature in Menu Bar",
        settings.show_temp,
    );
    let show_fan_rpm = check(
        MenuAction::ToggleFanRpm,
        "Show Fan Speed in Menu Bar",
        settings.show_fan_rpm,
    );
    // Filled by the first update, once the fans have been read.
    let fans_menu = Submenu::new(FANS_TITLE, true);

    // Keep Awake, laid out like Amphetamine's menu: the current session and
    // its options, then the ways to start one, then quick settings.
    let session_line = label("No Active Session");
    let activation_line = label("");
    let allow_display = check(
        MenuAction::ToggleAllowDisplaySleep,
        "Allow Display Sleep",
        settings.awake_allow_display_sleep,
    );
    let allow_closed = check(
        MenuAction::ToggleAllowClosedDisplaySleep,
        "Allow System Sleep When Display Is Closed",
        settings.awake_allow_closed_display_sleep,
    );
    let screen_saver = check(
        MenuAction::ToggleScreenSaver,
        &screen_saver_label(settings),
        settings.awake_screen_saver,
    );
    let end_session = item(MenuAction::EndAwake, "End Current Session", cmd(Code::KeyX));
    end_session.set_enabled(false);

    let minutes = Submenu::new("Minutes", true);
    for m in awake::MINUTES {
        let mode = Mode::For(Duration::from_secs(m * 60));
        minutes.append(&item(MenuAction::StartAwake(mode), &mode.label(), None))?;
    }
    let hours = Submenu::new("Hours", true);
    for h in awake::HOURS {
        let mode = Mode::For(Duration::from_secs(h * 3600));
        hours.append(&item(MenuAction::StartAwake(mode), &mode.label(), None))?;
    }
    let apps_menu = Submenu::new("While App Is Running", true);

    let on_launch = check(
        MenuAction::ToggleAwakeOnLaunch,
        "Start Session When Utilify Launches",
        settings.awake_on_launch,
    );
    let notify_end = check(
        MenuAction::ToggleNotifyEnd,
        "Notify When Session Ends",
        settings.awake_notify_end,
    );
    let login = check(
        MenuAction::ToggleLaunchAtLogin,
        "Launch Utilify at Login",
        false,
    );
    let quick = Submenu::new("Quick Settings", true);
    quick.append_items(&[&on_launch, &notify_end, &login])?;

    let awake_menu = Submenu::new(AWAKE_TITLE, true);
    awake_menu.append_items(&[
        &session_line,
        &PredefinedMenuItem::separator(),
        &allow_display,
        &allow_closed,
        &screen_saver,
        &end_session,
        &PredefinedMenuItem::separator(),
        &label("Start New Session:"),
        &item(
            MenuAction::StartAwake(Mode::Indefinite),
            "Indefinitely",
            cmd(Code::KeyI),
        ),
        &minutes,
        &hours,
        &item(MenuAction::OpenAwakeTab, "Other Time/Until…", None),
        &apps_menu,
        &item(
            MenuAction::StartAwake(Mode::WhileFileDownloading),
            "While File Is Downloading",
            cmd(Code::KeyF),
        ),
        &item(
            MenuAction::StartAwake(Mode::WhileNetworkBusy),
            "While Network Is Busy",
            None,
        ),
        &PredefinedMenuItem::separator(),
        &quick,
        &item(
            MenuAction::OpenAwakeTab,
            "Keep Awake Settings…",
            cmd(Code::Comma),
        ),
    ])?;

    let menu = Menu::new();
    menu.append_items(&[
        &item(MenuAction::OpenDashboard, "Open Dashboard", None),
        // Off in builds without the workbench.
        &MenuItem::with_id(
            MenuAction::OpenWorkbench.id(),
            "Open Workbench",
            crate::workbench::is_available(),
            cmd(Code::KeyB),
        ),
        &awake_menu,
        &fans_menu,
        &PredefinedMenuItem::separator(),
        &refresh,
        &show_rates,
        &show_temp,
        &show_fan_rpm,
        &PredefinedMenuItem::separator(),
        &item(MenuAction::Quit, "Quit Utilify", cmd(Code::KeyQ)),
    ])?;

    let renderer = IconRenderer::new([crate::assets::icon_font()]);
    let first = renderer.tray_icon(&TrayContent::default(), 2.0)?;
    let tray = tray_icon::TrayIconBuilder::new()
        .with_icon(tray_icon::Icon::from_rgba(
            first.rgba,
            first.width,
            first.height,
        )?)
        .with_icon_as_template(true)
        .with_tooltip(utilify_core::PRODUCT_NAME)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;

    let menu_tx = events.clone();
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |e: tray_icon::menu::MenuEvent| {
        if let Some(action) = MenuAction::parse(e.id.as_ref()) {
            let _ = menu_tx.try_send(TrayEvent::Menu(action));
        }
    }));
    tray_icon::TrayIconEvent::set_event_handler(Some(move |e: tray_icon::TrayIconEvent| {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            let _ = events.try_send(TrayEvent::Clicked);
        }
    }));

    NATIVE.with(|n| {
        *n.borrow_mut() = Some(Native {
            tray,
            intervals,
            show_rates,
            show_temp,
            show_fan_rpm,
            fans_menu,
            fans_shape: None,
            fan_lines: Vec::new(),
            presets: Vec::new(),
            last_fans_title: None,
            awake_menu,
            session_line,
            activation_line,
            activation_shown: false,
            allow_display,
            allow_closed,
            screen_saver,
            end_session,
            apps_menu,
            apps: Vec::new(),
            on_launch,
            notify_end,
            login,
            system_checked: None,
            renderer,
            last_icon: None,
            last_tooltip: None,
            last_awake_title: None,
        })
    });
    Ok(())
}

/// The name of an app listed under "While App Is Running".
pub fn app_name(pid: i32) -> Option<String> {
    NATIVE.with(|n| {
        let n = n.borrow();
        let app = n.as_ref()?.apps.iter().find(|a| a.pid == pid)?;
        Some(app.name.clone())
    })
}

/// Redraw the icon, tooltip and menu from the latest sample. Only touches
/// the native tray for parts that changed.
pub fn update(stats: &Stats) {
    let (monitor, settings, keep_awake) = (&stats.monitor, &stats.settings, &stats.keep_awake);
    NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        let Some(n) = n.as_mut() else { return };
        let now = Instant::now();

        let points = monitor.history().tail(utilify_core::icon::SLOTS);
        let r = monitor.rates();
        let content = TrayContent {
            points: &points,
            rates: settings.show_rates.then_some((r.down, r.up)),
            disk_used: monitor.disk().map(|d| d.used_fraction()),
            awake: keep_awake.is_active(),
            readout: readout(settings, &stats.fans),
        };
        match n.renderer.tray_icon(&content, 2.0) {
            Ok(bmp) if n.last_icon.as_ref() != Some(&bmp) => {
                match tray_icon::Icon::from_rgba(bmp.rgba.clone(), bmp.width, bmp.height) {
                    Ok(icon) => {
                        if let Err(e) = n.tray.set_icon_with_as_template(Some(icon), true) {
                            log::warn!("Setting the tray icon failed: {e}");
                        } else {
                            n.last_icon = Some(bmp);
                        }
                    }
                    Err(e) => log::warn!("Bad tray icon bitmap: {e}"),
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("Rendering the tray icon failed: {e}"),
        }

        let status = keep_awake.status(now);
        let tip = tooltip(monitor, status.as_deref());
        if n.last_tooltip.as_ref() != Some(&tip) {
            n.tray.set_tooltip(Some(&tip)).ok();
            n.last_tooltip = Some(tip);
        }

        for (ms, item) in &n.intervals {
            item.set_checked(*ms == settings.interval_ms);
        }
        n.show_rates.set_checked(settings.show_rates);
        n.show_temp.set_checked(settings.show_temp);
        n.show_fan_rpm.set_checked(settings.show_fan_rpm);
        n.update_awake(settings, keep_awake, status, now);
        n.update_fans(&stats.fans, &stats.fan_settings);
    });
}

impl Native {
    /// The Fans menu, laid out like Macs Fan Control's: each fan and how
    /// it's driven, then the presets with a check on the active one, then
    /// Show Fans.
    fn update_fans(&mut self, fans: &FanSnapshot, settings: &FanSettings) {
        let note = if fans.overheat.is_some() {
            Some("Too hot: fans on automatic")
        } else if fans.foreign && fans.helper.is_ready() {
            Some("Another app is controlling the fans")
        } else {
            helper_note(&fans.helper)
        };
        let names: Vec<String> = settings.presets.iter().map(|p| p.name.clone()).collect();
        let shape = (fans.fans.len(), names, note);
        if self.fans_shape.as_ref() != Some(&shape) {
            self.rebuild_fans(&shape, settings);
            self.fans_shape = Some(shape);
        }
        for (line, fan) in self.fan_lines.iter().zip(&fans.fans) {
            line.set_text(fan_line(fan, &fans.sensors));
        }
        let ready = fans.helper.is_ready();
        for (choice, item) in &self.presets {
            item.set_checked(choice.active(settings).as_ref() == Some(&settings.active));
            item.set_enabled(ready || *choice == PresetChoice::Automatic);
        }
        let title = match settings.active {
            Active::Automatic => FANS_TITLE.to_string(),
            ref a => format!("{FANS_TITLE}: {}", a.label()),
        };
        if self.last_fans_title.as_ref() != Some(&title) {
            self.fans_menu.set_text(&title);
            self.last_fans_title = Some(title);
        }
    }

    fn rebuild_fans(
        &mut self,
        (count, names, note): &(usize, Vec<String>, Option<&'static str>),
        settings: &FanSettings,
    ) {
        while self.fans_menu.remove_at(0).is_some() {}
        self.fan_lines = (0..*count).map(|_| label("")).collect();
        if *count == 0 {
            let _ = self.fans_menu.append(&label("No Fans"));
        }
        for line in &self.fan_lines {
            let _ = self.fans_menu.append(line);
        }
        let _ = self.fans_menu.append(&PredefinedMenuItem::separator());
        let mut choices = vec![
            (PresetChoice::Automatic, "Automatic".to_string()),
            (PresetChoice::FullBlast, "Full Blast".to_string()),
        ];
        choices.extend(
            names
                .iter()
                .enumerate()
                .map(|(i, n)| (PresetChoice::User(i), n.clone())),
        );
        self.presets = choices
            .into_iter()
            .map(|(choice, text)| {
                let on = choice.active(settings).as_ref() == Some(&settings.active);
                (choice, check(MenuAction::FanPreset(choice), &text, on))
            })
            .collect();
        for (_, item) in &self.presets {
            let _ = self.fans_menu.append(item);
        }
        let _ = self.fans_menu.append(&PredefinedMenuItem::separator());
        if let Some(note) = note {
            let _ = self.fans_menu.append(&label(note));
        }
        let _ = self
            .fans_menu
            .append(&item(MenuAction::OpenFansTab, "Show Fans…", None));
    }

    fn update_awake(
        &mut self,
        settings: &Settings,
        keep_awake: &KeepAwake,
        status: Option<String>,
        now: Instant,
    ) {
        let title = match &status {
            Some(s) => format!("{AWAKE_TITLE}: {s}"),
            None => AWAKE_TITLE.to_string(),
        };
        if self.last_awake_title.as_ref() != Some(&title) {
            self.awake_menu.set_text(&title);
            self.last_awake_title = Some(title);
        }
        self.session_line.set_text(session_line(keep_awake, now));
        // "Manual Activation" shows under the time only while a session runs.
        let activation = keep_awake.session().map(|s| s.activation.label());
        if let Some(text) = activation {
            self.activation_line.set_text(text);
        }
        if activation.is_some() != self.activation_shown {
            let ok = if activation.is_some() {
                self.awake_menu.insert(&self.activation_line, 1)
            } else {
                self.awake_menu.remove(&self.activation_line)
            };
            if ok.is_ok() {
                self.activation_shown = activation.is_some();
            }
        }
        self.end_session.set_enabled(keep_awake.is_active());
        self.allow_display
            .set_checked(settings.awake_allow_display_sleep);
        self.allow_closed
            .set_checked(settings.awake_allow_closed_display_sleep);
        self.screen_saver.set_checked(settings.awake_screen_saver);
        self.screen_saver.set_text(screen_saver_label(settings));
        // The screen saver only needs starting while the display is kept on.
        self.screen_saver
            .set_enabled(!settings.awake_allow_display_sleep);
        self.on_launch.set_checked(settings.awake_on_launch);
        self.notify_end.set_checked(settings.awake_notify_end);

        if self
            .system_checked
            .is_none_or(|t| now.duration_since(t) >= SYSTEM_REFRESH)
        {
            self.system_checked = Some(now);
            self.refresh_apps();
            match system::login::enabled() {
                Some(on) => self.login.set_checked(on),
                None => self.login.set_enabled(false),
            }
        }
    }

    /// Rebuild "While App Is Running" when the set of apps changed.
    fn refresh_apps(&mut self) {
        let apps = system::running_apps();
        if apps == self.apps {
            return;
        }
        while self.apps_menu.remove_at(0).is_some() {}
        if apps.is_empty() {
            let _ = self.apps_menu.append(&label("No Apps Running"));
        }
        for app in &apps {
            let _ = self.apps_menu.append(&item(
                MenuAction::StartAwake(Mode::WhileApp(app.pid)),
                &app.name,
                None,
            ));
        }
        self.apps = apps;
    }
}

/// Force the next [`update`] to re-read the login state and app list, after
/// changing them.
pub fn refresh_system() {
    NATIVE.with(|n| {
        if let Some(n) = n.borrow_mut().as_mut() {
            n.system_checked = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use utilify_core::awake::{Activation, Session};
    use utilify_core::disk::DiskUsage;
    use utilify_core::net::InterfaceCounters;

    #[test]
    fn menu_ids_round_trip() {
        for a in [
            MenuAction::OpenDashboard,
            MenuAction::OpenWorkbench,
            MenuAction::OpenAwakeTab,
            MenuAction::SetInterval(500),
            MenuAction::SetInterval(5000),
            MenuAction::ToggleRates,
            MenuAction::ToggleTemp,
            MenuAction::ToggleFanRpm,
            MenuAction::OpenFansTab,
            MenuAction::FanPreset(PresetChoice::Automatic),
            MenuAction::FanPreset(PresetChoice::FullBlast),
            MenuAction::FanPreset(PresetChoice::User(3)),
            MenuAction::StartAwake(Mode::Indefinite),
            MenuAction::StartAwake(Mode::For(Duration::from_secs(900))),
            MenuAction::StartAwake(Mode::Until(18 * 60 + 30)),
            MenuAction::StartAwake(Mode::WhileApp(4242)),
            MenuAction::StartAwake(Mode::WhileFileDownloading),
            MenuAction::StartAwake(Mode::WhileNetworkBusy),
            MenuAction::EndAwake,
            MenuAction::ToggleAllowDisplaySleep,
            MenuAction::ToggleAllowClosedDisplaySleep,
            MenuAction::ToggleScreenSaver,
            MenuAction::ToggleAwakeOnLaunch,
            MenuAction::ToggleNotifyEnd,
            MenuAction::ToggleLaunchAtLogin,
            MenuAction::Quit,
        ] {
            assert_eq!(MenuAction::parse(&a.id()), Some(a));
        }
        assert_eq!(MenuAction::parse("interval:abc"), None);
        assert_eq!(MenuAction::parse("bogus"), None);
        assert_eq!(MenuAction::parse("awake:start:for:soon"), None);
        assert_eq!(MenuAction::parse("awake:start:nap"), None);
        assert_eq!(MenuAction::parse("fans:preset:user:x"), None);
    }

    #[test]
    fn interval_labels() {
        assert_eq!(interval_label(500), "0.5 s");
        assert_eq!(interval_label(2000), "2 s");
    }

    #[test]
    fn session_lines_read_like_amphetamine() {
        let t0 = Instant::now();
        let mut k = KeepAwake::default();
        assert_eq!(session_line(&k, t0), "No Active Session");
        let start = |mode| Session::new(mode, Activation::Manual, None, t0, 0);
        k.start(start(Mode::Indefinite));
        assert_eq!(session_line(&k, t0), "Indeterminate time remaining");
        k.start(start(Mode::For(Duration::from_secs(300))));
        assert_eq!(session_line(&k, t0), "5:00 remaining");
        assert_eq!(
            session_line(&k, t0 + Duration::from_millis(1500)),
            "4:59 remaining"
        );
        k.start(start(Mode::WhileFileDownloading));
        assert_eq!(session_line(&k, t0), "While a file downloads");
        assert_eq!(
            screen_saver_label(&Settings::default()),
            "Allow Screen Saver After 45 min of Inactivity"
        );
    }

    #[test]
    fn tooltip_shows_rates_and_disk() {
        let iface = |rx| InterfaceCounters {
            name: "en0".into(),
            up: true,
            running: true,
            loopback: false,
            rx_bytes: rx,
            tx_bytes: 0,
            baudrate: 1_000_000_000,
        };
        let mut m = Monitor::new(10);
        let t0 = Instant::now();
        let disk = || {
            Some(DiskUsage {
                total: 500_000_000_000,
                available: 200_000_000_000,
            })
        };
        m.tick_with(t0, &[iface(0)], disk);
        m.tick_with(t0 + Duration::from_secs(1), &[iface(12_500_000)], disk);
        assert_eq!(
            tooltip(&m, None),
            "↓ 12.5 MB/s    ↑ 0 B/s\nDisk: 200 GB free of 500 GB (60% used)"
        );
        assert_eq!(
            tooltip(&Monitor::new(10), Some("4:59 left")),
            "↓ 0 B/s    ↑ 0 B/s\nDisk: unavailable\nKeeping awake: 4:59 left"
        );
    }

    fn fan(index: u8, current: f32, control: Control) -> FanStatus {
        FanStatus {
            info: utilify_smc::FanInfo {
                index,
                current,
                min: 1000.0,
                max: 5000.0,
                target: current,
                manual: false,
            },
            name: utilify_core::fans::fan_name(index, 2),
            control,
            target: None,
            issue: None,
        }
    }

    fn cpu(c: f32) -> sensors::Sensor {
        sensors::Sensor {
            id: sensors::CPU_AVERAGE_ID.into(),
            name: "CPU Core Average".into(),
            group: sensors::SensorGroup::Cpu,
            celsius: c,
        }
    }

    #[test]
    fn fan_lines_read_like_macs_fan_control() {
        let s = [cpu(50.0)];
        assert_eq!(
            fan_line(&fan(0, 2316.6, Control::Auto), &s),
            "Left side: 2317 rpm · Auto"
        );
        assert_eq!(
            fan_line(&fan(1, 3000.0, Control::Constant(3000.0)), &s),
            "Right side: 3000 rpm · Constant 3000 rpm"
        );
        let curve = Control::SensorCurve {
            sensor_id: sensors::CPU_AVERAGE_ID.into(),
            from_c: 50.0,
            to_c: 85.0,
        };
        assert_eq!(
            fan_line(&fan(1, 2000.0, curve), &s),
            "Right side: 2000 rpm · CPU Core Average 50–85°C"
        );
        assert_eq!(helper_note(&HelperStatus::Ready), None);
        assert_eq!(
            helper_note(&HelperStatus::NotInstalled),
            Some("Fan helper not installed")
        );
    }

    #[test]
    fn preset_choices_map_to_active() {
        let mut settings = FanSettings::default();
        assert_eq!(
            PresetChoice::FullBlast.active(&settings),
            Some(Active::FullBlast)
        );
        assert_eq!(PresetChoice::User(0).active(&settings), None);
        settings.presets.push(utilify_core::fans::Preset {
            name: "Quiet".into(),
            controls: vec![],
        });
        assert_eq!(
            PresetChoice::User(0).active(&settings),
            Some(Active::Preset("Quiet".into()))
        );
    }

    #[test]
    fn readout_follows_the_settings() {
        let fans = FanSnapshot {
            fans: vec![fan(0, 2000.0, Control::Auto), fan(1, 2500.0, Control::Auto)],
            sensors: vec![cpu(61.0)],
            read_at: Some(Instant::now()),
            ..Default::default()
        };
        let mut settings = Settings::default();
        assert_eq!(readout(&settings, &fans), Readout::default());
        settings.show_temp = true;
        settings.show_fan_rpm = true;
        assert_eq!(
            readout(&settings, &fans),
            Readout {
                celsius: Some(Some(61.0)),
                rpm: Some(Some(2500.0)),
            }
        );
        // An unknown sensor falls back to the CPU average; before the first
        // reading there's nothing yet.
        settings.menu_temp_sensor = "gone".into();
        assert_eq!(readout(&settings, &fans).celsius, Some(Some(61.0)));
        assert_eq!(
            readout(&settings, &FanSnapshot::default()),
            Readout {
                celsius: Some(None),
                rpm: Some(None),
            }
        );
        // Fanless: no rpm line at all.
        let fanless = FanSnapshot {
            read_at: Some(Instant::now()),
            ..Default::default()
        };
        assert_eq!(readout(&settings, &fanless).rpm, None);
    }
}
