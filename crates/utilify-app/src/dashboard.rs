//! The dashboard popover shown under the status item, in six tabs:
//! Overview (rates, storage and keep awake at a glance), Network (link, session totals
//! and the speed test), Storage (used space over time), Awake (keep-awake
//! sessions, see [`awake`]), Fans (fan control and temperatures, see
//! [`fans`]) and Settings.
//! The look follows `design/dashboard-mock.html`; the pieces live in
//! [`crate::ui`].

use crate::stats::{self, SpeedTest, Stats};
use crate::ui::{self, color::*, px, radius};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;
use std::time::{Duration, Instant};
use utilify_core::disk::DiskUsage;
use utilify_core::disk_history::{self, DiskSample};
use utilify_core::format;
use utilify_core::history::{self, History, RatePoint};
use utilify_core::icon::MIN_PEAK;
use utilify_core::net::NetRates;
use utilify_core::settings::{INTERVALS_MS, Settings};
use utilify_core::speedtest;

mod awake;
mod fans;
mod status;

pub const WIDTH: f32 = ui::CARD_W + 2. * ui::SHADOW;
/// The tallest the window gets, in design units; on shorter screens it's
/// cut to fit and the tab's content scrolls. The window is transparent
/// below the card, and clicking there closes the dashboard.
pub const HEIGHT: f32 = 1130.;
/// What the card spends outside the tab's content: header, tab bar,
/// panel padding and the shadow margin.
const CHROME_H: f32 = 236.;
const KEY_CONTEXT: &str = "Dashboard";
/// How long a toast stays up; one with an Undo stays longer.
const TOAST_FOR: Duration = Duration::from_secs(3);
const UNDO_FOR: Duration = Duration::from_secs(6);
/// A second click within this long confirms Reset.
const CONFIRM_FOR: Duration = Duration::from_secs(4);

actions!(
    dashboard,
    [
        Dismiss,
        NextTab,
        PrevTab,
        ShowOverview,
        ShowNetwork,
        ShowStorage,
        ShowAwake,
        ShowFans,
        ShowSettings,
        OpenWorkbench,
        QuitApp,
    ]
);

/// Keyboard shortcuts while the dashboard has focus.
pub fn init(cx: &mut App) {
    let k = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", Dismiss, k),
        KeyBinding::new("cmd-w", Dismiss, k),
        KeyBinding::new("ctrl-tab", NextTab, k),
        KeyBinding::new("ctrl-shift-tab", PrevTab, k),
        KeyBinding::new("cmd-shift-]", NextTab, k),
        KeyBinding::new("cmd-shift-[", PrevTab, k),
        KeyBinding::new("cmd-1", ShowOverview, k),
        KeyBinding::new("cmd-2", ShowNetwork, k),
        KeyBinding::new("cmd-3", ShowStorage, k),
        KeyBinding::new("cmd-4", ShowAwake, k),
        KeyBinding::new("cmd-5", ShowFans, k),
        KeyBinding::new("cmd-6", ShowSettings, k),
        KeyBinding::new("cmd-,", ShowSettings, k),
        KeyBinding::new("cmd-b", OpenWorkbench, k),
        KeyBinding::new("cmd-q", QuitApp, k),
    ]);
}
/// Bars in the rate graph; each covers `HISTORY_LEN / BARS` samples.
const BARS: usize = 40;
const CHART_H: f32 = 210.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Overview,
    Network,
    Storage,
    Awake,
    Fans,
    Settings,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::Network,
        Tab::Storage,
        Tab::Awake,
        Tab::Fans,
        Tab::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Network => "Network",
            Tab::Storage => "Storage",
            Tab::Awake => "Awake",
            Tab::Fans => "Fans",
            Tab::Settings => "Settings",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn next(self) -> Tab {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Tab {
        let n = Self::ALL.len();
        Self::ALL[(self.index() + n - 1) % n]
    }

    fn icon(self) -> &'static str {
        match self {
            Tab::Overview => "layout-dashboard",
            Tab::Network => "activity",
            Tab::Storage => "hard-drive",
            Tab::Awake => "coffee",
            Tab::Fans => "fan",
            Tab::Settings => "settings",
        }
    }

    pub fn parse(name: &str) -> Option<Tab> {
        Self::ALL
            .into_iter()
            .find(|t| t.label().eq_ignore_ascii_case(name))
    }
}

/// A short confirmation along the card's bottom edge.
pub(crate) struct Toast {
    tone: ui::Tone,
    message: SharedString,
    /// Label and handler of a button on it, like Undo.
    action: Option<(SharedString, ui::Callback)>,
}

pub struct Dashboard {
    stats: Entity<Stats>,
    tab: Tab,
    focus: FocusHandle,
    interval_menu_open: bool,
    /// Whether the speed test section on Network is unfolded.
    speed_test_open: bool,
    /// Pointer x over the storage chart, in design units from its left edge.
    storage_hover: Option<f32>,
    /// The Fans tab's preset name field, the preset being renamed, and a
    /// note under the field.
    preset_name: Entity<InputState>,
    fan_renaming: Option<String>,
    fan_hint: Option<SharedString>,
    /// The Settings tab's menu-bar sensor list is unfolded.
    temp_menu_open: bool,
    /// Reset was clicked once; a second click before this confirms it.
    reset_armed: Option<Instant>,
    toast: Option<Toast>,
    toast_task: Option<Task<()>>,
    /// Tallest the tab's content may be before it scrolls, in design units.
    body_max_h: f32,
    _subscriptions: Vec<Subscription>,
}

impl Dashboard {
    pub fn new(
        stats: Entity<Stats>,
        body_max_h: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&stats, |_, _, cx| cx.notify());
        // Dismiss like a popover when focus moves elsewhere. Debug builds
        // can pin it open with UTILIFY_DEV_PIN, for inspecting it.
        let pinned = cfg!(debug_assertions) && std::env::var_os("UTILIFY_DEV_PIN").is_some();
        let activation = cx.observe_window_activation(window, move |_, window, cx| {
            if !window.is_window_active() && !pinned {
                dismiss(window, cx);
            }
        });
        // Light or dark, live: the palette follows the OS.
        let appearance = cx.observe_window_appearance(window, |_, _, cx| {
            crate::theme::follow_system(cx);
            cx.notify();
        });
        let preset_name = cx.new(|cx| InputState::new(window, cx).placeholder("Preset name"));
        let enter = cx.subscribe_in(&preset_name, window, |this, _, ev, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.save_preset(window, cx);
            }
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            stats,
            tab: state(cx).tab,
            focus,
            speed_test_open: state(cx).speed_test_open,
            interval_menu_open: false,
            storage_hover: None,
            preset_name,
            fan_renaming: None,
            fan_hint: None,
            temp_menu_open: false,
            reset_armed: None,
            toast: None,
            toast_task: None,
            body_max_h,
            _subscriptions: vec![observe, activation, appearance, enter],
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.interval_menu_open = false;
        self.temp_menu_open = false;
        self.storage_hover = None;
        self.reset_armed = None;
        state(cx).tab = tab;
        cx.notify();
    }

    /// Show a toast for a few seconds, replacing any showing.
    pub(crate) fn show_toast(
        &mut self,
        tone: ui::Tone,
        message: impl Into<SharedString>,
        action: Option<(&str, ui::Callback)>,
        cx: &mut Context<Self>,
    ) {
        let stay = if action.is_some() { UNDO_FOR } else { TOAST_FOR };
        self.toast = Some(Toast {
            tone,
            message: message.into(),
            action: action.map(|(l, f)| (SharedString::from(l.to_string()), f)),
        });
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(stay).await;
            let _ = this.update(cx, |d, cx| {
                d.toast = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn toast_view(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.toast.as_ref()?;
        let action = t.action.as_ref().map(|(label, f)| {
            let f = f.clone();
            ui::banner_button("toast-action", label)
                .text_color(fg())
                .font_weight(FontWeight::MEDIUM)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toast = None;
                    f(window, cx);
                    cx.notify();
                }))
        });
        Some(ui::toast(t.tone, t.message.clone(), action))
    }
}

/// What the tabs show, copied out of [`Stats`] so rendering can borrow the
/// view mutably for its listeners.
struct Snapshot {
    rates: NetRates,
    points: Vec<RatePoint>,
    capacity: usize,
    settings: Settings,
    session: (u64, u64),
    disk: Option<DiskUsage>,
    disk_samples: Vec<DiskSample>,
    speed_test: SpeedTest,
    awake: awake::Snapshot,
    fans: fans::Snapshot,
    power: utilify_core::power::PowerState,
    problems: Vec<stats::Problem>,
    health: Vec<utilify_core::health::Check>,
    health_at: Option<Instant>,
    health_running: bool,
}

impl Snapshot {
    fn read(stats: &Stats) -> Self {
        let m = &stats.monitor;
        let disk = m.disk();
        Self {
            rates: m.rates().clone(),
            points: m.history().iter().copied().collect(),
            capacity: m.history().capacity(),
            settings: stats.settings.clone(),
            session: m.session_totals(),
            disk,
            disk_samples: stats.disk_history.with_live(disk_history::unix_now(), disk),
            speed_test: stats.speed_test.clone(),
            awake: awake::Snapshot::read(stats),
            fans: fans::Snapshot::read(stats),
            power: stats.power,
            problems: stats.problems.clone(),
            health: stats.health.clone(),
            health_at: stats.health_at,
            health_running: stats.health_running(),
        }
    }

    /// How far back the rate graph reaches.
    fn graph_span(&self) -> Duration {
        Duration::from_millis(self.settings.interval_ms) * self.capacity as u32
    }

    /// First and last stored disk samples, when there are two to compare.
    fn disk_range(&self) -> Option<(DiskSample, DiskSample)> {
        match (self.disk_samples.first(), self.disk_samples.last()) {
            (Some(a), Some(b)) if self.disk_samples.len() >= 2 && b.at > a.at => Some((*a, *b)),
            _ => None,
        }
    }
}

fn live_label(rates: &NetRates) -> AnyElement {
    match rates.link {
        Some(_) => ui::pill("live", ok()).into_any_element(),
        None => ui::pill("offline", danger()).into_any_element(),
    }
}

fn text(s: impl Into<SharedString>) -> AnyElement {
    div().child(s.into()).into_any_element()
}

/// The bar graph with its caption, and the live rates under it. Shared by
/// Overview and Network.
fn rates_section(s: &Snapshot) -> Div {
    let peak = History::peak(&s.points);
    v_flex()
        .gap(px(14.))
        .child(
            v_flex()
                .gap(px(8.))
                .child(ui::rate_bars(
                    history::bars(&s.points, s.capacity, BARS),
                    peak.max(MIN_PEAK),
                ))
                .child(ui::caption(
                    ui::mono(ui::text::MICRO, fg3()).child(format!("peak {}", format::rate(peak))),
                    ui::mono(ui::text::MICRO, fg3())
                        .child(format!("last {}", format::span(s.graph_span().as_secs()))),
                )),
        )
        .child(
            h_flex()
                .gap(px(12.))
                .child(ui::rate_box(
                    "arrow-down",
                    "Download",
                    &format::rate(s.rates.down),
                ))
                .child(ui::rate_box("arrow-up", "Upload", &format::rate(s.rates.up))),
        )
}

/// Used of total as a bar, with free space and the change over the stored
/// history. Shared by Overview and Storage.
fn storage_section(s: &Snapshot) -> Div {
    let Some(d) = s.disk else {
        return v_flex().child(ui::empty_state(
            "hard-drive",
            "Can't read the startup disk",
            "Utilify keeps trying; this clears once macOS answers.",
        ));
    };
    let change = match s.disk_range() {
        Some((first, last)) => format!(
            "{} in {}",
            format::bytes_delta(last.used as i64 - first.used as i64),
            format::span(last.at - first.at)
        ),
        None => "collecting history…".into(),
    };
    let level = s.settings.storage_level(d.used_fraction());
    v_flex()
        .gap(px(10.))
        .child(ui::label_row(
            "Startup disk",
            h_flex()
                .items_baseline()
                .gap(px(6.))
                .child(ui::mono(ui::text::LABEL, fg()).child(format::bytes(d.used())))
                .child(
                    ui::mono(ui::text::SMALL, fg3()).child(format!("of {}", format::bytes(d.total))),
                ),
        ))
        .child(ui::storage_bar(d.used_fraction(), ui::level_color(level)))
        .child(ui::caption(
            ui::dotted(vec![
                text(format!("{} used", format::percent(d.used_fraction()))),
                text(format!("{} free", format::bytes(d.available))),
            ]),
            div().text_color(fg2()).child(change),
        ))
}

/// A bordered group of settings rows, ruled off from each other.
fn group(rows: Vec<AnyElement>) -> Div {
    let mut g = v_flex()
        .rounded(radius(ui::R_MD))
        .border_1()
        .border_color(control_border())
        .bg(control());
    for (i, row) in rows.into_iter().enumerate() {
        g = g.child(
            div()
                .px(px(14.))
                .when(i > 0, |d| d.border_t_1().border_color(sep()))
                .child(row),
        );
    }
    g
}

impl Dashboard {
    fn header(&self, s: &Snapshot) -> Div {
        let link = s.rates.link.as_ref();
        let (icon, title, subtitle, chip): (&str, SharedString, Vec<AnyElement>, _) = match self.tab
        {
            Tab::Overview => {
                let mut parts = vec![];
                if let Some(l) = link {
                    parts.push(text(l.name.clone()));
                }
                if link.is_none() {
                    parts.push(live_label(&s.rates));
                }
                if let Some(d) = s.disk {
                    parts.push(text(format!("{} free", format::bytes(d.available))));
                }
                if s.power.on_battery {
                    // The same (clamped) threshold the low-battery alert uses.
                    let threshold = s.settings.alert_prefs().low_battery_pct;
                    let low = s.power.percent.is_some_and(|p| p <= threshold);
                    let label = match s.power.percent {
                        Some(p) => format!("battery {p}%"),
                        None => "on battery".into(),
                    };
                    parts.push(
                        div()
                            .text_color(if low { caution() } else { fg2() })
                            .child(label)
                            .into_any_element(),
                    );
                }
                ("layout-grid", "Utilify".into(), parts, None)
            }
            Tab::Network => {
                let mut parts = vec![];
                if let Some(l) = link {
                    parts.push(text(l.name.clone()));
                    if l.capacity_bps > 0 {
                        parts.push(text(format::bits_per_sec(l.capacity_bps)));
                    }
                }
                if link.is_none() {
                    parts.push(live_label(&s.rates));
                }
                ("activity", "Network".into(), parts, None)
            }
            Tab::Storage => {
                let mut parts = vec![text("Startup disk")];
                if let Some(d) = s.disk {
                    parts.push(text(format::bytes(d.total)));
                }
                let chip = s
                    .disk_range()
                    .map(|(a, b)| SharedString::from(format::span(b.at - a.at)));
                ("hard-drive", "Storage".into(), parts, chip)
            }
            Tab::Awake => {
                let (title, parts) = awake::header(s);
                ("pill", title, parts, None)
            }
            Tab::Fans => {
                let (title, parts, chip) = fans::header(s);
                ("fan", title, parts, chip)
            }
            Tab::Settings => (
                "sliders-horizontal",
                "Settings".into(),
                vec![
                    text(utilify_core::PRODUCT_NAME),
                    text(env!("CARGO_PKG_VERSION")),
                ],
                None,
            ),
        };
        let dot = if link.is_some() { ok() } else { danger() };
        let tile = if self.tab == Tab::Overview {
            ui::header_logo(dot)
        } else {
            ui::header_icon(icon, dot)
        };
        ui::header(
            ui::icon_button("tab-icon", tile),
            title,
            ui::dotted(subtitle),
            chip,
            h_flex().flex_none().gap(px(8.)).child(
                ui::icon_action("open-workbench", "chef-hat", "Open Workbench", Some("⌘B"))
                    .on_click(|_, _, cx| crate::workbench::open(cx)),
            ),
        )
    }

    fn tabs(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let items = Tab::ALL
            .iter()
            .enumerate()
            .map(|(i, &t)| ui::TabItem {
                id: t.label().into(),
                icon: t.icon(),
                label: t.label(),
                keys: format!("⌘{}", i + 1).into(),
                badge: tab_badge(t, s),
                on_click: Box::new(cx.listener(move |this, _, _, cx| this.set_tab(t, cx))),
            })
            .collect();
        ui::tab_bar(items, self.tab.index())
    }

    fn overview(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap(px(20.))
            .children(self.issue_banners(s, cx))
            .child(ui::section(
                "Network",
                Some(live_label(&s.rates)),
                rates_section(s),
            ))
            .child(ui::separator())
            .child(ui::section("Storage", None, storage_section(s)))
            .child(ui::separator())
            .child(awake::overview_section(s))
    }

    fn network(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let link: AnyElement = match &s.rates.link {
            Some(l) => {
                let mut parts = vec![text(l.name.clone())];
                if l.capacity_bps > 0 {
                    parts.push(
                        div()
                            .text_color(fg3())
                            .child(format::bits_per_sec(l.capacity_bps))
                            .into_any_element(),
                    );
                }
                if let Some(u) = s.rates.utilization() {
                    parts.push(text(format!("{} used", format::percent(u))));
                }
                ui::dotted(parts).into_any_element()
            }
            None => div()
                .text_color(danger())
                .child("Not connected")
                .into_any_element(),
        };
        let (down, up) = s.session;
        v_flex()
            .gap(px(20.))
            .child(ui::section(
                "Throughput",
                Some(live_label(&s.rates)),
                rates_section(s),
            ))
            .child(ui::section(
                "Connection",
                None,
                v_flex()
                    .gap(px(6.))
                    .child(ui::kv("Link", link))
                    .child(ui::kv(
                        "This session",
                        ui::dotted(vec![
                            text(format!("↓ {}", format::bytes(down))),
                            text(format!("↑ {}", format::bytes(up))),
                        ]),
                    )),
            ))
            .child(ui::separator())
            .child(self.speed_test(s, cx))
    }

    /// Folds to one summary line; unfolded it shows the bars, latency and
    /// the Run test button.
    fn speed_test(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let open = self.speed_test_open;
        let head = h_flex()
            .id("speed-test-head")
            .justify_between()
            .cursor_pointer()
            .rounded(radius(ui::R_SM))
            .child(ui::section_label("Speed test"))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(speed_test_status(s, open))
                    .child(ui::icon(
                        if open { "chevron-up" } else { "chevron-down" },
                        14.,
                        fg3(),
                    )),
            )
            .tooltip(ui::tip(if open { "Fold" } else { "Unfold" }, None))
            .on_click(cx.listener(|this, _, _, cx| {
                this.speed_test_open = !this.speed_test_open;
                state(cx).speed_test_open = this.speed_test_open;
                cx.notify();
            }));
        v_flex().gap(px(12.)).child(head).when(open, |v| {
            v.child(speed_test_body(s))
                .child(div().h(px(6.)))
                .child(speed_test_button(s, cx))
        })
    }

    fn storage(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap(px(20.))
            .child(ui::section(
                "Used space",
                Some(
                    ui::ui_text(ui::text::CAPTION, fg3())
                        .child("last 30 days")
                        .into_any_element(),
                ),
                storage_chart(s, self.storage_hover, cx.entity().downgrade()),
            ))
            .child(storage_section(s))
    }

    fn settings(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let show_rates = s.settings.show_rates;
        let mut menu_bar: Vec<AnyElement> = vec![
            setting_row(
                "Refresh every",
                "How often the icon and graphs update",
                self.interval_dropdown(&s.settings, cx),
            )
            .into_any_element(),
            setting_row(
                "Show speeds",
                "Rates as text next to the icon",
                ui::toggle("show-rates", show_rates).on_click(|_, _, cx| {
                    crate::update_settings(cx, |s| s.show_rates = !s.show_rates);
                }),
            )
            .into_any_element(),
            setting_row(
                "Show temperature",
                &format!("{}, like 80°C", self.menu_sensor_name(s)),
                ui::toggle("show-temp", s.settings.show_temp).on_click(|_, _, cx| {
                    crate::update_settings(cx, |s| s.show_temp = !s.show_temp);
                }),
            )
            .into_any_element(),
        ];
        if s.settings.show_temp {
            menu_bar.push(self.temp_sensor_picker(s, cx).into_any_element());
        }
        menu_bar.push(
            setting_row(
                "Show fan speed",
                "The fastest fan, like 5775rpm",
                ui::toggle("show-fan-rpm", s.settings.show_fan_rpm).on_click(|_, _, cx| {
                    crate::update_settings(cx, |s| s.show_fan_rpm = !s.show_fan_rpm);
                }),
            )
            .into_any_element(),
        );

        let storage = vec![
            storage_colors(s).into_any_element(),
            setting_row(
                "Notify in the red",
                &format!(
                    "When the startup disk passes {:.0}%",
                    s.settings.storage_thresholds().1 * 100.0
                ),
                ui::toggle("storage-notify", s.settings.storage_notify).on_click(|_, _, cx| {
                    crate::update_settings(cx, |s| s.storage_notify = !s.storage_notify);
                }),
            )
            .into_any_element(),
        ];

        let armed = self.reset_armed.is_some_and(|t| t.elapsed() < CONFIRM_FOR);
        let reset = if armed {
            ui::danger_button("reset", "Reset all settings?").on_click(cx.listener(
                |this, _, _, cx| {
                    this.reset_armed = None;
                    let before = this.stats.read(cx).settings.clone();
                    crate::update_settings(cx, |s| *s = Settings::default());
                    let undo: ui::Callback = Rc::new(move |_, cx| {
                        let before = before.clone();
                        crate::update_settings(cx, move |s| *s = before);
                    });
                    this.show_toast(ui::Tone::Ok, "Settings reset", Some(("Undo", undo)), cx);
                },
            ))
        } else {
            ui::button("reset", "Reset…", false)
                .tooltip(ui::tip("Put every setting back to its default", None))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reset_armed = Some(Instant::now());
                    // Disarm on its own if the second click doesn't come.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(CONFIRM_FOR).await;
                        let _ = this.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                    cx.notify();
                }))
        };

        v_flex()
            .gap(px(20.))
            .children(self.issue_banners(s, cx))
            .child(ui::section("Menu bar", None, group(menu_bar)))
            .child(ui::section("Storage", None, group(storage)))
            .children(self.settings_extra(s, cx))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(reset)
                    .child(
                        ui::button("quit", format!("Quit {}", utilify_core::PRODUCT_NAME), true)
                            .tooltip(ui::tip("Fans go back to automatic", Some("⌘Q")))
                            .on_click(|_, _, cx| cx.quit()),
                    ),
            )
    }

    fn menu_sensor_name(&self, s: &Snapshot) -> String {
        let id = &s.settings.menu_temp_sensor;
        utilify_core::sensors::find(fans::sensor_list_of(s), id)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| {
                if id == utilify_core::sensors::CPU_AVERAGE_ID {
                    "CPU Core Average".into()
                } else {
                    id.clone()
                }
            })
    }

    /// Which sensor the menu bar shows: the current one, unfolding to the
    /// whole list.
    fn temp_sensor_picker(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let open = self.temp_menu_open;
        let current = s.settings.menu_temp_sensor.clone();
        let head = h_flex()
            .id("temp-sensor")
            .h(px(33.))
            .px(px(12.))
            .gap(px(8.))
            .justify_between()
            .rounded(radius(ui::R_MD))
            .border_1()
            .border_color(if open { focus() } else { card_border() })
            .text_size(px(ui::text::SMALL))
            .cursor_pointer()
            .child(self.menu_sensor_name(s))
            .child(ui::icon(
                if open { "chevron-up" } else { "chevron-down" },
                12.,
                fg2(),
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.temp_menu_open = !this.temp_menu_open;
                cx.notify();
            }));
        let list = open.then(|| {
            let sensors = fans::sensor_list_of(s);
            let mut v = v_flex()
                .id("temp-sensor-list")
                .max_h(px(260.))
                .overflow_y_scroll()
                .p(px(4.))
                .rounded(radius(ui::R_MD))
                .border_1()
                .border_color(card_border())
                .bg(popover());
            if sensors.is_empty() {
                v = v.child(ui::ui_text(ui::text::SMALL, fg3()).p(px(6.)).child("Reading sensors…"));
            }
            for sensor in sensors {
                let selected = sensor.id == current;
                let id = sensor.id.clone();
                v = v.child(
                    h_flex()
                        .id(SharedString::from(format!("temp-{}", sensor.id)))
                        .h(px(31.))
                        .px(px(8.))
                        .gap(px(8.))
                        .rounded(radius(ui::R_SM))
                        .text_size(px(ui::text::SMALL))
                        .text_color(if selected { fg() } else { fg2() })
                        .cursor_pointer()
                        .hover(|s| s.bg(raised()).text_color(fg()))
                        .child(div().flex_1().child(sensor.name.clone()))
                        .child(ui::mono(ui::text::CAPTION, fg3()).child(format!("{:.0}°C", sensor.celsius)))
                        .child(
                            div()
                                .w(px(14.))
                                .children(selected.then(|| ui::icon("check", 14., ok()))),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.temp_menu_open = false;
                            let id = id.clone();
                            crate::update_settings(cx, move |s| s.menu_temp_sensor = id);
                        })),
                );
            }
            v
        });
        v_flex().gap(px(8.)).pb(px(14.)).child(head).children(list)
    }

    fn interval_dropdown(&self, settings: &Settings, cx: &mut Context<Self>) -> Div {
        let open = self.interval_menu_open;
        let pill = h_flex()
            .id("interval")
            .h(px(33.))
            .min_w(px(104.))
            .px(px(12.))
            .gap(px(8.))
            .justify_between()
            .rounded(radius(ui::R_MD))
            .border_1()
            .border_color(if open { focus() } else { card_border() })
            .text_size(px(ui::text::SMALL))
            .cursor_pointer()
            .child(crate::tray::interval_label(settings.interval_ms))
            .child(ui::icon(
                if open { "chevron-up" } else { "chevron-down" },
                12.,
                fg2(),
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.interval_menu_open = !this.interval_menu_open;
                cx.notify();
            }));

        let menu = open.then(|| {
            let items = INTERVALS_MS.iter().map(|&ms| {
                let selected = ms == settings.interval_ms;
                // The graph keeps a fixed number of samples, so the interval
                // sets how far back it reaches.
                let reach = Duration::from_millis(ms) * utilify_core::monitor::HISTORY_LEN as u32;
                h_flex()
                    .id(ElementId::Integer(ms))
                    .h(px(33.))
                    .px(px(10.))
                    .gap(px(8.))
                    .rounded(radius(ui::R_SM))
                    .text_size(px(ui::text::SMALL))
                    .text_color(if selected { fg() } else { fg2() })
                    .cursor_pointer()
                    .hover(|s| s.bg(raised()).text_color(fg()))
                    .child(div().flex_1().child(crate::tray::interval_label(ms)))
                    .child(
                        ui::mono(ui::text::CAPTION, fg3())
                            .child(format!("{} graph", format::span(reach.as_secs()))),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .children(selected.then(|| ui::icon("check", 14., ok()))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.interval_menu_open = false;
                        crate::update_settings(cx, |s| s.interval_ms = ms);
                    }))
            });
            deferred(
                v_flex()
                    .absolute()
                    .top(px(40.))
                    .right_0()
                    .w(px(220.))
                    .p(px(6.))
                    .bg(popover())
                    .border_1()
                    .border_color(card_border())
                    .rounded(radius(ui::R_MD))
                    .shadow(vec![BoxShadow {
                        color: shadow(),
                        offset: point(px(0.), px(16.)),
                        blur_radius: px(40.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .occlude()
                    .children(items),
            )
            .with_priority(1)
        });

        div()
            .relative()
            .flex_none()
            .child(pill)
            .children(menu)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.interval_menu_open {
                    this.interval_menu_open = false;
                    cx.notify();
                }
            }))
    }
}

/// Where the storage bar turns yellow and red: a settings row of its own.
fn storage_colors(s: &Snapshot) -> Div {
    const STEP: u8 = 5;
    let (yellow, red) = s.settings.storage_thresholds();
    let (y, r) = ((yellow * 100.0).round() as u8, (red * 100.0).round() as u8);
    let set = |f: fn(&mut Settings, u8), v: u8| {
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            crate::update_settings(cx, |s| f(s, v));
        }
    };
    let set_yellow: fn(&mut Settings, u8) = |s, v| s.storage_warn_pct = v;
    let set_red: fn(&mut Settings, u8) = |s, v| s.storage_danger_pct = v;
    let threshold = |label: &str, stepper: Div| {
        v_flex()
            .flex_1()
            .gap(px(8.))
            .child(ui::ui_text(ui::text::CAPTION, fg3()).child(label.to_string()))
            .child(stepper)
    };
    v_flex()
        .gap(px(12.))
        .py(px(14.))
        .child(
            h_flex()
                .justify_between()
                .items_baseline()
                .child(ui::ui_text(ui::text::BODY, fg()).child("Bar colours"))
                .child(ui::ui_text(ui::text::CAPTION, fg3()).child("% of the disk used")),
        )
        .child(ui::zone_bar(
            yellow,
            red,
            s.disk.map_or(0.0, |d| d.used_fraction()),
        ))
        .child(
            h_flex()
                .gap(px(14.))
                .child(threshold(
                    "Yellow from",
                    ui::stepper(
                        "yellow",
                        format!("{y}%"),
                        (y > STEP).then(|| set(set_yellow, y - STEP)),
                        (y + STEP < r).then(|| set(set_yellow, y + STEP)),
                    ),
                ))
                .child(threshold(
                    "Red from",
                    ui::stepper(
                        "red",
                        format!("{r}%"),
                        (r > y + STEP).then(|| set(set_red, r - STEP)),
                        (r + STEP <= 95).then(|| set(set_red, r + STEP)),
                    ),
                )),
        )
}

fn setting_row(title: &str, detail: &str, control: impl IntoElement) -> Div {
    h_flex()
        .gap(px(16.))
        .py(px(12.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(ui::ui_text(ui::text::BODY, fg()).child(title.to_string()))
                .when(!detail.is_empty(), |v| {
                    v.child(ui::ui_text(ui::text::CAPTION, fg3()).child(detail.to_string()))
                }),
        )
        .child(control)
}

/// Mbps as a short number for the bar values: `412`, `38.6`, `4.21`.
fn mbps(bps: u64) -> String {
    let v = bps as f64 / 1e6;
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// Right side of the speed test header. Folded, it carries the result.
fn speed_test_status(s: &Snapshot, open: bool) -> Div {
    let status: AnyElement = match &s.speed_test {
        SpeedTest::Idle => text("not run yet"),
        SpeedTest::Running { started } => div()
            .text_color(caution())
            .child(format!("testing… {} s", started.elapsed().as_secs()))
            .into_any_element(),
        SpeedTest::Done { finished, .. } => text(format!(
            "{} ago",
            format::span(finished.elapsed().as_secs())
        )),
        SpeedTest::Failed(_) => div()
            .text_color(danger())
            .child("didn't finish")
            .into_any_element(),
    };
    let parts = match &s.speed_test {
        SpeedTest::Done { result, .. } if !open => vec![
            text(format!("↓ {}", mbps(result.down_bps))),
            text(format!("↑ {} Mbps", mbps(result.up_bps))),
            status,
        ],
        // Units only when there are numbers to read them against.
        SpeedTest::Done { .. } => vec![text("Mbps"), status],
        _ => vec![status],
    };
    ui::dotted(parts)
        .text_size(px(ui::text::CAPTION))
        .text_color(fg3())
}

/// What went wrong, in words, from `networkQuality`'s error.
fn speed_test_problem(error: &str, offline: bool) -> (&'static str, String) {
    if offline {
        return (
            "You're offline",
            "The test needs an internet connection. Connect, then try again.".into(),
        );
    }
    if error.contains("macOS 12") {
        return (
            "Speed test needs macOS 12 or later",
            "It uses macOS's own networkQuality tool, which this Mac doesn't have.".into(),
        );
    }
    if error.contains("did not complete") {
        return (
            "The test stopped early",
            "One of the two measurements didn't finish, often because the connection \
             dropped for a moment. Try again."
                .into(),
        );
    }
    ("The speed test didn't finish", format!("{error}. Try again in a moment."))
}

/// The last result next to the link rate, and latency idle vs under load.
fn speed_test_body(s: &Snapshot) -> Div {
    let offline = s.rates.link.is_none();
    match &s.speed_test {
        SpeedTest::Done { result, .. } => {
            let cap = s.rates.link.as_ref().map_or(0, |l| l.capacity_bps);
            let scale = cap.max(result.down_bps).max(result.up_bps).max(1) as f64;
            let mut bars = v_flex().gap(px(11.));
            if cap > 0 {
                bars = bars.child(ui::hbar("Link rate", cap as f64 / scale, bar(), mbps(cap)));
            }
            bars = bars
                .child(ui::hbar(
                    "Download",
                    result.down_bps as f64 / scale,
                    bar_lite(),
                    mbps(result.down_bps),
                ))
                .child(ui::hbar(
                    "Upload",
                    result.up_bps as f64 / scale,
                    bar_lite(),
                    mbps(result.up_bps),
                ));
            let mut body = v_flex().gap(px(14.)).child(bars);
            if let Some(latency) = latency(result) {
                body = body.child(ui::kv("Latency", latency));
            }
            body
        }
        SpeedTest::Failed(e) => {
            let (title, detail) = speed_test_problem(e, offline);
            v_flex().child(ui::banner(
                ui::Tone::Danger,
                title,
                vec![ui::note(detail)],
                vec![],
            ))
        }
        _ if !speed_test_available() => v_flex().child(ui::banner(
            ui::Tone::Info,
            "Speed test needs macOS 12 or later",
            vec![ui::note(
                "It uses macOS's own networkQuality tool, which this Mac doesn't have.",
            )],
            vec![],
        )),
        _ if offline => v_flex().child(ui::banner(
            ui::Tone::Caution,
            "You're offline",
            vec![ui::note("Connect to a network to run a test.")],
            vec![],
        )),
        SpeedTest::Idle | SpeedTest::Running { .. } => v_flex().child(
            ui::ui_text(ui::text::SMALL, fg3())
                .child("Measures against Apple's servers. Takes about 20 seconds."),
        ),
    }
}

/// `18 ms idle · 48 ms loaded · Good`. `networkQuality` reports
/// responsiveness as round trips per minute under load, so 60000 / RPM is the
/// loaded round-trip time in ms.
fn latency(r: &speedtest::SpeedTestResult) -> Option<Div> {
    let parts = latency_parts(r);
    (!parts.is_empty()).then(|| {
        ui::dotted(
            parts
                .into_iter()
                .map(|(t, c)| match c {
                    Some(c) => div().text_color(c).child(t).into_any_element(),
                    None => text(t),
                })
                .collect(),
        )
    })
}

fn latency_parts(r: &speedtest::SpeedTestResult) -> Vec<(String, Option<Hsla>)> {
    let mut parts = vec![];
    if let Some(ms) = r.idle_latency_ms {
        parts.push((format!("{ms:.0} ms idle"), None));
    }
    if let Some(rpm) = r.responsiveness_rpm.filter(|r| *r > 0.0) {
        let loaded = 60_000.0 / rpm;
        parts.push((format!("{loaded:.0} ms loaded"), None));
        let (label, color) = match loaded {
            l if l < 60.0 => ("Good", ok()),
            l if l < 150.0 => ("Fair", caution()),
            _ => ("Poor", danger()),
        };
        parts.push((label.to_string(), Some(color)));
    }
    parts
}

/// A test's results as text, for the clipboard.
/// What the speed test's Copy puts on the clipboard (`utilifyctl` too).
pub(crate) fn speed_test_summary(r: &speedtest::SpeedTestResult) -> String {
    let mut out = format!(
        "Download {} Mbps, upload {} Mbps",
        mbps(r.down_bps),
        mbps(r.up_bps)
    );
    let latency: Vec<String> = latency_parts(r).into_iter().map(|(t, _)| t).collect();
    if !latency.is_empty() {
        out.push_str(&format!(", latency {}", latency.join(", ")));
    }
    if let Some(i) = &r.interface {
        out.push_str(&format!(" ({i})"));
    }
    out
}

/// Whether `networkQuality` is installed. It ships with macOS, so it's
/// looked up once rather than on every frame.
fn speed_test_available() -> bool {
    static AVAILABLE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(speedtest::is_available);
    *AVAILABLE
}

fn speed_test_button(s: &Snapshot, cx: &mut Context<Dashboard>) -> Div {
    let state = &s.speed_test;
    let can_run = s.rates.link.is_some() && speed_test_available();
    let start = |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        stats::stats(cx).update(cx, |s, cx| s.start_speed_test(cx));
    };
    let button = match state {
        SpeedTest::Running { started } => {
            let elapsed = started.elapsed();
            let progress = elapsed.as_secs_f32() / speedtest::MAX_RUNTIME.as_secs_f32();
            ui::button(
                "speed-test",
                format!("Testing · {} s", elapsed.as_secs()),
                false,
            )
            .text_color(fg())
            .cursor_default()
            // The button itself fills as the test runs.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(relative(progress.clamp(0.0, 1.0)))
                    .bg(ok().opacity(0.16)),
            )
        }
        _ if !can_run => ui::button("speed-test", "Run test", false)
            .opacity(0.5)
            .cursor_default(),
        SpeedTest::Failed(_) => ui::button("speed-test", "Try again", true).on_click(start),
        SpeedTest::Done { .. } => ui::button("speed-test", "Run again", true).on_click(start),
        SpeedTest::Idle => ui::button("speed-test", "Run test", true).on_click(start),
    };
    let copy = match state {
        SpeedTest::Done { result, .. } => {
            let summary = speed_test_summary(result);
            Some(
                ui::icon_action("speed-test-copy", "copy", "Copy results", None)
                    .size(px(42.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(summary.clone()));
                        this.show_toast(ui::Tone::Ok, "Results copied", None, cx);
                    })),
            )
        }
        _ => None,
    };
    let running = matches!(state, SpeedTest::Running { .. });
    h_flex()
        .gap(px(10.))
        .child(button)
        .children(copy)
        .when(running, |row| row.child(cancel_button()))
}

/// Square red ✕ beside the running test's button; stops the test.
fn cancel_button() -> Stateful<Div> {
    div()
        .id("speed-test-cancel")
        .flex_none()
        .size(px(42.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius(ui::R_MD))
        .border_1()
        .border_color(danger().opacity(0.45))
        .bg(danger().opacity(0.12))
        .cursor_pointer()
        .hover(|s| s.bg(danger().opacity(0.22)))
        .tooltip(ui::tip("Stop the test", None))
        .child(ui::icon("x", 18., danger()))
        .on_click(|_, _, cx| {
            stats::stats(cx).update(cx, |s, cx| s.cancel_speed_test(cx));
        })
}

/// Used space over the stored history: the line, a dashed straight trend
/// from first to last, and the two end values. Hovering shows the sample
/// nearest the pointer.
fn storage_chart(s: &Snapshot, hover: Option<f32>, view: WeakEntity<Dashboard>) -> Div {
    const PAD_X: f32 = 24.;
    const PAD_TOP: f32 = 30.;
    const PAD_BOTTOM: f32 = 40.;
    let w = ui::CONTENT_W - 2.;
    let h = CHART_H - 2.;
    let frame = div()
        .relative()
        .w_full()
        .h(px(CHART_H))
        .rounded(radius(ui::R_MD))
        .border_1()
        .border_color(control_border())
        .bg(chart())
        .overflow_hidden();

    let Some((first, last)) = s.disk_range() else {
        return frame.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::empty_state("chart-line", "Collecting history", "Used space is recorded every 15 minutes; the chart appears after the second sample.")),
        );
    };

    let pts: Vec<(f32, f32)> =
        disk_history::graph_points(&s.disk_samples, w - 2. * PAD_X, h - PAD_TOP - PAD_BOTTOM)
            .into_iter()
            .map(|(x, y)| (x + PAD_X, y + PAD_TOP))
            .collect();
    let (x0, y0) = pts[0];
    let (x1, y1) = pts[pts.len() - 1];
    let span = last.at - first.at;
    let delta = last.used as f64 - first.used as f64;
    let (per, unit) = if span >= 86_400 {
        (86_400.0, "day")
    } else {
        (3_600.0, "h")
    };
    let rate = format!(
        "avg {}/{unit}",
        format::bytes_delta((delta * per / span as f64) as i64)
    );
    // Sample nearest the pointer, and its point on the line.
    let hovered = hover.and_then(|hx| {
        let i = (0..pts.len())
            .min_by(|&a, &b| (pts[a].0 - hx).abs().total_cmp(&(pts[b].0 - hx).abs()))?;
        Some((s.disk_samples[i], pts[i]))
    });
    let hover_pt = hovered.map(|(_, p)| p);

    let plot = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let x = bounds
                    .contains(&e.position)
                    .then(|| (e.position.x - bounds.origin.x).as_f32() / ui::SCALE);
                let _ = view.update(cx, |d, cx| {
                    if d.storage_hover != x {
                        d.storage_hover = x;
                        cx.notify();
                    }
                });
            });
            let at = |(x, y): (f32, f32)| bounds.origin + point(px(x), px(y));
            let (bw, bh) = (
                bounds.size.width.as_f32() / ui::SCALE,
                bounds.size.height.as_f32() / ui::SCALE,
            );
            // Grid: fine every 13.5 px, coarse every 54.
            let mut x = 0.0;
            let mut i = 0;
            while x < bw {
                let c = if i % 4 == 0 { grid_strong() } else { grid() };
                window.paint_quad(fill(Bounds::new(at((x, 0.)), size(px(1.), px(bh))), c));
                x += 13.5;
                i += 1;
            }
            let (mut y, mut i) = (0.0, 0);
            while y < bh {
                let c = if i % 4 == 0 { grid_strong() } else { grid() };
                window.paint_quad(fill(Bounds::new(at((0., y)), size(px(bw), px(1.))), c));
                y += 13.5;
                i += 1;
            }
            let line = |from: (f32, f32), to: (f32, f32), dash: &[Pixels], width: f32| {
                let mut b = PathBuilder::stroke(px(width)).dash_array(dash);
                b.move_to(at(from));
                b.line_to(at(to));
                b.build().ok()
            };
            if let Some((hx, _)) = hover_pt {
                window.paint_quad(fill(
                    Bounds::new(at((hx - 0.5, 0.)), size(px(1.), px(bh))),
                    hairline(),
                ));
            }
            // Guides at the first and last values.
            if let Some(p) = line((x0, y0), (bw - 10., y0), &[px(2.), px(4.)], 1.) {
                window.paint_path(p, guide());
            }
            if let Some(p) = line((PAD_X + 20., y1), (x1, y1), &[px(2.), px(4.)], 1.) {
                window.paint_path(p, guide());
            }
            if let Some(p) = line((x0, y0), (x1, y1), &[px(4.), px(4.)], 1.3) {
                window.paint_path(p, caution().opacity(0.85));
            }
            let mut b = PathBuilder::stroke(px(2.));
            b.move_to(at(pts[0]));
            for &p in &pts[1..] {
                b.line_to(at(p));
            }
            if let Ok(p) = b.build() {
                window.paint_path(p, ok());
            }
            for (cx, cy) in [(x0, y0), (x1, y1)].into_iter().chain(hover_pt) {
                for (r, c) in [(5.5, ok()), (3.5, chart())] {
                    window.paint_quad(
                        fill(
                            Bounds::new(at((cx - r, cy - r)), size(px(2. * r), px(2. * r))),
                            c,
                        )
                        .corner_radii(px(r)),
                    );
                }
            }
        },
    )
    .absolute()
    .inset_0();

    let label = |s: String| ui::mono(ui::text::MICRO, fg2()).absolute().child(s);
    // While hovering, the tooltip replaces the end labels it would collide with.
    let tooltip = hovered.map(|(sample, (hx, hy))| {
        const TIP_W: f32 = 150.;
        let ago = last.at.saturating_sub(sample.at);
        let when = if ago < 60 {
            "now".to_string()
        } else {
            format!("{} ago", format::span(ago))
        };
        // Above the point, or below it when there's no room.
        let top = if hy - 52. >= 4. { hy - 52. } else { hy + 14. };
        v_flex()
            .absolute()
            .w(px(TIP_W))
            .left(px((hx - TIP_W / 2.).clamp(6., w - TIP_W - 6.)))
            .top(px(top))
            .items_center()
            .py(px(5.))
            .rounded(radius(ui::R_SM))
            .border_1()
            .border_color(raised_border())
            .bg(popover())
            .child(ui::mono(ui::text::CAPTION, fg()).child(format::bytes(sample.used)))
            .child(ui::mono(ui::text::MICRO, fg3()).child(when))
    });
    frame
        .child(plot)
        .when(tooltip.is_none(), |f| {
            f.child(
                label(format::bytes(last.used))
                    .left(px(PAD_X + 20.))
                    .top(px((y1 - 20.).max(4.))),
            )
            .child({
                // Right-aligned over its guide, unless that puts it on the
                // newest point's marker: then it steps left of the marker.
                let top = (y0 - 20.).max(4.);
                let on_marker = y1 + 6. > top && y1 - 6. < top + 16.;
                label(format::bytes(first.used))
                    .right(px(if on_marker { w - x1 + 10. } else { 10. }))
                    .top(px(top))
            })
        })
        .children(tooltip)
        .child(
            h_flex()
                .absolute()
                .left(px(14.))
                .right(px(14.))
                .bottom(px(12.))
                .justify_between()
                .font_family(crate::theme::MONO_FONT)
                .text_size(px(ui::text::MICRO))
                .text_color(fg3())
                .child(format!("{} ago", format::span(span)))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .text_color(fg2())
                        .child(div().text_color(caution()).child("- -"))
                        .child(rate),
                )
                .child("now"),
        )
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Snapshot::read(self.stats.read(cx));
        let body = match self.tab {
            Tab::Overview => self.overview(&s, cx),
            Tab::Network => self.network(&s, cx),
            Tab::Storage => self.storage(&s, cx),
            Tab::Awake => awake::tab(&s, cx),
            Tab::Fans => fans::tab(self, &s, cx),
            Tab::Settings => self.settings(&s, cx),
        };
        let body = div()
            // A fresh id per tab, so each starts scrolled to the top.
            .id(SharedString::from(format!("tab-body-{}", self.tab.label())))
            .max_h(px(self.body_max_h))
            .overflow_y_scroll()
            .child(div().p(px(20.)).child(body));
        let card = ui::card(self.header(&s), self.tabs(&s, cx), body)
            .relative()
            .children(self.toast_view(cx).map(|t| {
                div()
                    .absolute()
                    .left(px(16.))
                    .right(px(16.))
                    .bottom(px(18.))
                    .child(t)
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

        // The window is taller than most tabs; a click on the transparent
        // part below the card closes the dashboard like a click elsewhere.
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(ui::SHADOW))
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|_, _: &Dismiss, window, cx| dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.set_tab(this.tab.next(), cx)))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| this.set_tab(this.tab.prev(), cx)))
            .on_action(cx.listener(|this, _: &ShowOverview, _, cx| this.set_tab(Tab::Overview, cx)))
            .on_action(cx.listener(|this, _: &ShowNetwork, _, cx| this.set_tab(Tab::Network, cx)))
            .on_action(cx.listener(|this, _: &ShowStorage, _, cx| this.set_tab(Tab::Storage, cx)))
            .on_action(cx.listener(|this, _: &ShowAwake, _, cx| this.set_tab(Tab::Awake, cx)))
            .on_action(cx.listener(|this, _: &ShowFans, _, cx| this.set_tab(Tab::Fans, cx)))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.set_tab(Tab::Settings, cx)))
            .on_action(|_: &OpenWorkbench, _, cx| crate::workbench::open(cx))
            .on_action(|_: &QuitApp, _, cx| cx.quit())
            .on_mouse_down(MouseButton::Left, |_, window, cx| dismiss(window, cx))
            .child(card)
    }
}

/// A dot on a tab's icon when something there needs a look.
fn tab_badge(tab: Tab, s: &Snapshot) -> Option<Hsla> {
    match tab {
        Tab::Fans => fans::badge(s),
        Tab::Storage => s.disk.and_then(|d| {
            (s.settings.storage_level(d.used_fraction())
                == utilify_core::settings::StorageLevel::Danger)
                .then(danger)
        }),
        Tab::Settings => status::settings_badge(s),
        _ => None,
    }
}

/// Open/close bookkeeping for the single dashboard window.
#[derive(Default)]
struct DashboardWindow {
    handle: Option<WindowHandle<Dashboard>>,
    /// When it last closed by losing focus. Clicking the status item while
    /// the dashboard is open first deactivates it (closing it) and then
    /// delivers the click, which must not reopen it.
    closed_at: Option<Instant>,
    /// Reopen on the tab that was showing, with the speed test folded the
    /// same way.
    tab: Tab,
    speed_test_open: bool,
    /// The Awake tab's custom length and end time.
    awake: awake::Picks,
}

impl Global for DashboardWindow {}

const REOPEN_GUARD: Duration = Duration::from_millis(300);

fn state(cx: &mut App) -> &mut DashboardWindow {
    if !cx.has_global::<DashboardWindow>() {
        cx.set_global(DashboardWindow::default());
    }
    cx.global_mut::<DashboardWindow>()
}

fn dismiss(window: &mut Window, cx: &mut App) {
    let s = state(cx);
    s.handle = None;
    s.closed_at = Some(Instant::now());
    window.remove_window();
}

/// Open the Network tab's speed test section unfolded (debug launches).
/// Carry out a health check's fix, as its button in Settings does.
pub(crate) fn run_health_fix(fix: &str, cx: &App) {
    status::run_fix(fix, cx);
}

pub fn unfold_speed_test(cx: &mut App) {
    state(cx).speed_test_open = true;
}

/// The tab the dashboard opens on next.
pub fn set_tab(tab: Tab, cx: &mut App) {
    state(cx).tab = tab;
}

/// Open the dashboard on `tab`, switching tabs if it's already open.
pub fn open_on(tab: Tab, cx: &mut App) {
    set_tab(tab, cx);
    if let Some(handle) = state(cx).handle {
        let _ = handle.update(cx, |d, _, cx| d.set_tab(tab, cx));
    }
    open(cx);
}

pub fn is_open(cx: &mut App) -> bool {
    state(cx).handle.is_some()
}

/// Status-item click: open, or close if already open.
pub fn toggle(cx: &mut App) {
    if is_open(cx) {
        close(cx);
        return;
    }
    let recently_closed = state(cx)
        .closed_at
        .is_some_and(|t| t.elapsed() < REOPEN_GUARD);
    if !recently_closed {
        open(cx);
    }
}

pub fn close(cx: &mut App) {
    if let Some(handle) = state(cx).handle.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

pub fn open(cx: &mut App) {
    if let Some(handle) = state(cx).handle {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        return;
    }
    crate::theme::follow_system(cx);
    let placement = crate::placement::below_menu_bar((WIDTH * ui::SCALE) as f64);
    let display_id = placement
        .and_then(|p| p.display_id)
        .map(|id| DisplayId::new(id as u64))
        .filter(|id| cx.find_display(*id).is_some());
    // Fit the screen: the window is never taller than the room under the
    // menu bar, and the tab's content scrolls inside what's left.
    let height = placement
        .map_or(HEIGHT, |p| (p.room as f32 / ui::SCALE).min(HEIGHT));
    let body_max_h = height - CHROME_H;
    let origin = placement
        // Screen coordinates, so not scaled.
        .map(|p| point(gpui_kit::px(p.x as f32), gpui_kit::px(p.y as f32)))
        .unwrap_or_else(|| point(gpui_kit::px(100.), gpui_kit::px(40.)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin,
            size: size(px(WIDTH), px(height)),
        })),
        display_id,
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(utilify_core::APP_IDENTIFIER.into()),
        ..Default::default()
    };
    let stats = stats::stats(cx);
    let mut native = None;
    match cx.open_window(options, |window, cx| {
        native = native_window(window);
        cx.new(|cx| Dashboard::new(stats, body_max_h, window, cx))
    }) {
        Ok(handle) => {
            state(cx).handle = Some(handle);
            cx.activate(true);
            // Restyling makes AppKit call back into GPUI, so do it once GPUI
            // is done updating, from the run loop.
            if let Some(native) = native {
                cx.foreground_executor()
                    .spawn(async move { strip_window_chrome(&native) })
                    .detach();
            }
        }
        Err(e) => log::error!("Failed to open the dashboard: {e}"),
    }
}

#[cfg(target_os = "macos")]
type NativeWindow = objc2::rc::Retained<objc2_app_kit::NSWindow>;
#[cfg(not(target_os = "macos"))]
type NativeWindow = ();

#[cfg(target_os = "macos")]
fn native_window(window: &Window) -> Option<NativeWindow> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: GPUI hands out its live content view, and we're on the main
    // thread inside its window callback.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    view.window()
}

#[cfg(not(target_os = "macos"))]
fn native_window(_: &Window) -> Option<NativeWindow> {
    None
}

/// Make the popover window draw nothing but the card. GPUI's transparent
/// windows keep a near-invisible background and a titled frame, so macOS
/// still draws a rim and a shadow around the whole window rectangle, which
/// is taller than most tabs. Going borderless without the system shadow
/// leaves only what we paint (the card brings its own shadow).
#[cfg(target_os = "macos")]
fn strip_window_chrome(window: &NativeWindow) {
    use objc2_app_kit::NSWindowStyleMask;
    let keep = window.styleMask() & NSWindowStyleMask::NonactivatingPanel;
    window.setStyleMask(NSWindowStyleMask::Borderless | keep);
    window.setHasShadow(false);
}

#[cfg(not(target_os = "macos"))]
fn strip_window_chrome(_: &NativeWindow) {}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's glob export includes its own `test` attribute.
    use super::{Tab, mbps};

    #[test]
    fn tabs_cycle() {
        assert_eq!(Tab::Overview.next(), Tab::Network);
        assert_eq!(Tab::Storage.next(), Tab::Awake);
        assert_eq!(Tab::Awake.next(), Tab::Fans);
        assert_eq!(Tab::parse("fans"), Some(Tab::Fans));
        assert_eq!(Tab::parse("awake"), Some(Tab::Awake));
        assert_eq!(Tab::Settings.next(), Tab::Overview);
        assert_eq!(Tab::Storage.index(), 2);
        assert_eq!(Tab::parse("settings"), Some(Tab::Settings));
        assert_eq!(Tab::parse("nope"), None);
    }

    #[test]
    fn mbps_keeps_three_digits() {
        assert_eq!(mbps(412_300_000), "412");
        assert_eq!(mbps(38_600_000), "38.6");
        assert_eq!(mbps(4_210_000), "4.21");
    }
}
