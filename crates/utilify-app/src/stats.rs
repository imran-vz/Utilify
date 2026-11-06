//! The app's model: the sampling [`Monitor`], settings, the stored disk
//! history, the speed test, keep-awake sessions and fan control. Views
//! observe it; [`spawn_sampler`] advances it on a timer, and the fan thread
//! ([`FanService`]) posts its own snapshots.

use gpui_kit::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use utilify_core::awake::{
    self, Activation, Assertions, KeepAwake, Kind, Mode, ScreenSaverGate, Session, Signals,
};
use utilify_core::disk_history::{self, DiskHistory};
use utilify_core::fans::client::HelperPaths;
use utilify_core::fans::service::{FanService, FanSnapshot};
use utilify_core::fans::{self, FanSettings};
use utilify_core::health::{self, Check, Health};
use utilify_core::monitor::Monitor;
use utilify_core::power::{self, PowerState};
use utilify_core::alerts::{
    AlertEvent, AlertInputs, AlertKind, Alerts, AwakeInputs, FanInputs, StorageInputs, Thermal,
};
use utilify_core::settings::{self, Settings};
use utilify_core::speedtest::{self, SpeedTestResult};
use utilify_smc::FanInfo;

/// How often the power source is read.
const POWER_EVERY: Duration = Duration::from_secs(5);
/// How often the self-checks run, and how long after launch the first does.
const HEALTH_EVERY: Duration = Duration::from_secs(15 * 60);
const HEALTH_FIRST: Duration = Duration::from_secs(5);
/// A failed save is retried this often until it works.
const SAVE_RETRY: Duration = Duration::from_secs(30);

/// Something the user should know about and may be able to fix: a save
/// that failed, a file that had to be set aside. Shown as a banner on
/// Overview and Settings until it clears.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub id: &'static str,
    pub status: Health,
    pub title: String,
    pub detail: String,
    /// Can be closed by hand (it won't clear by itself).
    pub dismissable: bool,
}

/// The files the model saves, retried when a save fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Store {
    Settings,
    Fans,
    Awake,
    DiskHistory,
}

impl Store {
    fn problem_id(self) -> &'static str {
        match self {
            Store::Settings => "save-settings",
            Store::Fans => "save-fans",
            Store::Awake => "save-awake",
            Store::DiskHistory => "save-disk-history",
        }
    }

    fn what(self) -> &'static str {
        match self {
            Store::Settings => "your settings",
            Store::Fans => "your fan presets",
            Store::Awake => "the keep-awake total",
            Store::DiskHistory => "the storage history",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum SpeedTest {
    #[default]
    Idle,
    Running {
        started: Instant,
    },
    Done {
        result: SpeedTestResult,
        finished: Instant,
    },
    Failed(String),
}

pub struct Stats {
    pub monitor: Monitor,
    pub settings: Settings,
    settings_path: Option<PathBuf>,
    pub disk_history: DiskHistory,
    disk_history_path: Option<PathBuf>,
    pub speed_test: SpeedTest,
    /// Set to stop the running test; each run gets its own flag.
    speed_test_cancel: Option<Arc<AtomicBool>>,
    /// What the speed test showed before the current run, restored on cancel.
    speed_test_before: SpeedTest,
    /// Runs started so far, and the one whose state [`Stats::speed_test`]
    /// shows (0 for none), so a waiting `utilifyctl` can tell its own
    /// result from one a cancel put back.
    speed_test_runs: u64,
    pub speed_test_run: u64,
    speed_test_before_run: u64,
    /// The notification rules, and the system thermal state they watch
    /// (set by the sampler from gpui).
    pub alerts: Alerts,
    pub thermal: Thermal,
    pub keep_awake: KeepAwake,
    awake_path: Option<PathBuf>,
    /// Held while a keep-awake session runs.
    assertions: Assertions,
    screen_saver: ScreenSaverGate,
    /// Watched by "While File Is Downloading".
    downloads_dir: Option<PathBuf>,
    /// The latest fan and sensor readings, and the helper's state.
    pub fans: FanSnapshot,
    pub fan_settings: FanSettings,
    fan_settings_path: Option<PathBuf>,
    fan_service: Option<FanService>,
    /// Last sensor demand sent to the fan thread.
    fan_sensors_wanted: bool,
    /// When `utilifyctl` last asked for the sensors; they're read for a
    /// while after ([`crate::control::SENSORS_FOR`]).
    pub cli_sensors_at: Option<Instant>,
    pub power: PowerState,
    power_read_at: Option<Instant>,
    /// The last self-check results, and when they ran.
    pub health: Vec<Check>,
    pub health_at: Option<Instant>,
    health_running: bool,
    started: Instant,
    pub problems: Vec<Problem>,
    /// Saves that failed, and when the last retry was.
    unsaved: std::collections::HashSet<Store>,
    retried_at: Option<Instant>,
}

impl Stats {
    pub fn new(settings: Settings, settings_path: Option<PathBuf>) -> Self {
        let disk_history_path = disk_history::default_path();
        let awake_path = awake::default_path();
        let fan_settings_path = fans::default_path();
        Self {
            monitor: Monitor::default(),
            settings,
            settings_path,
            disk_history: disk_history_path
                .as_deref()
                .map(disk_history::load)
                .unwrap_or_default(),
            disk_history_path,
            speed_test: SpeedTest::Idle,
            speed_test_cancel: None,
            speed_test_before: SpeedTest::Idle,
            speed_test_runs: 0,
            speed_test_run: 0,
            speed_test_before_run: 0,
            alerts: Alerts::new(),
            thermal: Thermal::Nominal,
            keep_awake: KeepAwake::with_total(
                awake_path
                    .as_deref()
                    .map(awake::load_total)
                    .unwrap_or_default(),
            ),
            awake_path,
            assertions: Assertions::default(),
            screen_saver: ScreenSaverGate::default(),
            downloads_dir: awake::downloads_dir(),
            fans: FanSnapshot::default(),
            fan_settings: fan_settings_path
                .as_deref()
                .map(fans::load)
                .unwrap_or_default(),
            fan_settings_path,
            fan_service: None,
            fan_sensors_wanted: false,
            cli_sensors_at: None,
            power: power::read(),
            power_read_at: Some(Instant::now()),
            health: Vec::new(),
            health_at: None,
            health_running: false,
            started: Instant::now(),
            problems: recovered_problems(),
            unsaved: Default::default(),
            retried_at: None,
        }
    }

    /// Record a problem, replacing one with the same id.
    pub fn set_problem(&mut self, problem: Problem) {
        match self.problems.iter_mut().find(|p| p.id == problem.id) {
            Some(p) => *p = problem,
            None => self.problems.push(problem),
        }
    }

    pub fn clear_problem(&mut self, id: &str) {
        self.problems.retain(|p| p.id != id);
    }

    /// Write one store now. A failure becomes a [`Problem`] and is retried
    /// every [`SAVE_RETRY`]; the next success clears it.
    fn save(&mut self, store: Store) {
        let result = match store {
            Store::Settings => self
                .settings_path
                .as_ref()
                .map(|p| settings::save(p, &self.settings)),
            Store::Fans => self
                .fan_settings_path
                .as_ref()
                .map(|p| fans::save(p, &self.fan_settings)),
            Store::Awake => self
                .awake_path
                .as_ref()
                .map(|p| awake::save_total(p, self.keep_awake.total())),
            Store::DiskHistory => self
                .disk_history_path
                .as_ref()
                .map(|p| disk_history::save(p, &self.disk_history)),
        };
        match result {
            None | Some(Ok(())) => {
                if self.unsaved.remove(&store) {
                    log::info!("Saved {} after an earlier failure", store.what());
                }
                self.clear_problem(store.problem_id());
            }
            Some(Err(e)) => {
                log::warn!("Saving {} failed: {e}", store.what());
                self.unsaved.insert(store);
                self.set_problem(Problem {
                    id: store.problem_id(),
                    status: Health::Fail,
                    title: format!("Couldn't save {}", store.what()),
                    detail: format!(
                        "{}. Changes are kept while Utilify runs and saved as soon as \
                         this works; retrying every {} s.",
                        save_error(&e),
                        SAVE_RETRY.as_secs()
                    ),
                    dismissable: false,
                });
            }
        }
    }

    /// Retry failed saves, at most every [`SAVE_RETRY`].
    fn retry_saves(&mut self, now: Instant) {
        if self.unsaved.is_empty() {
            self.retried_at = None;
            return;
        }
        // The first failure starts the clock; retry once it's run out.
        let since = *self.retried_at.get_or_insert(now);
        if now.duration_since(since) < SAVE_RETRY {
            return;
        }
        self.retried_at = Some(now);
        let stores: Vec<Store> = self.unsaved.iter().copied().collect();
        for store in stores {
            self.save(store);
        }
    }

    fn read_power(&mut self, now: Instant) {
        if self
            .power_read_at
            .is_some_and(|t| now.duration_since(t) < POWER_EVERY)
        {
            return;
        }
        self.power_read_at = Some(now);
        // Keep the last good reading: a failed one would look like AC.
        if let Some(p) = power::try_read() {
            self.power = p;
        }
    }

    /// Whether the self-checks are due: shortly after launch, then every
    /// [`HEALTH_EVERY`].
    pub fn health_due(&self, now: Instant) -> bool {
        !self.health_running
            && match self.health_at {
                None => now.duration_since(self.started) >= HEALTH_FIRST,
                Some(t) => now.duration_since(t) >= HEALTH_EVERY,
            }
    }

    pub fn health_running(&self) -> bool {
        self.health_running
    }

    /// Run the self-checks off the main thread, then add the app's own:
    /// the fan helper, launch at login.
    pub fn run_health(&mut self, cx: &mut Context<Self>) {
        if self.health_running {
            return;
        }
        self.health_running = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let mut checks = smol::unblock(health::run_core_checks).await;
            let _ = this.update(cx, |s, cx| {
                checks.extend(s.app_checks());
                for c in &checks {
                    if c.status != Health::Ok {
                        log::warn!("Health check {}: {}", c.name, c.detail);
                    }
                }
                s.health = checks;
                s.health_at = Some(Instant::now());
                s.health_running = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Checks only the app can make.
    fn app_checks(&self) -> Vec<Check> {
        use utilify_core::fans::controller::HelperStatus;
        let check = |id, name: &str, status, detail: &str| Check {
            id,
            name: name.into(),
            status,
            detail: detail.into(),
            fix: None,
        };
        let helper = match &self.fans.helper {
            HelperStatus::Ready => check("fan-helper", "Fan helper", Health::Ok, "Connected"),
            HelperStatus::Unknown => {
                check("fan-helper", "Fan helper", Health::Ok, "Still connecting")
            }
            HelperStatus::Unsupported => check(
                "fan-helper",
                "Fan helper",
                Health::Ok,
                "Not needed: fans are read-only on this Mac",
            ),
            HelperStatus::NotInstalled => check(
                "fan-helper",
                "Fan helper",
                Health::Warn,
                "Not installed; fan speeds can't be set",
            ),
            HelperStatus::NotRunning => check(
                "fan-helper",
                "Fan helper",
                Health::Warn,
                "Installed but not running",
            ),
            HelperStatus::PermissionDenied => check(
                "fan-helper",
                "Fan helper",
                Health::Warn,
                "Only admin users can set fan speeds",
            ),
            HelperStatus::Broken(e) => check("fan-helper", "Fan helper", Health::Fail, e),
        };
        let sensors = match self.fans.read_at {
            Some(t) if t.elapsed() > Duration::from_secs(30) => check(
                "fan-readings",
                "Fan and sensor readings",
                Health::Fail,
                "No new reading for over 30 s",
            ),
            Some(_) => check(
                "fan-readings",
                "Fan and sensor readings",
                Health::Ok,
                "Updating",
            ),
            None => check(
                "fan-readings",
                "Fan and sensor readings",
                Health::Warn,
                "No reading yet",
            ),
        };
        let login = match crate::system::login::enabled() {
            Some(true) => check("login-item", "Launch at login", Health::Ok, "On"),
            Some(false) => check("login-item", "Launch at login", Health::Ok, "Off"),
            None => check(
                "login-item",
                "Launch at login",
                Health::Ok,
                "Needs macOS 13 and the bundled app",
            ),
        };
        vec![helper, sensors, login]
    }

    /// Start the fan thread; its snapshots land in [`Stats::fans`].
    pub fn start_fans(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = smol::channel::unbounded();
        self.fan_service = Some(FanService::spawn(
            self.fan_settings.clone(),
            helper_paths(),
            move |snapshot| {
                let _ = tx.try_send(snapshot);
            },
        ));
        cx.spawn(async move |this, cx| {
            while let Ok(mut snapshot) = rx.recv().await {
                // Only the newest matters.
                while let Ok(newer) = rx.try_recv() {
                    snapshot = newer;
                }
                if this
                    .update(cx, |s, cx| {
                        s.fans = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }

    /// The fans as last read, for edits that depend on them.
    pub fn fan_infos(&self) -> Vec<FanInfo> {
        self.fans.fans.iter().map(|f| f.info).collect()
    }

    /// Change the fan presets or controls, persist them, and apply them now.
    pub fn update_fan_settings(
        &mut self,
        f: impl FnOnce(&mut FanSettings, &[FanInfo]),
        cx: &mut Context<Self>,
    ) {
        let infos = self.fan_infos();
        f(&mut self.fan_settings, &infos);
        self.save(Store::Fans);
        if let Some(service) = &self.fan_service {
            service.set_settings(self.fan_settings.clone());
        }
        cx.notify();
    }

    /// Have the fan thread read the sensors every tick (for the dashboard
    /// or the menu-bar temperature), or only when a curve needs them.
    pub fn want_fan_sensors(&mut self, on: bool) {
        if on != self.fan_sensors_wanted
            && let Some(service) = &self.fan_service
        {
            service.want_sensors(on);
            self.fan_sensors_wanted = on;
        }
    }

    /// Sample the OS counters, advance the keep-awake session, and store
    /// disk usage when a sample is due. Returns what to post and take back
    /// in Notification Center.
    pub fn tick(&mut self, now: Instant) -> Outbox {
        self.monitor.tick(now);
        let mut out = Outbox::default();
        let signals = self.awake_signals();
        if self.keep_awake.tick(now, &signals) {
            log::info!("Keep-awake session ended");
            if self.settings.notify_enabled && self.settings.awake_notify_end {
                out.post.push(SystemNotification {
                    tag: "awake-ended".into(),
                    title: "Keep-awake session ended".into(),
                    body: "Your Mac can sleep normally again.".into(),
                    actions: Vec::new(),
                });
            }
        }
        self.sync_awake();
        self.screen_saver_check();
        if let Some(disk) = self.monitor.disk()
            && self.disk_history.record(disk_history::unix_now(), disk)
        {
            self.save(Store::DiskHistory);
        }
        self.read_power(now);
        self.retry_saves(now);
        for event in self.alerts.tick(&self.alert_inputs(now), &self.settings.alert_prefs()) {
            match event {
                AlertEvent::Post(a) => out.post.push(SystemNotification {
                    tag: a.kind.tag().into(),
                    title: a.title.into(),
                    body: a.body.into(),
                    actions: a
                        .actions
                        .into_iter()
                        .map(|(id, label)| SystemNotificationAction {
                            id: id.into(),
                            label: label.into(),
                        })
                        .collect(),
                }),
                AlertEvent::Retract(kind) => out.retract.push(kind.tag()),
            }
        }
        out
    }

    /// What the notification rules look at this tick.
    fn alert_inputs(&self, now: Instant) -> AlertInputs {
        let (_, red) = self.settings.storage_thresholds();
        AlertInputs {
            now,
            power: self.power,
            fans: FanInputs::from_snapshot(&self.fans, &self.fan_settings.active),
            thermal: self.thermal,
            awake: AwakeInputs::from_keep_awake(&self.keep_awake, self.settings.awake_allow(), now),
            online: self.monitor.rates().link.is_some(),
            storage: StorageInputs {
                used: self.monitor.disk().map(|d| d.used_fraction()),
                red,
            },
        }
    }

    /// Every notification the rules may have posted: on quit they no
    /// longer apply (the fans go back to automatic, the session ends).
    pub fn alert_tags() -> impl Iterator<Item = &'static str> {
        AlertKind::ALL.into_iter().map(AlertKind::tag)
    }

    /// Only what the running session's mode needs is looked up.
    fn awake_signals(&self) -> Signals {
        let mode = self.keep_awake.session().map(|s| s.mode);
        Signals {
            down_rate: self.monitor.rates().down,
            file_downloading: mode == Some(Mode::WhileFileDownloading)
                && self
                    .downloads_dir
                    .as_deref()
                    .is_some_and(awake::downloads_in_progress),
            app_running: matches!(mode, Some(Mode::WhileApp(pid)) if awake::process_alive(pid)),
        }
    }

    /// "Allow screen saver after N min": blocking display sleep also blocks
    /// the screen saver, so start it ourselves once the user is idle that
    /// long.
    fn screen_saver_check(&mut self) {
        let watching = self.keep_awake.is_active()
            && !self.settings.awake_allow_display_sleep
            && self.settings.awake_screen_saver;
        let idle = if watching {
            awake::idle_time()
        } else {
            Duration::ZERO
        };
        if self
            .screen_saver
            .update(idle, self.settings.awake_screen_saver_after())
        {
            log::info!("Idle for {} s; starting the screen saver", idle.as_secs());
            awake::start_screen_saver();
        }
    }

    /// Start a speed test in the background, unless one is running.
    pub fn start_speed_test(&mut self, cx: &mut Context<Self>) {
        if matches!(self.speed_test, SpeedTest::Running { .. }) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.speed_test_cancel = Some(cancel.clone());
        self.speed_test_runs += 1;
        self.speed_test_before_run = std::mem::replace(&mut self.speed_test_run, self.speed_test_runs);
        self.speed_test_before = std::mem::replace(
            &mut self.speed_test,
            SpeedTest::Running {
                started: Instant::now(),
            },
        );
        cx.notify();
        cx.spawn(async move |this, cx| {
            let flag = cancel.clone();
            let result = smol::unblock(move || speedtest::run(&flag)).await;
            let _ = this.update(cx, |s, cx| {
                // A cancelled run already restored the previous state.
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                s.speed_test_cancel = None;
                s.speed_test = match result {
                    Ok(result) => SpeedTest::Done {
                        result,
                        finished: Instant::now(),
                    },
                    Err(e) => {
                        log::warn!("Speed test failed: {e}");
                        SpeedTest::Failed(e.to_string())
                    }
                };
                // Closed the dashboard while it ran: say how it went.
                if s.settings.alert_prefs().allows_speed_test() && !crate::dashboard::is_open(cx) {
                    let (title, body) = match &s.speed_test {
                        SpeedTest::Done { result, .. } => (
                            "Speed test finished",
                            format!(
                                "↓ {} · ↑ {}",
                                utilify_core::format::bits_per_sec(result.down_bps),
                                utilify_core::format::bits_per_sec(result.up_bps)
                            ),
                        ),
                        _ => (
                            "Speed test didn't finish",
                            "Open the Network tab to try again.".to_string(),
                        ),
                    };
                    cx.show_system_notification(SystemNotification {
                        tag: "speed-test".into(),
                        title: title.into(),
                        body: body.into(),
                        actions: Vec::new(),
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stop the running test and go back to what was shown before it.
    pub fn cancel_speed_test(&mut self, cx: &mut Context<Self>) {
        let Some(cancel) = self.speed_test_cancel.take() else {
            return;
        };
        cancel.store(true, Ordering::Relaxed);
        self.speed_test = std::mem::take(&mut self.speed_test_before);
        self.speed_test_run = self.speed_test_before_run;
        cx.notify();
    }

    /// Start a keep-awake session, replacing a running one. `subject` names
    /// the app for [`Mode::WhileApp`].
    pub fn start_keep_awake(
        &mut self,
        mode: Mode,
        activation: Activation,
        subject: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.keep_awake.start(Session::new(
            mode,
            activation,
            subject,
            Instant::now(),
            awake::local_secs_of_day(),
        ));
        self.sync_awake();
        cx.notify();
    }

    pub fn stop_keep_awake(&mut self, cx: &mut Context<Self>) {
        self.keep_awake.stop(Instant::now());
        self.sync_awake();
        cx.notify();
    }

    /// On quit: end the session so its last stretch is saved, release the
    /// assertion, and put the fans back to automatic.
    pub fn shutdown(&mut self) {
        self.keep_awake.stop(Instant::now());
        self.sync_awake();
        if let Some(service) = self.fan_service.take()
            && !service.shutdown(Duration::from_secs(2))
        {
            // The helper restores them when our connection closes anyway.
            log::warn!("The fan thread didn't stop in time");
        }
    }

    /// Hold the power assertions the session and settings call for, and
    /// save the awake total when due.
    fn sync_awake(&mut self) {
        let want = if self.keep_awake.is_active() {
            Kind::needed(self.settings.awake_allow())
        } else {
            Vec::new()
        };
        self.assertions.sync(&want);
        if self.keep_awake.take_save_due() {
            self.save(Store::Awake);
        }
    }

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        f(&mut self.settings);
        self.sync_awake();
        self.save(Store::Settings);
        cx.notify();
    }
}

/// Files that didn't parse at launch and were set aside, as one notice.
fn recovered_problems() -> Vec<Problem> {
    let files = utilify_core::take_recovered_files();
    if files.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    vec![Problem {
        id: "recovered-files",
        status: Health::Warn,
        title: "Some saved data couldn't be read".into(),
        detail: format!(
            "Utilify started those parts fresh and kept the damaged copies next to \
             the originals: {}.",
            names.join(", ")
        ),
        dismissable: true,
    }]
}

/// An I/O error in words: the disk is full, no permission, or the error.
fn save_error(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(28) => "The disk is full".into(),
        Some(13) | Some(1) => "Utilify isn't allowed to write to its data folder".into(),
        Some(30) => "The data folder is on a read-only disk".into(),
        _ => {
            let mut s = e.to_string();
            if let Some(first) = s.get(..1) {
                s = first.to_uppercase() + &s[1..];
            }
            s
        }
    }
}

/// The helper's socket. Debug builds can point it elsewhere with
/// UTILIFY_DEV_FAN_SOCKET, e.g. at `utilify-fanhelper --fake --no-root-check
/// --socket /tmp/fans.sock`, to try fan control without touching the fans.
fn helper_paths() -> HelperPaths {
    #[cfg(debug_assertions)]
    if let Some(socket) = std::env::var_os("UTILIFY_DEV_FAN_SOCKET") {
        return HelperPaths {
            socket: socket.into(),
            plist: None,
        };
    }
    HelperPaths::default()
}

/// Global handle so tray events can reach the model.
pub struct StatsGlobal(pub Entity<Stats>);

/// Notifications to post and to take back, from one [`Stats::tick`].
#[derive(Default)]
pub struct Outbox {
    pub post: Vec<SystemNotification>,
    pub retract: Vec<&'static str>,
}
impl Global for StatsGlobal {}

pub fn stats(cx: &App) -> Entity<Stats> {
    cx.global::<StatsGlobal>().0.clone()
}

fn thermal(state: ThermalState) -> Thermal {
    match state {
        ThermalState::Nominal => Thermal::Nominal,
        ThermalState::Fair => Thermal::Fair,
        ThermalState::Serious => Thermal::Serious,
        ThermalState::Critical => Thermal::Critical,
    }
}

/// Sample now, then every `settings.interval` (re-read each tick, so changes
/// apply from the next one). `on_tick` runs after each sample.
pub fn spawn_sampler(
    stats: Entity<Stats>,
    on_tick: impl Fn(&Entity<Stats>, &mut App) + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            let interval = cx.update(|cx| {
                let thermal = thermal(cx.thermal_state());
                let out = stats.update(cx, |s, cx| {
                    s.thermal = thermal;
                    let now = Instant::now();
                    let out = s.tick(now);
                    if s.health_due(now) {
                        s.run_health(cx);
                    }
                    cx.notify();
                    out
                });
                for tag in out.retract {
                    log::info!("Retracting notification {tag}");
                    cx.dismiss_system_notification(tag);
                }
                for alert in out.post {
                    log::info!("Notifying: {}", alert.title);
                    cx.show_system_notification(alert);
                }
                on_tick(&stats, cx);
                stats.read(cx).settings.interval()
            });
            cx.background_executor().timer(interval).await;
        }
    })
    .detach();
}
