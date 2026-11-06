//! What needs the user's attention across tabs: problems (a save that
//! failed, a file that had to be set aside, a check that didn't pass) as
//! banners on Overview and Settings, and the Settings tab's Notifications
//! and Health sections.

use super::{Dashboard, Snapshot, Tab, group, setting_row};
use crate::ui::{self, Tone, color::*, px};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use utilify_core::format;
use utilify_core::health::{self, Check, Health};
use utilify_core::settings::Settings;

fn tone(h: Health) -> Tone {
    match h {
        Health::Ok => Tone::Ok,
        Health::Warn => Tone::Caution,
        Health::Fail => Tone::Danger,
    }
}

/// Failing self-checks.
fn failing(s: &Snapshot) -> Vec<&Check> {
    s.health.iter().filter(|c| c.status == Health::Fail).collect()
}

/// A dot on the Settings tab when a check fails or a problem is open.
pub(super) fn settings_badge(s: &Snapshot) -> Option<Hsla> {
    if s.problems.iter().any(|p| p.status == Health::Fail) || !failing(s).is_empty() {
        Some(danger())
    } else if !s.problems.is_empty() {
        Some(caution())
    } else {
        None
    }
}

impl Dashboard {
    /// Problems worth a banner, most serious first. On Overview, failing
    /// checks are summed up in one banner that leads to Settings.
    pub(super) fn issue_banners(&self, s: &Snapshot, cx: &mut Context<Self>) -> Vec<Div> {
        let mut problems = s.problems.clone();
        problems.sort_by_key(|p| std::cmp::Reverse(p.status));
        let mut out: Vec<Div> = problems
            .into_iter()
            .map(|p| {
                let id = p.id;
                let actions = if p.dismissable {
                    vec![
                        ui::banner_button(SharedString::from(format!("dismiss-{id}")), "Dismiss")
                            .on_click(|_, _, cx| {
                                crate::stats::stats(cx).update(cx, |s, cx| {
                                    s.clear_problem(id);
                                    cx.notify();
                                })
                            }),
                    ]
                } else {
                    vec![]
                };
                ui::banner(tone(p.status), p.title, vec![ui::note(p.detail)], actions)
            })
            .collect();
        let failing = failing(s);
        if self.tab == Tab::Overview && !failing.is_empty() {
            let title = match failing.as_slice() {
                [one] => format!("{} needs a look", one.name),
                many => format!("{} checks need a look", many.len()),
            };
            let detail = failing
                .iter()
                .map(|c| format!("{}: {}", c.name, c.detail))
                .collect::<Vec<_>>()
                .join(". ");
            out.push(ui::banner(
                Tone::Danger,
                title,
                vec![ui::note(detail)],
                vec![
                    ui::banner_button("show-health", "Show details").on_click(cx.listener(
                        |this, _, _, cx| this.set_tab(Tab::Settings, cx),
                    )),
                ],
            ));
        }
        out
    }

    /// The Settings tab's Notifications and Health sections.
    pub(super) fn settings_extra(&self, s: &Snapshot, _cx: &mut Context<Self>) -> Vec<Div> {
        vec![notifications(&s.settings), self.health_section(s)]
    }

    fn health_section(&self, s: &Snapshot) -> Div {
        let overall = health::overall(&s.health);
        let worst = s.health.iter().filter(|c| c.status != Health::Ok).count();
        let summary = match (s.health_at, worst) {
            (None, _) => "Not run yet".to_string(),
            (Some(_), 0) => "Everything's working".to_string(),
            (Some(_), 1) => "1 thing needs a look".to_string(),
            (Some(_), n) => format!("{n} things need a look"),
        };
        let when = match s.health_at {
            _ if s.health_running => "checking…".to_string(),
            Some(t) if t.elapsed().as_secs() < 60 => "checked just now".to_string(),
            Some(t) => format!("checked {} ago", format::span(t.elapsed().as_secs())),
            None => "runs shortly after launch, then every 15 min".to_string(),
        };
        let status_icon = if s.health_at.is_some() {
            tone(overall)
        } else {
            Tone::Info
        };
        let run = ui::small_button(
            "run-health",
            if s.health_running { "Checking…" } else { "Check now" },
            !s.health_running,
        )
        .when(!s.health_running, |b| {
            b.on_click(|_, _, cx| crate::stats::stats(cx).update(cx, |s, cx| s.run_health(cx)))
        });
        let head = h_flex()
            .gap(px(12.))
            .py(px(12.))
            .child(ui::icon(status_icon.icon(), 18., status_icon.color()))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(ui::ui_text(ui::text::BODY, fg()).child(summary))
                    .child(ui::ui_text(ui::text::CAPTION, fg3()).child(when)),
            )
            .child(run);
        let mut rows = vec![head.into_any_element()];
        let mut checks: Vec<&Check> = s.health.iter().collect();
        // Problems first, then the rest in their usual order.
        checks.sort_by_key(|c| std::cmp::Reverse(c.status));
        for c in checks {
            let t = tone(c.status);
            let fix = c.fix.map(|fix| {
                ui::banner_button(SharedString::from(format!("fix-{}", c.id)), "Show in Finder")
                    .on_click(move |_, _, cx| run_fix(fix, cx))
            });
            rows.push(
                h_flex()
                    .gap(px(10.))
                    .py(px(9.))
                    .child(ui::icon(t.icon(), 14., t.color()))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(ui::ui_text(ui::text::SMALL, fg()).child(c.name.clone()))
                            .child(ui::ui_text(ui::text::CAPTION, fg3()).child(c.detail.clone())),
                    )
                    .children(fix)
                    .into_any_element(),
            );
        }
        ui::section("Health", None, group(rows))
    }
}

/// Carry out a check's suggested fix.
pub(super) fn run_fix(fix: &str, cx: &App) {
    if fix == health::FIX_REVEAL_DATA_DIR
        && let Some(dir) = utilify_core::settings::default_path().and_then(|p| {
            p.parent().map(|d| d.to_path_buf())
        })
    {
        let _ = std::fs::create_dir_all(&dir);
        cx.open_with_system(&dir);
    }
}

/// One notification switch.
fn notify_row(
    id: &'static str,
    title: &str,
    detail: &str,
    on: bool,
    enabled: bool,
    flip: fn(&mut Settings),
) -> AnyElement {
    let toggle = ui::toggle(id, on && enabled).when(enabled, |t| {
        t.on_click(move |_, _, cx| crate::update_settings(cx, flip))
    });
    setting_row(title, detail, toggle)
        .when(!enabled, |r| r.opacity(0.45))
        .into_any_element()
}

/// Every notification Utilify can post, each with its own switch, under a
/// master switch.
fn notifications(set: &Settings) -> Div {
    let all = set.notify_enabled;
    let low = set.low_battery_pct.clamp(5, 50);
    let set_low = |v: u8| {
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            crate::update_settings(cx, move |s| s.low_battery_pct = v)
        }
    };
    let master = setting_row(
        "Allow notifications",
        "Clicking one opens its tab. Focus modes still apply.",
        ui::toggle("notify-all", all).on_click(|_, _, cx| {
            crate::update_settings(cx, |s| s.notify_enabled = !s.notify_enabled)
        }),
    )
    .into_any_element();

    let power = vec![
        master,
        notify_row(
            "notify-fans-battery",
            "Fans at full speed on battery",
            "After 10 min, then every hour while it lasts",
            set.notify_fans_battery,
            all,
            |s| s.notify_fans_battery = !s.notify_fans_battery,
        ),
        notify_row(
            "notify-low-battery",
            "Low battery while kept awake",
            &format!("At {low}% and again at {}%", set.alert_prefs().critical_battery_pct),
            set.notify_low_battery_awake,
            all,
            |s| s.notify_low_battery_awake = !s.notify_low_battery_awake,
        ),
        h_flex()
            .gap(px(12.))
            .py(px(10.))
            .when(!(all && set.notify_low_battery_awake), |r| r.opacity(0.45))
            .child(
                ui::ui_text(ui::text::SMALL, fg2())
                    .flex_1()
                    .child("Low battery at"),
            )
            .child(div().w(px(170.)).flex().child(ui::stepper_sized(
                "low-battery-pct",
                format!("{low}%"),
                // Stays above the second warning, at 10%.
                (low > 15).then(|| set_low(low - 5)),
                (low < 50).then(|| set_low(low + 5)),
                ui::SMALL_H,
            )))
            .into_any_element(),
        notify_row(
            "notify-lid-battery",
            "Lid sleep on battery",
            "When a session can't stop the Mac sleeping with the lid closed",
            set.notify_lid_battery,
            all,
            |s| s.notify_lid_battery = !s.notify_lid_battery,
        ),
        notify_row(
            "notify-awake-ending",
            "Session ending soon",
            "5 min before a timed session ends, with Add 30 Minutes",
            set.notify_awake_ending,
            all,
            |s| s.notify_awake_ending = !s.notify_awake_ending,
        ),
    ];

    let thermal = vec![
        notify_row(
            "notify-thermal",
            "Thermal pressure",
            "macOS is slowing the Mac down to cool it",
            set.notify_thermal,
            all,
            |s| s.notify_thermal = !s.notify_thermal,
        ),
        notify_row(
            "notify-overheat",
            "Overheating",
            "The safety guard put the fans back on automatic",
            set.notify_fan_overheat,
            all,
            |s| s.notify_fan_overheat = !s.notify_fan_overheat,
        ),
        notify_row(
            "notify-helper",
            "Fan helper lost",
            "Fan control stopped while you had it set",
            set.notify_fan_helper,
            all,
            |s| s.notify_fan_helper = !s.notify_fan_helper,
        ),
        notify_row(
            "notify-foreign",
            "Another app takes the fans",
            "Such as Macs Fan Control",
            set.notify_foreign_fans,
            all,
            |s| s.notify_foreign_fans = !s.notify_foreign_fans,
        ),
    ];

    let other = vec![
        notify_row(
            "notify-offline",
            "Network offline",
            "After a minute without a connection",
            set.notify_offline,
            all,
            |s| s.notify_offline = !s.notify_offline,
        ),
        notify_row(
            "notify-speed-test",
            "Speed test finished",
            "Only when the dashboard is closed",
            set.notify_speed_test,
            all,
            |s| s.notify_speed_test = !s.notify_speed_test,
        ),
    ];

    ui::section(
        "Notifications",
        None,
        v_flex()
            .gap(px(10.))
            .child(group(power))
            .child(group(thermal))
            .child(group(other)),
    )
}
