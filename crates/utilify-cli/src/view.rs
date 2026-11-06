//! What each command shows, as lines of text. The same renderers back the
//! one-shot commands (`utilifyctl network`) and the live view (`utilifyctl
//! watch`), and they mirror the dashboard's tabs.

use crate::chart;
use crate::term::{Color, Paint, pad, pad_left, visible_len};
use utilify_core::format;
use utilify_core::ipc::{AwakeMode, FanView, SensorView, Snapshot, SpeedTestState, Status};
use utilify_core::settings::{FIELDS, FieldKind};

/// What a renderer needs besides the data.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub p: Paint,
    /// Columns to fill.
    pub width: usize,
    /// Advances every frame, for spinners.
    pub tick: u64,
}

impl Ctx {
    /// The content width: the terminal's, within reason.
    fn w(&self) -> usize {
        self.width.clamp(40, 110)
    }
}

/// Indent for everything under a section title.
const IN: &str = "  ";
/// Width of a label column.
const LABEL: usize = 34;

fn title(c: &Ctx, name: &str, aside: &str) -> String {
    let head = c.p.bold(name);
    if aside.is_empty() {
        head
    } else {
        format!("{head}  {}", c.p.dim(aside))
    }
}

/// `label ........ value`, with the label padded.
fn row(c: &Ctx, label: &str, value: impl AsRef<str>) -> String {
    format!("{IN}{}{}", pad(&c.p.dim(label), LABEL), value.as_ref())
}

/// Left and right parts on one line, the right one pushed to `width`.
fn spread(left: &str, right: &str, width: usize) -> String {
    if right.is_empty() {
        return left.to_string();
    }
    let gap = width
        .saturating_sub(visible_len(left) + visible_len(right))
        .max(1);
    format!("{left}{}{right}", " ".repeat(gap))
}

fn on_off(c: &Ctx, on: bool) -> String {
    if on {
        c.p.paint(Color::Ok, "on")
    } else {
        c.p.dim("off")
    }
}

fn status_mark(c: &Ctx, s: Status) -> String {
    match s {
        Status::Ok => c.p.paint(Color::Ok, "✓"),
        Status::Warn => c.p.paint(Color::Warn, "!"),
        Status::Fail => c.p.paint(Color::Danger, "✗"),
    }
}

fn level_color(level: Option<&str>) -> Color {
    match level {
        Some("danger") => Color::Danger,
        Some("warn") => Color::Warn,
        _ => Color::Ok,
    }
}

fn temp_color(celsius: f32) -> Color {
    if celsius >= 90.0 {
        Color::Danger
    } else if celsius >= 75.0 {
        Color::Warn
    } else {
        Color::Ok
    }
}

/// The dashboard header's subtitle: link, free space, battery.
fn header(c: &Ctx, s: &Snapshot) -> String {
    let mut parts = vec![];
    match &s.network.link {
        Some(l) => parts.push(l.name.clone()),
        None => parts.push(c.p.paint(Color::Danger, "offline")),
    }
    if let Some(d) = s.storage.disk {
        parts.push(format!("{} free", format::bytes(d.available)));
    }
    parts.push(power_text(c, s));
    let low = s.power.on_battery
        && s.power
            .percent
            .is_some_and(|p| p <= s.settings.low_battery_pct.clamp(5, 50));
    let left = format!(
        "{} {}  {}",
        c.p.bold(utilify_core::PRODUCT_NAME),
        c.p.dim(&s.app_version),
        parts.join(&c.p.dim(" · "))
    );
    let right = if low {
        c.p.paint(Color::Warn, "low battery")
    } else {
        String::new()
    };
    spread(&left, &right, c.w())
}

fn power_text(c: &Ctx, s: &Snapshot) -> String {
    let p = s.power;
    if !p.has_battery {
        return "AC power".into();
    }
    let pct = p.percent.map(|v| format!(" {v}%")).unwrap_or_default();
    let mut t = if p.on_battery {
        format!("battery{pct}")
    } else {
        format!("AC{pct}")
    };
    if p.charging {
        t.push_str(" charging");
    }
    if p.low_power_mode {
        t.push_str(&c.p.dim(" low power"));
    }
    t
}

/// Rate rows: `↓ 12.3 MB/s ▁▂▃▅▇` for each direction, scaled together, as
/// the dashboard's paired bars are.
fn rate_lines(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let n = &s.network;
    let label_w = 14;
    let chart_w = c.w().saturating_sub(IN.len() + label_w + 1).max(10);
    let down: Vec<f64> = n.history.iter().map(|p| p[0]).collect();
    let up: Vec<f64> = n.history.iter().map(|p| p[1]).collect();
    let peak = down
        .iter()
        .chain(&up)
        .copied()
        .fold(0.0, f64::max)
        .max(1000.0);
    let line = |arrow: &str, rate: f64, values: &[f64], color: Color| {
        format!(
            "{IN}{} {} {}",
            c.p.paint(color, arrow),
            pad_left(&format::rate(rate), label_w - 2),
            c.p.paint(
                color,
                chart::sparkline(&chart::resample(values, n.history_capacity, chart_w), peak)
            )
        )
    };
    vec![
        line("↓", n.down, &down, Color::Down),
        line("↑", n.up, &up, Color::Up),
        format!(
            "{IN}{}",
            c.p.dim(format!(
                "peak {} · last {}",
                format::rate(peak.max(0.0)),
                format::span(graph_span_secs(s))
            ))
        ),
    ]
}

fn graph_span_secs(s: &Snapshot) -> u64 {
    (s.settings.interval().as_secs_f64() * s.network.history_capacity as f64) as u64
}

fn storage_line(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let Some(d) = s.storage.disk else {
        return vec![format!(
            "{IN}{}",
            c.p.dim("Can't read the startup disk yet; Utilify keeps trying")
        )];
    };
    let used = d.used as f64 / d.total.max(1) as f64;
    let bar_w = c.w().saturating_sub(IN.len() + 8).clamp(10, 60);
    let color = level_color(s.storage.level.as_deref());
    let change = match s.storage.change {
        Some((delta, secs)) => format!("{} in {}", format::bytes_delta(delta), format::span(secs)),
        None => "collecting history…".into(),
    };
    vec![
        format!(
            "{IN}{} {}",
            chart::bar(&c.p, used, bar_w, color),
            c.p.paint(color, format::percent(used))
        ),
        format!(
            "{IN}{} of {} used · {} free · {}",
            format::bytes(d.used),
            format::bytes(d.total),
            format::bytes(d.available),
            c.p.dim(change)
        ),
    ]
}

fn awake_line(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let a = &s.awake;
    let total =
        c.p.dim(format!("kept awake {}", format::span_long(a.total_secs)));
    let Some(sess) = &a.session else {
        return vec![format!("{IN}{} {}  {total}", c.p.dim("○"), c.p.dim("off"))];
    };
    let mut lines = vec![format!(
        "{IN}{} {}  {}  {total}",
        c.p.paint(Color::Ok, "●"),
        c.p.paint(Color::Ok, &sess.status),
        c.p.dim(format!(
            "{} · {}",
            sess.activation.to_lowercase(),
            format::span(sess.elapsed_secs)
        ))
    )];
    if let Some(left) = sess.remaining_secs {
        let whole = (sess.elapsed_secs + left).max(1) as f64;
        let bar_w = c.w().saturating_sub(IN.len() + 8).clamp(10, 60);
        lines.push(format!(
            "{IN}{} {}",
            chart::bar(&c.p, sess.elapsed_secs as f64 / whole, bar_w, Color::Ok),
            c.p.dim(format::countdown(left))
        ));
    }
    lines
}

fn fans_line(c: &Ctx, s: &Snapshot) -> String {
    let f = &s.fans;
    let mut parts = vec![f.active_label.clone()];
    match f.fans.len() {
        0 if f.read_ago_secs.is_some() => parts.push("no fans".into()),
        0 => parts.push("reading…".into()),
        n => {
            parts.push(format!("{n} fan{}", if n == 1 { "" } else { "s" }));
            if let Some(top) = f.fans.iter().map(|x| x.rpm).max_by(|a, b| a.total_cmp(b)) {
                parts.push(format!("{top:.0} rpm fastest"));
            }
        }
    }
    if let Some(t) = f.cpu_average {
        parts.push(c.p.paint(temp_color(t), format!("CPU {t:.0}°C")));
    }
    let mut line = format!("{IN}{}", parts.join(&c.p.dim(" · ")));
    if f.foreign {
        line.push_str(&c.p.paint(Color::Warn, "  another app has the fans"));
    }
    if f.overheat.is_some() {
        line.push_str(&c.p.paint(Color::Danger, "  overheating guard on"));
    }
    line
}

fn problem_lines(c: &Ctx, s: &Snapshot) -> Vec<String> {
    s.problems
        .iter()
        .map(|p| {
            format!(
                "{IN}{} {}  {}",
                status_mark(c, p.status),
                p.title,
                c.p.dim(&p.detail)
            )
        })
        .collect()
}

/// The Overview tab.
pub fn overview(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let mut v = vec![header(c, s), String::new()];
    let live = match &s.network.link {
        Some(_) => c.p.paint(Color::Ok, "live"),
        None => c.p.paint(Color::Danger, "offline"),
    };
    v.push(spread(&title(c, "Network", ""), &live, c.w()));
    v.extend(rate_lines(c, s));
    v.push(String::new());
    v.push(title(c, "Storage", "startup disk"));
    v.extend(storage_line(c, s));
    v.push(String::new());
    v.push(title(c, "Keep awake", ""));
    v.extend(awake_line(c, s));
    v.push(String::new());
    v.push(title(c, "Fans", ""));
    v.push(fans_line(c, s));
    if !s.problems.is_empty() {
        v.push(String::new());
        v.push(title(c, "Problems", ""));
        v.extend(problem_lines(c, s));
    }
    v
}

/// The Network tab: a tall chart each way, the link, the session, and the
/// speed test.
pub fn network(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let n = &s.network;
    let mut aside = vec![];
    match &n.link {
        Some(l) => {
            aside.push(l.name.clone());
            if l.capacity_bps > 0 {
                aside.push(format::bits_per_sec(l.capacity_bps));
            }
        }
        None => aside.push("offline".into()),
    }
    let mut v = vec![title(c, "Network", &aside.join(" · ")), String::new()];
    let chart_w = c.w().saturating_sub(IN.len()).max(10);
    let span = format::span(graph_span_secs(s));
    for (name, i, rate, color, height) in [
        ("Download", 0, n.down, Color::Down, 6),
        ("Upload", 1, n.up, Color::Up, 3),
    ] {
        let values: Vec<f64> = n.history.iter().map(|p| p[i]).collect();
        let peak = values.iter().copied().fold(0.0, f64::max);
        v.push(spread(
            &format!(
                "{IN}{} {}",
                c.p.paint(color, if i == 0 { "↓" } else { "↑" }),
                c.p.bold(format!("{name} {}", format::rate(rate)))
            ),
            &c.p.dim(format!("peak {}", format::rate(peak))),
            c.w(),
        ));
        let cols = chart::resample(&values, n.history_capacity, chart_w);
        for line in chart::columns(&cols, peak.max(1000.0), height) {
            v.push(format!("{IN}{}", c.p.paint(color, line)));
        }
        v.push(format!(
            "{IN}{}",
            spread(&c.p.dim(format!("{span} ago")), &c.p.dim("now"), chart_w)
        ));
        v.push(String::new());
    }
    if let Some(u) = n.utilization {
        v.push(row(
            c,
            "Link in use",
            format!(
                "{} {}",
                chart::bar(&c.p, u, 20, Color::Accent),
                format::percent(u)
            ),
        ));
    }
    v.push(row(
        c,
        "Since Utilify started",
        format!(
            "{} {}  {} {}",
            c.p.paint(Color::Down, "↓"),
            format::bytes(n.session_down),
            c.p.paint(Color::Up, "↑"),
            format::bytes(n.session_up)
        ),
    ));
    v.push(String::new());
    v.extend(speed_test(c, s));
    v
}

/// The speed test's state, as the Network tab shows it.
pub fn speed_test(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let mut v = vec![title(
        c,
        "Speed test",
        "networkQuality, against Apple's servers",
    )];
    match &s.speed_test {
        SpeedTestState::Idle => v.push(format!(
            "{IN}{}",
            c.p.dim("Not run yet. Start one with `utilifyctl speedtest`.")
        )),
        SpeedTestState::Running { elapsed_secs } => {
            // networkQuality runs for about 20 s.
            let f = (elapsed_secs / 20.0).min(0.97);
            v.push(format!(
                "{IN}{} Testing… {}  {}",
                c.p.paint(Color::Accent, chart::spinner(c.tick).to_string()),
                chart::bar(&c.p, f, 24, Color::Accent),
                c.p.dim(format!("{:.0} s", elapsed_secs))
            ));
            v.push(format!(
                "{IN}{} {}  {} {}",
                c.p.paint(Color::Down, "↓"),
                format::rate(s.network.down),
                c.p.paint(Color::Up, "↑"),
                format::rate(s.network.up)
            ));
        }
        SpeedTestState::Done {
            down_bps,
            up_bps,
            idle_latency_ms,
            responsiveness_rpm,
            interface,
            ago_secs,
            ..
        } => {
            v.push(format!(
                "{IN}{} {}   {} {}",
                c.p.paint(Color::Down, "↓"),
                c.p.bold(format::bits_per_sec(*down_bps)),
                c.p.paint(Color::Up, "↑"),
                c.p.bold(format::bits_per_sec(*up_bps))
            ));
            let mut parts = vec![];
            if let Some(ms) = idle_latency_ms {
                parts.push(format!("{ms:.0} ms idle latency"));
            }
            if let Some(rpm) = responsiveness_rpm {
                parts.push(format!("{rpm:.0} RPM responsiveness"));
            }
            if let Some(i) = interface {
                parts.push(format!("over {i}"));
            }
            parts.push(format!("{} ago", format::span(*ago_secs)));
            v.push(format!("{IN}{}", c.p.dim(parts.join(" · "))));
        }
        SpeedTestState::Failed { error } => {
            v.push(format!("{IN}{} {}", c.p.paint(Color::Danger, "✗"), error))
        }
    }
    v
}

/// The Storage tab: the bar, the colour thresholds and 30 days of history.
pub fn storage(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let mut v = vec![title(c, "Storage", "startup disk"), String::new()];
    v.extend(storage_line(c, s));
    v.push(String::new());
    let set = &s.settings;
    v.push(row(
        c,
        "Bar colours",
        format!(
            "{} from {}%  {} from {}%",
            c.p.paint(Color::Warn, "yellow"),
            set.storage_warn_pct,
            c.p.paint(Color::Danger, "red"),
            set.storage_danger_pct
        ),
    ));
    v.push(row(c, "Notify in the red", on_off(c, set.storage_notify)));
    v.push(String::new());
    v.push(title(
        c,
        "History",
        "used space, a sample every 15 min, kept 30 days",
    ));
    let h = &s.storage.history;
    if h.len() < 2 {
        v.push(format!("{IN}{}", c.p.dim("Collecting history…")));
        return v;
    }
    let (t0, t1) = (h[0].at, h[h.len() - 1].at);
    let label_w = 9;
    let chart_w = c.w().saturating_sub(IN.len() + label_w + 1).max(10);
    // One column per time slot, holding the slot's latest sample; slots
    // when Utilify wasn't running stay empty.
    let mut cols: Vec<Option<u64>> = vec![None; chart_w];
    let span = (t1 - t0).max(1);
    for sample in h {
        let i = (((sample.at - t0) as u128 * (chart_w as u128 - 1)) / span as u128) as usize;
        cols[i.min(chart_w - 1)] = Some(sample.used);
    }
    let lo = h.iter().map(|x| x.used).min().unwrap_or(0);
    let hi = h.iter().map(|x| x.used).max().unwrap_or(0);
    // Scale between the lowest and highest use, with a floor, so changes
    // of a few GB on a large disk still show.
    let floor = lo.saturating_sub((hi - lo) / 4 + 1);
    let values: Vec<Option<f64>> = cols.iter().map(|c| c.map(|u| (u - floor) as f64)).collect();
    let rows = chart::columns(&values, (hi - floor).max(1) as f64, 5);
    let color = level_color(s.storage.level.as_deref());
    let last = rows.len() - 1;
    for (i, line) in rows.into_iter().enumerate() {
        // The low label only when it reads differently from the high one.
        let label = match i {
            0 => format::bytes(hi),
            _ if i == last && format::bytes(lo) != format::bytes(hi) => format::bytes(lo),
            _ => String::new(),
        };
        v.push(format!(
            "{IN}{} {}",
            pad_left(&c.p.dim(label), label_w),
            c.p.paint(color, line)
        ));
    }
    v.push(format!(
        "{IN}{} {}",
        " ".repeat(label_w),
        spread(
            &c.p.dim(format!("{} ago", format::span(span))),
            &c.p.dim("now"),
            chart_w
        )
    ));
    v
}

/// The Awake tab: the session, its options and the quick settings.
pub fn awake(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let set = &s.settings;
    let mut v = vec![title(c, "Keep awake", ""), String::new()];
    v.extend(awake_line(c, s));
    if let Some(sess) = &s.awake.session {
        v.push(row(
            c,
            "Mode",
            format!("{}  {}", sess.label, c.p.dim(&sess.activation)),
        ));
        if let AwakeMode::WhileApp { pid } = sess.mode {
            v.push(row(
                c,
                "Following",
                format!(
                    "{} (pid {pid})",
                    sess.subject.as_deref().unwrap_or("an app")
                ),
            ));
        }
    }
    v.push(String::new());
    v.push(title(c, "Session options", ""));
    v.push(row(
        c,
        "Allow display sleep",
        on_off(c, set.awake_allow_display_sleep),
    ));
    let lid_note = if s.power.on_battery && !set.awake_allow_closed_display_sleep {
        c.p.paint(
            Color::Warn,
            "  on battery now: macOS sleeps on lid close until you plug in",
        )
    } else if !set.awake_allow_closed_display_sleep {
        c.p.dim("  only blocks lid sleep on AC power")
    } else {
        String::new()
    };
    v.push(row(
        c,
        "Allow sleep with the lid closed",
        format!(
            "{}{lid_note}",
            on_off(c, set.awake_allow_closed_display_sleep)
        ),
    ));
    let saver = if set.awake_allow_display_sleep {
        c.p.dim("n/a while the display may sleep")
    } else {
        format!(
            "{}  {}",
            on_off(c, set.awake_screen_saver),
            c.p.dim(format!(
                "after {}",
                format::span_long(set.awake_screen_saver_after().as_secs())
            ))
        )
    };
    v.push(row(c, "Allow the screen saver", saver));
    v.push(String::new());
    v.push(title(c, "Quick settings", ""));
    v.push(row(
        c,
        "Start a session at launch",
        on_off(c, set.awake_on_launch),
    ));
    v.push(row(
        c,
        "Notify when a session ends",
        on_off(c, set.awake_notify_end),
    ));
    v.push(row(
        c,
        "Launch Utilify at login",
        match s.launch_at_login {
            Some(on) => on_off(c, on),
            None => c.p.dim("needs macOS 13 and the bundled app"),
        },
    ));
    v
}

/// The Fans tab: banners, presets, each fan, and the hottest sensors.
pub fn fans(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let f = &s.fans;
    let mut aside = vec![match f.fans.len() {
        0 if f.read_ago_secs.is_some() => "no fans".to_string(),
        0 => "reading…".to_string(),
        1 => "1 fan".to_string(),
        n => format!("{n} fans"),
    }];
    aside.push(f.active_label.to_lowercase());
    let chip = f
        .cpu_average
        .map(|t| c.p.paint(temp_color(t), format!("CPU {t:.0}°C")))
        .unwrap_or_default();
    let mut v = vec![
        spread(&title(c, "Fans", &aside.join(" · ")), &chip, c.w()),
        String::new(),
    ];
    let mut banner = |color: Color, text: String| {
        v.push(format!("{IN}{} {text}", c.p.paint(color, "▌")));
    };
    if !f.ready {
        banner(
            if f.helper == "unsupported" || f.helper == "unknown" {
                Color::Accent
            } else {
                Color::Warn
            },
            format!(
                "{}. Speeds and temperatures still show.",
                capitalize(&f.helper_detail)
            ),
        );
    }
    if f.foreign {
        banner(
            Color::Warn,
            "Another app is holding the fans, such as Macs Fan Control".into(),
        );
    }
    if let Some(o) = &f.overheat {
        banner(
            Color::Danger,
            format!(
                "Overheating guard: {} at {:.0}°C, fans back on automatic",
                o.sensor, o.celsius
            ),
        );
    }
    if let Some(e) = &f.error {
        banner(Color::Danger, e.clone());
    }
    if f.taking_over {
        banner(
            Color::Accent,
            "Waiting for macOS to hand the fans over…".into(),
        );
    }
    if v.len() > 2 {
        v.push(String::new());
    }

    let mut presets: Vec<(String, bool)> = vec![
        (
            "Automatic".into(),
            f.active == utilify_core::fans::Active::Automatic,
        ),
        (
            "Full blast".into(),
            f.active == utilify_core::fans::Active::FullBlast,
        ),
    ];
    if f.active == utilify_core::fans::Active::Custom {
        presets.push(("Custom".into(), true));
    }
    for p in &f.presets {
        presets.push((
            p.name.clone(),
            f.active == utilify_core::fans::Active::Preset(p.name.clone()),
        ));
    }
    let list: Vec<String> = presets
        .iter()
        .map(|(name, on)| {
            if *on {
                format!("{} {}", c.p.paint(Color::Ok, "●"), c.p.bold(name))
            } else {
                format!("{} {name}", c.p.dim("○"))
            }
        })
        .collect();
    v.push(title(c, "Presets", ""));
    v.push(format!("{IN}{}", list.join("   ")));
    v.push(String::new());

    if !f.fans.is_empty() {
        v.push(title(c, "Fans", ""));
        let name_w = f
            .fans
            .iter()
            .map(|x| x.name.len())
            .max()
            .unwrap_or(4)
            .max(4);
        let bar_w = c
            .w()
            .saturating_sub(IN.len() + 4 + name_w + 10 + 40)
            .clamp(8, 30);
        for fan in &f.fans {
            v.push(fan_row(c, fan, name_w, bar_w));
            if let Some(issue) = &fan.issue {
                v.push(format!("{IN}    {}", c.p.paint(Color::Warn, issue)));
            }
        }
        v.push(String::new());
    }

    let mut hottest: Vec<&SensorView> = f.sensors.iter().collect();
    hottest.sort_by(|a, b| b.celsius.total_cmp(&a.celsius));
    v.push(title(
        c,
        "Hottest sensors",
        "`utilifyctl sensors` lists them all",
    ));
    if hottest.is_empty() {
        v.push(format!("{IN}{}", c.p.dim("Reading sensors…")));
    }
    hottest.truncate(5);
    let name_w = sensor_name_w(&hottest);
    for sensor in hottest {
        v.push(sensor_row(c, sensor, name_w));
    }
    v
}

fn fan_row(c: &Ctx, fan: &FanView, name_w: usize, bar_w: usize) -> String {
    let color = if fan.control.is_auto() {
        Color::Accent
    } else {
        Color::Ok
    };
    let target = match fan.target {
        Some(t) => c.p.paint(Color::Ok, format!("→ {t:.0}")),
        None if fan.manual => c.p.paint(Color::Warn, "held by another app"),
        None => String::new(),
    };
    let mut line = format!(
        "{IN}{} {} {} {} {}  {}",
        c.p.dim(fan.index.to_string()),
        pad(&fan.name, name_w),
        pad_left(&format!("{:.0} rpm", fan.rpm), 9),
        chart::range_bar(&c.p, fan.rpm, fan.min, fan.max, bar_w, color),
        c.p.dim(format!("{:.0}–{:.0}", fan.min, fan.max)),
        fan.control_label,
    );
    if !target.is_empty() {
        line.push_str("  ");
        line.push_str(&target);
    }
    line
}

/// Name column for a list of sensors.
fn sensor_name_w(list: &[&SensorView]) -> usize {
    list.iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(16, 40)
}

fn sensor_row(c: &Ctx, s: &SensorView, name_w: usize) -> String {
    format!(
        "{IN}{} {} {}  {}",
        pad(&s.name, name_w),
        pad_left(
            &c.p.paint(temp_color(s.celsius), format!("{:.1}°C", s.celsius)),
            8
        ),
        chart::range_bar(&c.p, s.celsius, 20.0, 110.0, 16, temp_color(s.celsius)),
        c.p.dim(&s.id)
    )
}

/// Every sensor, by group.
pub fn sensors(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let mut v = vec![title(
        c,
        "Sensors",
        &format!("{} readings", s.fans.sensors.len()),
    )];
    if s.fans.sensors.is_empty() {
        v.push(format!(
            "{IN}{}",
            c.p.dim("No temperature sensors were read on this Mac.")
        ));
        return v;
    }
    let mut group = "";
    let name_w = sensor_name_w(&s.fans.sensors.iter().collect::<Vec<_>>());
    for sensor in &s.fans.sensors {
        if sensor.group != group {
            group = &sensor.group;
            v.push(String::new());
            v.push(c.p.bold(group));
        }
        v.push(sensor_row(c, sensor, name_w));
    }
    v
}

/// Settings, by the dashboard's sections.
pub fn settings(
    c: &Ctx,
    s: &utilify_core::settings::Settings,
    sensors: &[SensorView],
) -> Vec<String> {
    let mut v = vec![];
    let mut section = "";
    let label_w = FIELDS.iter().map(|f| f.label.len()).max().unwrap_or(20) + 2;
    for f in FIELDS {
        if f.section != section {
            if !section.is_empty() {
                v.push(String::new());
            }
            section = f.section;
            v.push(c.p.bold(section));
        }
        let raw = s.get_field(f.key).unwrap_or_default();
        let shown = match f.kind {
            FieldKind::Bool => on_off(c, raw == "on"),
            FieldKind::Interval => format::interval(s.interval_ms),
            FieldKind::Percent { .. } => format!("{raw}%"),
            FieldKind::Minutes { .. } => format!("{raw} min"),
            FieldKind::Sensor => utilify_core::sensors::CPU_AVERAGE_ID
                .eq(raw.as_str())
                .then(|| "CPU Core Average".to_string())
                .or_else(|| sensors.iter().find(|x| x.id == raw).map(|x| x.name.clone()))
                .unwrap_or(raw.clone()),
        };
        v.push(format!(
            "{IN}{}{}  {}",
            pad(f.label, label_w),
            pad(&shown, 18),
            c.p.dim(f.key)
        ));
    }
    v
}

/// The Health section and any problems.
pub fn health(c: &Ctx, s: &Snapshot) -> Vec<String> {
    let h = &s.health;
    let when = if h.running {
        format!("{} running…", chart::spinner(c.tick))
    } else {
        match h.ago_secs {
            Some(a) => format!("checked {} ago, every 15 min", format::span(a)),
            None => "not run yet".into(),
        }
    };
    let worst = h
        .checks
        .iter()
        .map(|x| x.status)
        .max()
        .unwrap_or(Status::Ok);
    let summary = match (h.checks.is_empty(), worst) {
        (true, _) => "No results yet".to_string(),
        (_, Status::Ok) => "Everything checks out".into(),
        (_, Status::Warn) => "Some things need a look".into(),
        (_, Status::Fail) => "Something isn't working".into(),
    };
    let mut v = vec![title(c, "Health", &when), String::new()];
    v.push(format!("{IN}{} {summary}", status_mark(c, worst)));
    let mut checks: Vec<_> = h.checks.iter().collect();
    checks.sort_by_key(|x| std::cmp::Reverse(x.status));
    for check in checks {
        v.push(format!(
            "{IN}{} {}  {}",
            status_mark(c, check.status),
            pad(&check.name, 26),
            c.p.dim(&check.detail)
        ));
        if check.fix.is_some() && check.status != Status::Ok {
            v.push(format!(
                "{IN}    {}",
                c.p.dim(format!("fix it with `utilifyctl health fix {}`", check.id))
            ));
        }
    }
    if !s.problems.is_empty() {
        v.push(String::new());
        v.push(title(
            c,
            "Problems",
            "`utilifyctl problems dismiss <id>` closes one",
        ));
        for p in &s.problems {
            v.push(format!(
                "{IN}{} {}  {}",
                status_mark(c, p.status),
                p.title,
                c.p.dim(format!("[{}]", p.id))
            ));
            v.push(format!("{IN}  {}", c.p.dim(&p.detail)));
        }
    }
    v
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
