//! The app's end of `utilifyctl`: a socket server ([`utilify_core::ipc`])
//! whose requests run on the main thread through the same functions as the
//! menu and the dashboard (`crate::update_settings`,
//! `crate::start_keep_awake`, ...), so the tray, the dashboard and the
//! workbench pick up every change the CLI makes straight away.

use crate::stats::{self, SpeedTest, Stats};
use crate::{dashboard, system, workbench};
use gpui_kit::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use utilify_core::awake::{Activation, Mode};
use utilify_core::fans::controller::HelperStatus;
use utilify_core::fans::{self, Active, Control};
use utilify_core::ipc::{self, AwakeMode, Request, Response};
use utilify_core::settings::Settings;
use utilify_core::{disk_history, sensors, speedtest};

/// How long a request may wait for the main thread.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);
/// A CLI asking for sensors keeps them read for this long after.
pub const SENSORS_FOR: Duration = Duration::from_secs(10);
/// The temperatures a fan curve may span, as on the Fans tab.
const CURVE_C: std::ops::RangeInclusive<f32> = 20.0..=110.0;
/// The longest session the CLI starts, as on the Awake tab.
const FOR_MAX_MINS: u32 = 24 * 60;

/// Whether this copy owns the socket (and so removes it on quit).
static SERVING: AtomicBool = AtomicBool::new(false);

/// A request on its way to the main thread. Whoever sets `claimed` first
/// decides its fate: the main thread (it runs, and the answer is awaited
/// however long it takes) or the connection's thread giving up after
/// [`ANSWER_WITHIN`] (it never runs). So a request the CLI was told timed
/// out can't take effect later.
struct Job {
    request: Request,
    reply: mpsc::Sender<Response>,
    claimed: Arc<AtomicBool>,
}

fn claim(flag: &AtomicBool) -> bool {
    flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// Start answering on the socket.
pub fn start(cx: &mut App) {
    let Some(path) = ipc::socket_path() else {
        log::warn!("No data folder, so no CLI socket");
        return;
    };
    let listener = match ipc::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("Not serving the CLI at {}: {e}", path.display());
            return;
        }
    };
    SERVING.store(true, Ordering::Relaxed);
    log::info!("Serving the CLI at {}", path.display());
    let (tx, rx) = smol::channel::unbounded::<Job>();
    ipc::serve(listener, move |request| {
        let (reply, answer) = mpsc::channel();
        let claimed = Arc::new(AtomicBool::new(false));
        let job = Job {
            request,
            reply,
            claimed: claimed.clone(),
        };
        if tx.send_blocking(job).is_err() {
            return Response::error("Utilify is quitting");
        }
        match answer.recv_timeout(ANSWER_WITHIN) {
            Ok(r) => r,
            Err(_) if claim(&claimed) => {
                Response::error("Utilify didn't answer in time; nothing was changed")
            }
            // The main thread took it just now: its answer is on the way.
            Err(_) => answer
                .recv()
                .unwrap_or_else(|_| Response::error("Utilify is quitting")),
        }
    });
    cx.spawn(async move |cx| {
        while let Ok(job) = rx.recv().await {
            if !claim(&job.claimed) {
                log::warn!("Dropped a CLI request that waited too long");
                continue;
            }
            let response = cx.update(|cx| handle(job.request, cx));
            let _ = job.reply.send(response);
        }
    })
    .detach();
}

/// On quit: take the socket away so the CLI says Utilify isn't running.
pub fn stop() {
    if SERVING.swap(false, Ordering::Relaxed)
        && let Some(path) = ipc::socket_path()
    {
        let _ = std::fs::remove_file(path);
    }
}

fn handle(request: Request, cx: &mut App) -> Response {
    match request {
        Request::Snapshot {
            disk_history,
            sensors,
        } => {
            if sensors {
                stats::stats(cx).update(cx, |s, _| {
                    s.cli_sensors_at = Some(Instant::now());
                    s.want_fan_sensors(true);
                });
            }
            Response::Snapshot {
                snapshot: Box::new(snapshot(disk_history, cx)),
            }
        }
        Request::SetSetting { key, value } => set_setting(&key, &value, cx),
        Request::ResetSettings => {
            crate::update_settings(cx, |s| *s = Settings::default());
            Response::done("Settings reset to their defaults")
        }
        Request::AwakeStart { mode } => awake_start(mode, cx),
        Request::AwakeStop => {
            if !stats::stats(cx).read(cx).keep_awake.is_active() {
                return Response::done("No session was running");
            }
            crate::stop_keep_awake(cx);
            Response::done("Session ended; the Mac can sleep normally again")
        }
        Request::AwakeExtend { minutes } => awake_extend(minutes, cx),
        Request::RunningApps => Response::Apps {
            apps: system::running_apps()
                .into_iter()
                .map(|a| ipc::RunningApp {
                    pid: a.pid,
                    name: a.name,
                })
                .collect(),
        },
        Request::FanActivate { active } => fan_activate(active, cx),
        Request::FanSetControl { fan, control } => fan_set_control(fan, control, cx),
        Request::FanSavePreset { name } => fan_presets(cx, |f, fans| {
            if f.save_preset(fans, &name) {
                Ok(format!(
                    "Saved the current fan settings as \"{}\"",
                    name.trim()
                ))
            } else {
                Err(preset_name_problem(&name))
            }
        }),
        Request::FanRenamePreset { from, to } => fan_presets(cx, |f, _| {
            if f.preset(&from).is_none() {
                Err(format!("No preset called \"{from}\""))
            } else if f.rename_preset(&from, &to) {
                Ok(format!("Renamed \"{from}\" to \"{}\"", to.trim()))
            } else {
                Err(preset_name_problem(&to))
            }
        }),
        Request::FanDeletePreset { name } => fan_presets(cx, |f, _| {
            if f.preset(&name).is_none() {
                return Err(format!("No preset called \"{name}\""));
            }
            f.delete_preset(&name);
            Ok(format!("Deleted \"{name}\""))
        }),
        Request::SpeedTestStart => {
            if !speedtest::is_available() {
                return Response::error("The speed test needs macOS 12 or later");
            }
            let stats = stats::stats(cx);
            let message = if matches!(stats.read(cx).speed_test, SpeedTest::Running { .. }) {
                "A speed test is already running"
            } else {
                stats.update(cx, |s, cx| s.start_speed_test(cx));
                "Speed test started"
            };
            Response::SpeedTest {
                run: stats.read(cx).speed_test_run,
                message: message.into(),
            }
        }
        Request::SpeedTestCancel => {
            let stats = stats::stats(cx);
            if !matches!(stats.read(cx).speed_test, SpeedTest::Running { .. }) {
                return Response::done("No speed test was running");
            }
            stats.update(cx, |s, cx| s.cancel_speed_test(cx));
            Response::done("Speed test cancelled")
        }
        Request::SpeedTestCopy => match &stats::stats(cx).read(cx).speed_test {
            SpeedTest::Done { result, .. } => {
                let summary = dashboard::speed_test_summary(result);
                cx.write_to_clipboard(ClipboardItem::new_string(summary.clone()));
                Response::done(format!("Copied: {summary}"))
            }
            _ => Response::error("There's no speed test result to copy yet"),
        },
        Request::HealthFix { id } => {
            let fix = stats::stats(cx)
                .read(cx)
                .health
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.fix);
            match fix {
                None => Response::error(format!("No health check with the id \"{id}\"")),
                Some(None) => Response::error(format!("The \"{id}\" check has no fix to run")),
                Some(Some(fix)) => {
                    dashboard::run_health_fix(fix, cx);
                    Response::done("Done")
                }
            }
        }
        Request::RunHealth => {
            stats::stats(cx).update(cx, |s, cx| s.run_health(cx));
            Response::done("Running the health checks")
        }
        Request::DismissProblem { id } => {
            let stats = stats::stats(cx);
            match stats.read(cx).problems.iter().find(|p| p.id == id) {
                None => return Response::error(format!("No problem with the id \"{id}\"")),
                Some(p) if !p.dismissable => {
                    return Response::error(format!(
                        "\"{}\" clears by itself once it's fixed",
                        p.title
                    ));
                }
                Some(_) => {}
            }
            stats.update(cx, |s, cx| {
                s.clear_problem(&id);
                cx.notify();
            });
            Response::done("Dismissed")
        }
        Request::SnoozeAlert { tag } => {
            let Some(kind) = utilify_core::alerts::AlertKind::from_tag(&tag) else {
                return Response::error(format!("No notification called \"{tag}\""));
            };
            stats::stats(cx).update(cx, |s, cx| {
                s.alerts.snooze(kind, Instant::now());
                cx.notify();
            });
            Response::done("Snoozed for an hour")
        }
        Request::SetLaunchAtLogin { on } => {
            let Some(now) = system::login::enabled() else {
                return Response::error("Launch at login needs macOS 13 and the bundled app");
            };
            if now != on {
                crate::toggle_launch_at_login(cx);
            }
            match system::login::enabled() {
                Some(state) if state == on => Response::done(if on {
                    "Utilify will open at login"
                } else {
                    "Utilify won't open at login"
                }),
                _ => Response::error(
                    "macOS didn't take the change. Check System Settings › General › Login Items.",
                ),
            }
        }
        Request::OpenDashboard { tab } => {
            match tab.as_deref() {
                Some(name) => match dashboard::Tab::parse(name) {
                    Some(tab) => dashboard::open_on(tab, cx),
                    None => return Response::error(format!("No tab called \"{name}\"")),
                },
                None => dashboard::open(cx),
            }
            Response::ok()
        }
        Request::OpenWorkbench => match workbench::open() {
            Ok(()) => Response::ok(),
            Err(e) => Response::error(e),
        },
        Request::Quit => {
            // Answer first, then quit through the normal path (fans back to
            // automatic, the session's last stretch saved).
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
            Response::done("Quitting Utilify")
        }
    }
}

fn set_setting(key: &str, value: &str, cx: &mut App) -> Response {
    let s = stats::stats(cx).read(cx);
    let readings = &s.fans.sensors;
    let known = |id: &str| {
        id == sensors::CPU_AVERAGE_ID
            // Not read yet: take it, as the setting falls back by itself.
            || readings.is_empty()
            || sensors::find(readings, id).is_some()
    };
    let mut new = s.settings.clone();
    match new.set_field(key, value, known) {
        Ok(stored) => {
            let field = utilify_core::settings::find_field(key).map_or(key, |f| f.key);
            let message = format!("{field} = {stored}");
            crate::update_settings(cx, move |s| *s = new);
            Response::done(message)
        }
        Err(e) => Response::error(e),
    }
}

fn awake_start(mode: AwakeMode, cx: &mut App) -> Response {
    let mut subject = None;
    match mode {
        AwakeMode::For { minutes } if minutes == 0 || minutes > FOR_MAX_MINS => {
            return Response::error(format!(
                "A session lasts 1 minute to {} hours",
                FOR_MAX_MINS / 60
            ));
        }
        AwakeMode::WhileApp { pid } => {
            match system::running_apps().into_iter().find(|a| a.pid == pid) {
                Some(app) => subject = Some(app.name),
                None => {
                    return Response::error(format!(
                        "No running app with pid {pid} (`utilifyctl awake apps` lists them)"
                    ));
                }
            }
        }
        _ => {}
    }
    let mode = mode.to_mode();
    crate::start_keep_awake_with(cx, mode, Activation::Manual, subject);
    let status = stats::stats(cx)
        .read(cx)
        .keep_awake
        .status(Instant::now())
        .unwrap_or_default();
    Response::done(format!("Keeping the Mac awake: {status}"))
}

fn awake_extend(minutes: u32, cx: &mut App) -> Response {
    let now = Instant::now();
    let left = {
        let k = &stats::stats(cx).read(cx).keep_awake;
        match k.session() {
            None => return Response::error("No session is running"),
            Some(s) => match s.remaining(now) {
                Some(left) => left,
                None => {
                    return Response::error(format!(
                        "Only timed sessions can be extended; this one runs {}",
                        s.status(now).to_lowercase()
                    ));
                }
            },
        }
    };
    // Whole seconds, so 1:29:59.6 left plus 15 minutes reads 1:45:00.
    let more = Duration::from_secs(left.as_secs_f64().ceil() as u64 + minutes as u64 * 60);
    if more > Duration::from_secs(FOR_MAX_MINS as u64 * 60) {
        return Response::error(format!(
            "A session can't run longer than {} hours",
            FOR_MAX_MINS / 60
        ));
    }
    // As the notification's Add 30 Minutes does.
    crate::start_keep_awake(cx, Mode::For(more), Activation::Manual);
    Response::done(format!(
        "Session extended: {} left",
        utilify_core::format::countdown(more.as_secs())
    ))
}

/// Why changing the fans isn't possible now, as the Fans tab's banner says.
fn fans_unavailable(stats: &Stats) -> Option<String> {
    let (_, detail) = helper_state(&stats.fans.helper);
    (!stats.fans.helper.is_ready()).then(|| format!("Can't set the fans: {detail}"))
}

fn fan_activate(active: Active, cx: &mut App) -> Response {
    let s = stats::stats(cx).read(cx);
    // Automatic is always allowed, as in the dashboard and the menu: it's
    // how to clear manual control the helper can no longer apply.
    if active != Active::Automatic
        && let Some(why) = fans_unavailable(s)
    {
        return Response::error(why);
    }
    if let Active::Preset(name) = &active
        && s.fan_settings.preset(name).is_none()
    {
        return Response::error(format!("No preset called \"{name}\""));
    }
    let label = active.label().to_string();
    crate::update_fan_settings(cx, move |s, _| s.active = active);
    Response::done(format!("Fans: {label}"))
}

fn fan_set_control(index: u8, control: Control, cx: &mut App) -> Response {
    let s = stats::stats(cx).read(cx);
    if let Some(why) = fans_unavailable(s) {
        return Response::error(why);
    }
    let Some(fan) = s.fans.fans.iter().find(|f| f.info.index == index) else {
        return Response::error(format!("No fan {index} (`utilifyctl fans` lists them)"));
    };
    let info = fan.info;
    let name = fan.name.clone();
    let control = match control {
        Control::Auto => Control::Auto,
        Control::Constant(rpm) if rpm.is_finite() => {
            Control::Constant(rpm.clamp(info.min, info.max))
        }
        Control::Constant(_) => return Response::error("That isn't a speed"),
        Control::SensorCurve {
            sensor_id,
            from_c,
            to_c,
        } => {
            if s.fans.sensors.is_empty() {
                return Response::error("The sensors haven't been read yet; try again in a moment");
            }
            if sensors::find(&s.fans.sensors, &sensor_id).is_none() {
                return Response::error(format!(
                    "No sensor called \"{sensor_id}\" (`utilifyctl sensors` lists them)"
                ));
            }
            if !CURVE_C.contains(&from_c) || !CURVE_C.contains(&to_c) || from_c >= to_c {
                return Response::error(format!(
                    "A curve runs between {:.0} and {:.0}°C, from a lower temperature to a higher one",
                    CURVE_C.start(),
                    CURVE_C.end()
                ));
            }
            Control::SensorCurve {
                sensor_id,
                from_c,
                to_c,
            }
        }
    };
    let label = control.describe(|id| sensors::find(&s.fans.sensors, id).map(|x| x.name.clone()));
    crate::update_fan_settings(cx, move |s, fans| s.set_control(fans, index, control));
    let active = stats::stats(cx)
        .read(cx)
        .fan_settings
        .active
        .label()
        .to_string();
    Response::done(format!("{name}: {label} (fans: {active})"))
}

/// Edit the presets as the Fans tab's editor does.
fn fan_presets(
    cx: &mut App,
    edit: impl FnOnce(&mut fans::FanSettings, &[utilify_smc::FanInfo]) -> Result<String, String>,
) -> Response {
    let stats = stats::stats(cx);
    let mut outcome = Err(String::new());
    stats.update(cx, |s, cx| {
        let mut f = s.fan_settings.clone();
        outcome = edit(&mut f, &s.fan_infos());
        if outcome.is_ok() {
            s.update_fan_settings(|settings, _| *settings = f, cx);
        }
    });
    crate::tray::update(stats.read(cx));
    match outcome {
        Ok(m) => Response::done(m),
        Err(e) => Response::error(e),
    }
}

fn preset_name_problem(name: &str) -> String {
    if name.trim().is_empty() {
        "Type a name first".into()
    } else {
        format!("Can't use \"{}\": it's taken or built in", name.trim())
    }
}

/// The helper's state as a code and in words.
fn helper_state(h: &HelperStatus) -> (&'static str, String) {
    match h {
        HelperStatus::Unknown => ("unknown", "checking the fan helper".into()),
        HelperStatus::Unsupported => (
            "unsupported",
            "fan control needs an Apple Silicon Mac; speeds are read-only here".into(),
        ),
        HelperStatus::NotInstalled => (
            "not_installed",
            format!(
                "the fan helper isn't installed; run {} from the Utilify source folder",
                fans::client::INSTALL_COMMAND
            ),
        ),
        HelperStatus::NotRunning => (
            "not_running",
            "the fan helper is installed but not running".into(),
        ),
        HelperStatus::PermissionDenied => (
            "permission_denied",
            "only admin users can set fan speeds".into(),
        ),
        HelperStatus::Ready => ("ready", "connected".into()),
        HelperStatus::Broken(e) => ("broken", e.clone()),
    }
}

fn snapshot(with_disk_history: bool, cx: &mut App) -> ipc::Snapshot {
    let dashboard_open = dashboard::is_open(cx);
    let workbench_open = workbench::is_open();
    let s = stats::stats(cx).read(cx);
    let now = Instant::now();
    let m = &s.monitor;
    let rates = m.rates();
    let (session_down, session_up) = m.session_totals();
    let network = ipc::Network {
        down: rates.down,
        up: rates.up,
        link: rates.link.as_ref().map(|l| ipc::LinkView {
            name: l.name.clone(),
            capacity_bps: l.capacity_bps,
        }),
        utilization: rates.utilization(),
        session_down,
        session_up,
        history: m.history().iter().map(|p| [p.down, p.up]).collect(),
        history_capacity: m.history().capacity(),
    };

    let disk = m.disk();
    let samples = s.disk_history.with_live(disk_history::unix_now(), disk);
    let change = match (samples.first(), samples.last()) {
        (Some(a), Some(b)) if samples.len() >= 2 && b.at > a.at => {
            Some((b.used as i64 - a.used as i64, b.at - a.at))
        }
        _ => None,
    };
    let storage = ipc::Storage {
        disk: disk.map(|d| ipc::DiskView {
            total: d.total,
            available: d.available,
            used: d.used(),
        }),
        level: disk.map(|d| {
            use utilify_core::settings::StorageLevel;
            match s.settings.storage_level(d.used_fraction()) {
                StorageLevel::Ok => "ok",
                StorageLevel::Warn => "warn",
                StorageLevel::Danger => "danger",
            }
            .into()
        }),
        history: if with_disk_history {
            samples
        } else {
            Vec::new()
        },
        change,
    };

    let speed_test = match &s.speed_test {
        SpeedTest::Idle => ipc::SpeedTestState::Idle,
        SpeedTest::Running { started } => ipc::SpeedTestState::Running {
            elapsed_secs: started.elapsed().as_secs_f64(),
        },
        SpeedTest::Done { result, finished } => ipc::SpeedTestState::Done {
            down_bps: result.down_bps,
            up_bps: result.up_bps,
            idle_latency_ms: result.idle_latency_ms,
            responsiveness_rpm: result.responsiveness_rpm,
            interface: result.interface.clone(),
            ago_secs: finished.elapsed().as_secs(),
            summary: dashboard::speed_test_summary(result),
        },
        SpeedTest::Failed(e) => ipc::SpeedTestState::Failed { error: e.clone() },
    };

    let k = &s.keep_awake;
    let awake = ipc::Awake {
        session: k.session().map(|session| ipc::AwakeSession {
            mode: AwakeMode::from_mode(session.mode),
            label: session.mode.label(),
            status: session.status(now),
            activation: session.activation.label().into(),
            subject: session.subject.clone(),
            elapsed_secs: now.saturating_duration_since(session.started).as_secs(),
            remaining_secs: session
                .remaining(now)
                .map(|d| d.as_secs_f64().ceil() as u64),
        }),
        total_secs: k.total().as_secs(),
    };

    let f = &s.fans;
    let readings = &f.sensors;
    let sensor_name = |id: &str| sensors::find(readings, id).map(|x| x.name.clone());
    let (helper, helper_detail) = helper_state(&f.helper);
    let fans_view = ipc::Fans {
        control_supported: fans::service::CONTROL_SUPPORTED,
        helper: helper.into(),
        helper_detail,
        ready: f.helper.is_ready(),
        error: f.error.clone(),
        foreign: f.foreign,
        overheat: f.overheat.as_ref().map(|o| ipc::OverheatView {
            sensor: o.sensor.clone(),
            celsius: o.celsius,
        }),
        taking_over: f.taking_over,
        read_ago_secs: f.read_at.map(|t| t.elapsed().as_secs_f64()),
        active: s.fan_settings.active.clone(),
        active_label: s.fan_settings.active.label().into(),
        presets: s.fan_settings.presets.clone(),
        fans: f
            .fans
            .iter()
            .map(|fan| ipc::FanView {
                index: fan.info.index,
                name: fan.name.clone(),
                rpm: fan.info.current,
                min: fan.info.min,
                max: fan.info.max,
                manual: fan.info.manual,
                control: fan.control.clone(),
                control_label: fan.control.describe(sensor_name),
                target: fan.target,
                issue: fan.issue.clone(),
            })
            .collect(),
        sensors: readings
            .iter()
            .map(|x| ipc::SensorView {
                id: x.id.clone(),
                name: x.name.clone(),
                group: x.group.label().into(),
                celsius: x.celsius,
            })
            .collect(),
        cpu_average: sensors::cpu_average(readings),
    };

    ipc::Snapshot {
        app_version: env!("CARGO_PKG_VERSION").into(),
        network,
        storage,
        speed_test,
        speed_test_run: s.speed_test_run,
        awake,
        fans: fans_view,
        power: s.power.into(),
        thermal: format!("{:?}", s.thermal).to_lowercase(),
        problems: s
            .problems
            .iter()
            .map(|p| ipc::ProblemView {
                id: p.id.into(),
                status: p.status.into(),
                title: p.title.clone(),
                detail: p.detail.clone(),
                dismissable: p.dismissable,
            })
            .collect(),
        health: ipc::HealthReport {
            checks: s
                .health
                .iter()
                .map(|c| ipc::CheckView {
                    id: c.id.into(),
                    name: c.name.clone(),
                    status: c.status.into(),
                    detail: c.detail.clone(),
                    fix: c.fix.map(str::to_string),
                })
                .collect(),
            ago_secs: s.health_at.map(|t| t.elapsed().as_secs()),
            running: s.health_running(),
        },
        settings: s.settings.clone(),
        launch_at_login: system::login::enabled_cached(),
        dashboard_open,
        workbench_open,
        workbench_available: workbench::is_available(),
    }
}
