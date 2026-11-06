//! `utilifyctl`: Utilify from the terminal. It reads and changes the
//! running app over its socket ([`utilify_core::ipc`]), so the menu bar and
//! the dashboard follow along. Settings can also be read and changed while
//! the app is closed; they're written to the file it loads at launch.

mod chart;
mod parse;
mod term;
mod view;

use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use term::{Color, Paint};
use utilify_core::fans::Control;
use utilify_core::ipc::{self, Client, ClientError, Request, Response, Snapshot, SpeedTestState};
use utilify_core::settings::{self, Settings};

#[derive(Parser)]
#[command(
    name = "utilifyctl",
    version,
    about = "Read and control Utilify from the terminal",
    long_about = "Read and control Utilify from the terminal. Changes go through the running \
                  app, so the menu bar and the dashboard update as you make them.",
    after_help = "Run a command with --help for its options. With no command, shows the overview."
)]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Don't use colour (also off when NO_COLOR is set or output isn't a terminal).
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Network, storage, keep awake, fans and problems at a glance.
    Status {
        /// Keep it on screen and updating.
        #[arg(short, long)]
        watch: bool,
    },
    /// A live view of the dashboard's tabs (keys 1–6 switch tabs, q quits).
    Watch {
        #[arg(value_enum, default_value_t = Tab::Overview)]
        tab: Tab,
    },
    /// Download and upload rates, the link, and the last speed test.
    Network {
        #[arg(short, long)]
        watch: bool,
    },
    /// Run a speed test with macOS's networkQuality, and wait for the result.
    Speedtest {
        #[command(subcommand)]
        action: Option<SpeedtestAction>,
        /// Start it and return without waiting.
        #[arg(long)]
        no_wait: bool,
    },
    /// Disk use, the bar colours, and 30 days of history.
    Storage {
        #[arg(short, long)]
        watch: bool,
    },
    /// Keep the Mac awake: show, start, end or extend a session.
    Awake {
        #[command(subcommand)]
        action: Option<AwakeAction>,
        #[arg(short, long)]
        watch: bool,
    },
    /// Fan speeds, controls and presets.
    Fans {
        #[command(subcommand)]
        action: Option<FansAction>,
        #[arg(short, long)]
        watch: bool,
    },
    /// Every temperature sensor.
    Sensors,
    /// List, read or change settings.
    Settings {
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },
    /// The self-checks.
    Health {
        #[command(subcommand)]
        action: Option<HealthAction>,
    },
    /// Problems shown as banners in the dashboard.
    Problems {
        #[command(subcommand)]
        action: Option<ProblemsAction>,
    },
    /// Silence one kind of notification for an hour.
    Snooze {
        /// fans-high-battery, low-battery-awake, lid-sleep-battery,
        /// awake-ending-soon, thermal-pressure, fan-overheat, fan-helper-lost,
        /// foreign-fan-control, network-offline or storage-red.
        kind: String,
    },
    /// Show or change whether Utilify opens at login.
    Login {
        #[arg(value_enum)]
        state: Option<OnOff>,
    },
    /// Open the dashboard, on a tab (starts Utilify if it isn't running).
    Open {
        #[arg(value_enum)]
        tab: Option<DashboardTab>,
    },
    /// The data workbench.
    Workbench {
        #[command(subcommand)]
        action: WorkbenchAction,
    },
    /// Start Utilify.
    Launch,
    /// Quit Utilify (fans go back to automatic and any session ends).
    Quit,
}

#[derive(Subcommand)]
enum SpeedtestAction {
    /// Start one (the default).
    Start,
    /// Stop the one that's running.
    Cancel,
    /// Show the last result.
    Status,
    /// Put the last result on the clipboard, as the dashboard's Copy does.
    Copy,
}

#[derive(Subcommand)]
enum AwakeAction {
    /// Start a session, replacing any running one.
    #[command(
        after_help = "Examples:\n  utilifyctl awake start\n  utilifyctl awake start 2h\n  \
                            utilifyctl awake start 1h30m\n  utilifyctl awake start until 18:30\n  \
                            utilifyctl awake start app Xcode\n  utilifyctl awake start download\n  \
                            utilifyctl awake start network"
    )]
    Start {
        /// indefinitely (the default), a length like 45m or 2h, until <time>,
        /// app <name or pid>, download, or network.
        mode: Vec<String>,
    },
    /// End the session.
    Stop,
    /// Add time to a timed session.
    Extend {
        #[arg(default_value = "30m")]
        by: String,
    },
    /// The apps a session can follow with `start app <name>`.
    Apps,
}

#[derive(Subcommand)]
enum FansAction {
    /// Switch to a preset: automatic, full-blast, custom or one of yours.
    Use {
        preset: String,
    },
    /// Set one fan: auto, a speed like 3000, min, max, or curve <sensor> <from °C> <to °C>.
    #[command(
        after_help = "Examples:\n  utilifyctl fans set 0 3000\n  utilifyctl fans set left max\n  \
                            utilifyctl fans set right curve cpu_core_average 50 85\n  \
                            utilifyctl fans set 1 auto\n\nInside one of your presets this edits the \
                            preset; otherwise the fans switch to Custom, as in the dashboard."
    )]
    Set {
        /// The fan's number or name, as `utilifyctl fans` shows.
        fan: String,
        #[arg(required = true, num_args = 1..)]
        control: Vec<String>,
    },
    /// Save the current fan settings as a preset.
    Save {
        name: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        name: String,
    },
}

#[derive(Subcommand)]
enum SettingsAction {
    /// Every setting with its value and key (the default).
    List,
    Get {
        key: String,
    },
    #[command(
        after_help = "Examples:\n  utilifyctl settings set awake_allow_display_sleep off\n  \
                            utilifyctl settings set interval_ms 2s\n  \
                            utilifyctl settings set storage_danger_pct 90\n  \
                            utilifyctl settings set menu_temp_sensor gpu_max"
    )]
    Set {
        key: String,
        value: String,
    },
    /// Put every setting back to its default.
    Reset {
        /// Confirm it.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum HealthAction {
    /// Run the checks now and show the results.
    Run,
    /// Carry out a check's suggested fix (like Show in Finder), by its id.
    Fix { id: String },
}

#[derive(Subcommand)]
enum ProblemsAction {
    /// Close a problem that doesn't clear by itself.
    Dismiss { id: String },
}

#[derive(Subcommand)]
enum WorkbenchAction {
    /// Open the workbench window (starts Utilify if it isn't running).
    Open,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Tab {
    Overview,
    Network,
    Storage,
    Awake,
    Fans,
    Health,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::Network,
        Tab::Storage,
        Tab::Awake,
        Tab::Fans,
        Tab::Health,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Network => "Network",
            Tab::Storage => "Storage",
            Tab::Awake => "Awake",
            Tab::Fans => "Fans",
            Tab::Health => "Health",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum DashboardTab {
    Overview,
    Network,
    Storage,
    Awake,
    Fans,
    Settings,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// How a command failed, and the exit code it gets.
enum Fail {
    /// The app said no, or the input was wrong: exit 1.
    Message(String),
    /// The app isn't running: exit 2.
    NotRunning,
    /// Already reported in the command's own output: exit 1, quietly.
    Shown,
}

impl From<ClientError> for Fail {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::NotRunning => Fail::NotRunning,
            other => Fail::Message(other.to_string()),
        }
    }
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Message(s)
    }
}

type Result<T> = std::result::Result<T, Fail>;

/// What every command gets: output options and a lazily opened connection.
struct Ctx {
    paint: Paint,
    json: bool,
    client: Option<Client>,
}

impl Ctx {
    fn client(&mut self) -> Result<&mut Client> {
        if self.client.is_none() {
            self.client = Some(Client::connect()?);
        }
        Ok(self.client.as_mut().expect("just set"))
    }

    /// Send a request. Gives up at once on Ctrl-C where it's caught (the
    /// live view, the speed test), instead of waiting out a slow answer.
    fn send(&mut self, request: Request) -> Result<Response> {
        self.send_until(request, &term::stopped)
    }

    /// [`Ctx::send`], with `cancelled` checked every 100 ms while waiting.
    fn send_until(&mut self, request: Request, cancelled: &dyn Fn() -> bool) -> Result<Response> {
        let answer = self.client()?.request_until(request, cancelled);
        if answer.is_err() {
            // Whatever went wrong, this connection is done with.
            self.client = None;
        }
        match answer? {
            Response::Error { message } => Err(Fail::Message(term::clean(&message))),
            other => Ok(other),
        }
    }

    fn snapshot(&mut self, disk_history: bool, sensors: bool) -> Result<Snapshot> {
        match self.send(Request::Snapshot {
            disk_history,
            sensors,
        })? {
            Response::Snapshot { snapshot } => Ok(*snapshot),
            other => Err(unexpected(other)),
        }
    }

    /// A snapshot with the sensors read: the app only reads them while
    /// something shows them, so the first ask can come back empty.
    fn snapshot_with_sensors(&mut self, disk_history: bool) -> Result<Snapshot> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let s = self.snapshot(disk_history, true)?;
            if !s.fans.sensors.is_empty() || Instant::now() >= deadline {
                return Ok(s);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Send a change and say how it went.
    fn act(&mut self, request: Request) -> Result<()> {
        let message = match self.send(request)? {
            Response::Ok { message } => message,
            other => return Err(unexpected(other)),
        };
        self.done(message.as_deref().unwrap_or("Done"));
        Ok(())
    }

    fn done(&self, message: &str) {
        if self.json {
            println!("{}", serde_json::json!({ "ok": true, "message": message }));
        } else {
            println!(
                "{} {}",
                self.paint.paint(Color::Ok, "✓"),
                term::clean(message)
            );
        }
    }

    fn view(&self) -> view::Ctx {
        view::Ctx {
            p: self.paint,
            width: term::size().0,
            tick: 0,
        }
    }

    fn print(&self, lines: Vec<String>) {
        let mut out = std::io::stdout().lock();
        for l in lines {
            let _ = writeln!(out, "{l}");
        }
    }

    fn print_json(&self, value: impl serde::Serialize) {
        match serde_json::to_string_pretty(&value) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("couldn't write JSON: {e}"),
        }
    }
}

/// A snapshot for drawing as text: every name and message made safe to
/// print ([`term::clean`]), since preset, app, sensor and interface names
/// come from outside and could hold escape sequences. Only for the screen:
/// JSON output and the names sent back in requests use the snapshot as the
/// app sent it.
fn display(mut s: Snapshot) -> Snapshot {
    clean_snapshot(&mut s);
    s
}

fn clean_snapshot(s: &mut Snapshot) {
    fn c(s: &mut String) {
        if s.chars().any(char::is_control) {
            *s = term::clean(s);
        }
    }
    if let Some(l) = &mut s.network.link {
        c(&mut l.name);
    }
    match &mut s.speed_test {
        SpeedTestState::Done {
            interface, summary, ..
        } => {
            interface.as_mut().map(c);
            c(summary);
        }
        SpeedTestState::Failed { error } => c(error),
        _ => {}
    }
    if let Some(a) = &mut s.awake.session {
        c(&mut a.label);
        c(&mut a.status);
        c(&mut a.activation);
        a.subject.as_mut().map(c);
    }
    let f = &mut s.fans;
    c(&mut f.helper_detail);
    c(&mut f.active_label);
    f.error.as_mut().map(c);
    if let Some(o) = &mut f.overheat {
        c(&mut o.sensor);
    }
    for p in &mut f.presets {
        c(&mut p.name);
    }
    for fan in &mut f.fans {
        c(&mut fan.name);
        c(&mut fan.control_label);
        fan.issue.as_mut().map(c);
    }
    for x in &mut f.sensors {
        c(&mut x.id);
        c(&mut x.name);
        c(&mut x.group);
    }
    for p in &mut s.problems {
        c(&mut p.title);
        c(&mut p.detail);
    }
    for x in &mut s.health.checks {
        c(&mut x.name);
        c(&mut x.detail);
    }
    c(&mut s.settings.menu_temp_sensor);
}

fn unexpected(r: Response) -> Fail {
    Fail::Message(format!("unexpected answer from Utilify: {r:?}"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut ctx = Ctx {
        paint: Paint::detect(cli.no_color || cli.json),
        json: cli.json,
        client: None,
    };
    let command = cli.command.unwrap_or(Command::Status { watch: false });
    match run(command, &mut ctx) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail::Message(m)) => {
            if ctx.json {
                println!("{}", serde_json::json!({ "ok": false, "error": m }));
            } else {
                eprintln!(
                    "{} {}",
                    ctx.paint.paint(Color::Danger, "error:"),
                    term::clean(&m)
                );
            }
            ExitCode::from(1)
        }
        Err(Fail::Shown) => ExitCode::from(1),
        Err(Fail::NotRunning) => {
            let m = format!(
                "{} isn't running. Start it with `utilifyctl launch`.",
                utilify_core::PRODUCT_NAME
            );
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": false, "error": m, "running": false })
                );
            } else {
                eprintln!("{} {m}", ctx.paint.paint(Color::Danger, "error:"));
            }
            ExitCode::from(2)
        }
    }
}

fn run(command: Command, ctx: &mut Ctx) -> Result<()> {
    match command {
        Command::Status { watch } => show(ctx, Tab::Overview, watch),
        Command::Watch { tab } => {
            if ctx.json {
                return Err(Fail::Message(
                    "the live view has no JSON form; use `status --json` (or a tab's command) and poll"
                        .into(),
                ));
            }
            live(ctx, tab)
        }
        Command::Network { watch } => show(ctx, Tab::Network, watch),
        Command::Storage { watch } => show(ctx, Tab::Storage, watch),
        Command::Speedtest { action, no_wait } => speedtest(ctx, action, no_wait),
        Command::Awake { action, watch } => awake(ctx, action, watch),
        Command::Fans { action, watch } => fans(ctx, action, watch),
        Command::Sensors => {
            let s = ctx.snapshot_with_sensors(false)?;
            if ctx.json {
                ctx.print_json(&s.fans.sensors);
            } else {
                ctx.print(view::sensors(&ctx.view(), &display(s)));
            }
            Ok(())
        }
        Command::Settings { action } => settings_cmd(ctx, action.unwrap_or(SettingsAction::List)),
        Command::Health { action } => health(ctx, action),
        Command::Problems { action } => match action {
            None => {
                let s = ctx.snapshot(false, false)?;
                if ctx.json {
                    ctx.print_json(&s.problems);
                } else if s.problems.is_empty() {
                    println!("No problems.");
                } else {
                    for p in &display(s).problems {
                        println!(
                            "{}  {}\n   {}\n   {}",
                            p.title,
                            ctx.paint.dim(format!("[{}]", p.id)),
                            p.detail,
                            ctx.paint.dim(if p.dismissable {
                                "dismiss with `utilifyctl problems dismiss <id>`"
                            } else {
                                "clears by itself once fixed"
                            })
                        );
                    }
                }
                Ok(())
            }
            Some(ProblemsAction::Dismiss { id }) => ctx.act(Request::DismissProblem { id }),
        },
        Command::Snooze { kind } => {
            let tag = kind.trim().to_ascii_lowercase();
            if utilify_core::alerts::AlertKind::from_tag(&tag).is_none() {
                let tags: Vec<&str> = utilify_core::alerts::AlertKind::ALL
                    .iter()
                    .map(|k| k.tag())
                    .collect();
                return Err(Fail::Message(format!(
                    "no notification called \"{kind}\"; use one of {}",
                    tags.join(", ")
                )));
            }
            ctx.act(Request::SnoozeAlert { tag })
        }
        Command::Login { state } => match state {
            None => {
                let s = ctx.snapshot(false, false)?;
                if ctx.json {
                    ctx.print_json(serde_json::json!({ "launch_at_login": s.launch_at_login }));
                } else {
                    println!(
                        "Launch at login: {}",
                        match s.launch_at_login {
                            Some(true) => "on",
                            Some(false) => "off",
                            None => "unavailable (needs macOS 13 and the bundled app)",
                        }
                    );
                }
                Ok(())
            }
            Some(state) => ctx.act(Request::SetLaunchAtLogin {
                on: matches!(state, OnOff::On),
            }),
        },
        Command::Open { tab } => {
            ensure_running(ctx)?;
            let tab = tab.map(|t| {
                match t {
                    DashboardTab::Overview => "overview",
                    DashboardTab::Network => "network",
                    DashboardTab::Storage => "storage",
                    DashboardTab::Awake => "awake",
                    DashboardTab::Fans => "fans",
                    DashboardTab::Settings => "settings",
                }
                .to_string()
            });
            ctx.send(Request::OpenDashboard { tab })?;
            ctx.done("Opened the dashboard");
            Ok(())
        }
        Command::Workbench {
            action: WorkbenchAction::Open,
        } => {
            ensure_running(ctx)?;
            ctx.send(Request::OpenWorkbench)?;
            ctx.done("Opened the workbench");
            Ok(())
        }
        Command::Launch => {
            if Client::connect().is_ok() {
                ctx.done("Utilify is already running");
                return Ok(());
            }
            ensure_running(ctx)?;
            ctx.done("Utilify is running");
            Ok(())
        }
        Command::Quit => match ctx.act(Request::Quit) {
            Err(Fail::NotRunning) => {
                ctx.done("Utilify wasn't running");
                Ok(())
            }
            other => other,
        },
    }
}

/// Start the app if it isn't running and wait for its socket.
fn ensure_running(ctx: &mut Ctx) -> Result<()> {
    if ctx.client().is_ok() {
        return Ok(());
    }
    let opened = std::process::Command::new("/usr/bin/open")
        .args(["-g", "-b", utilify_core::APP_IDENTIFIER])
        .status()
        .is_ok_and(|s| s.success())
        || std::process::Command::new("/usr/bin/open")
            .args(["-g", "-a", utilify_core::PRODUCT_NAME])
            .status()
            .is_ok_and(|s| s.success());
    if !opened {
        return Err(Fail::Message(
            "couldn't find Utilify.app; install it with ./install.sh".into(),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(c) = Client::connect() {
            ctx.client = Some(c);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(Fail::Message(
        "Utilify started but isn't answering; it may be an older version without the CLI".into(),
    ))
}

/// One tab, printed once or kept live.
fn show(ctx: &mut Ctx, tab: Tab, watch: bool) -> Result<()> {
    if watch && !ctx.json {
        return live(ctx, tab);
    }
    let s = match tab {
        Tab::Fans => ctx.snapshot_with_sensors(false)?,
        _ => ctx.snapshot(tab == Tab::Storage, false)?,
    };
    if ctx.json {
        match tab {
            Tab::Overview => ctx.print_json(&s),
            Tab::Network => ctx.print_json(serde_json::json!({
                "network": s.network, "speed_test": s.speed_test
            })),
            Tab::Storage => ctx.print_json(&s.storage),
            Tab::Awake => ctx.print_json(serde_json::json!({
                "awake": s.awake,
                "allow_display_sleep": s.settings.awake_allow_display_sleep,
                "allow_closed_display_sleep": s.settings.awake_allow_closed_display_sleep,
                "screen_saver": s.settings.awake_screen_saver,
                "screen_saver_mins": s.settings.awake_screen_saver_mins,
                "on_launch": s.settings.awake_on_launch,
                "notify_end": s.settings.awake_notify_end,
                "launch_at_login": s.launch_at_login,
            })),
            Tab::Fans => ctx.print_json(&s.fans),
            Tab::Health => ctx.print_json(serde_json::json!({
                "health": s.health, "problems": s.problems
            })),
        }
        return Ok(());
    }
    ctx.print(render(tab, &ctx.view(), &display(s)));
    Ok(())
}

fn render(tab: Tab, c: &view::Ctx, s: &Snapshot) -> Vec<String> {
    match tab {
        Tab::Overview => view::overview(c, s),
        Tab::Network => view::network(c, s),
        Tab::Storage => view::storage(c, s),
        Tab::Awake => view::awake(c, s),
        Tab::Fans => view::fans(c, s),
        Tab::Health => view::health(c, s),
    }
}

/// The live view: a tab bar, the tab, and a footer, redrawn at the app's
/// refresh rate over one open connection.
fn live(ctx: &mut Ctx, mut tab: Tab) -> Result<()> {
    if !term::is_tty(libc::STDOUT_FILENO) {
        return Err(Fail::Message("the live view needs a terminal".into()));
    }
    // Fail before taking over the screen if the app isn't there.
    ctx.client()?;
    let screen = term::Screen::enter();
    let mut tick = 0u64;
    let mut disk: Option<(Instant, Vec<utilify_core::disk_history::DiskSample>)> = None;
    let mut last: Option<Snapshot> = None;
    let mut fetched: Option<Instant> = None;
    let mut lost_since: Option<Instant> = None;
    loop {
        if term::stopped() {
            break;
        }
        // Back from Ctrl-Z: the screen was cleared, so redraw now.
        screen.job_control();
        let interval = last
            .as_ref()
            .map_or(Duration::from_secs(1), |s| s.settings.interval());
        if fetched.is_none_or(|t| t.elapsed() >= interval) {
            fetched = Some(Instant::now());
            // Disk history is big and changes every 15 minutes: fetch it
            // once a minute, only for the Storage tab.
            let want_disk = tab == Tab::Storage
                && disk
                    .as_ref()
                    .is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(60));
            match fetch(ctx, &screen, tab == Tab::Fans, want_disk) {
                Ok(mut s) => {
                    lost_since = None;
                    if want_disk {
                        disk = Some((Instant::now(), std::mem::take(&mut s.storage.history)));
                    }
                    if let Some((_, h)) = &disk {
                        s.storage.history = h.clone();
                    }
                    last = Some(display(s));
                }
                Err(_) => {
                    ctx.client = None;
                    lost_since.get_or_insert_with(Instant::now);
                }
            }
        }
        let (cols, rows) = term::size();
        let c = view::Ctx {
            p: ctx.paint,
            width: cols,
            tick,
        };
        let mut lines = vec![tab_bar(&c, tab), String::new()];
        match (&last, lost_since) {
            (Some(s), None) => lines.extend(render(tab, &c, s)),
            _ => lines.push(format!(
                "  {} Utilify isn't answering; waiting for it to come back…",
                ctx.paint
                    .paint(Color::Warn, chart::spinner(tick).to_string())
            )),
        }
        // Footer on the last row.
        let footer = ctx.paint.dim(format!(
            "1–6 or ←/→ switch tabs · r refresh · q quit · every {}",
            utilify_core::format::interval(interval.as_millis() as u64)
        ));
        let room = rows.saturating_sub(1);
        lines.truncate(room);
        lines.resize(room, String::new());
        lines.push(footer);
        screen.draw(&lines);
        tick += 1;

        // Spinners need frames between fetches.
        let animating = lost_since.is_some()
            || last.as_ref().is_some_and(|s| {
                matches!(s.speed_test, SpeedTestState::Running { .. }) || s.health.running
            });
        let wait = if animating {
            Duration::from_millis(120)
        } else {
            interval.saturating_sub(fetched.map_or(Duration::ZERO, |t| t.elapsed()))
        };
        if let Some(key) = screen.key(wait) {
            match key {
                b'q' | b'Q' | 0x1b | 0x04 => break,
                b'1'..=b'6' => tab = Tab::ALL[(key - b'1') as usize],
                b'\t' | b'l' => tab = Tab::ALL[(tab.index() + 1) % Tab::ALL.len()],
                term::KEY_BACK | b'h' => {
                    tab = Tab::ALL[(tab.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
                }
                b'r' | b'R' => {}
                _ => continue,
            }
            // A new tab may need other data: fetch now.
            fetched = None;
        }
        term::take_resized();
    }
    drop(screen);
    Ok(())
}

/// One snapshot for the live view. While it waits, Ctrl-C ends the view and
/// Ctrl-Z suspends it, as they do between frames.
fn fetch(ctx: &mut Ctx, screen: &term::Screen, sensors: bool, disk: bool) -> Result<Snapshot> {
    if ctx.client.is_none() {
        ctx.client = Some(Client::connect()?);
    }
    let cancelled = || {
        screen.job_control();
        term::stopped()
    };
    match ctx.send_until(
        Request::Snapshot {
            disk_history: disk,
            sensors,
        },
        &cancelled,
    )? {
        Response::Snapshot { snapshot } => Ok(*snapshot),
        other => Err(unexpected(other)),
    }
}

fn tab_bar(c: &view::Ctx, active: Tab) -> String {
    let tabs: Vec<String> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let n = c.p.dim((i + 1).to_string());
            if *t == active {
                format!("{n} {}", c.p.paint(Color::Accent, c.p.bold(t.label())))
            } else {
                format!("{n} {}", t.label())
            }
        })
        .collect();
    format!("{}  {}", c.p.bold("utilify"), tabs.join("   "))
}

fn speedtest(ctx: &mut Ctx, action: Option<SpeedtestAction>, no_wait: bool) -> Result<()> {
    match action.unwrap_or(SpeedtestAction::Start) {
        SpeedtestAction::Cancel => ctx.act(Request::SpeedTestCancel),
        SpeedtestAction::Copy => ctx.act(Request::SpeedTestCopy),
        SpeedtestAction::Status => {
            let s = ctx.snapshot(false, false)?;
            if ctx.json {
                ctx.print_json(&s.speed_test);
            } else {
                ctx.print(view::speed_test(&ctx.view(), &display(s)));
            }
            Ok(())
        }
        SpeedtestAction::Start => {
            if no_wait {
                return match ctx.send(Request::SpeedTestStart)? {
                    Response::SpeedTest { message, .. } => {
                        ctx.done(&message);
                        Ok(())
                    }
                    other => Err(unexpected(other)),
                };
            }
            // Catch Ctrl-C from here on, so even one while the start is
            // being acknowledged stops the test rather than abandoning it.
            term::catch_stop();
            let started = ctx.send(Request::SpeedTestStart);
            if term::stopped() {
                return Err(cancel_speed_test(ctx));
            }
            match started? {
                Response::SpeedTest { run, .. } => wait_for_speed_test(ctx, run),
                other => Err(unexpected(other)),
            }
        }
    }
}

/// After Ctrl-C: ask Utilify, on a fresh connection (the current one may
/// have an answer pending), to stop the test, and say what came of it.
fn cancel_speed_test(ctx: &mut Ctx) -> Fail {
    if term::is_tty(libc::STDOUT_FILENO) && !ctx.json {
        print!("\r\x1b[K");
    }
    ctx.client = None;
    match Client::connect().and_then(|mut c| c.request(Request::SpeedTestCancel)) {
        Ok(Response::Ok { message }) => Fail::Message(format!(
            "interrupted ({})",
            term::clean(message.as_deref().unwrap_or("speed test cancelled")).to_lowercase()
        )),
        Ok(Response::Error { message }) => Fail::Message(format!(
            "interrupted, but Utilify didn't stop the speed test: {}",
            term::clean(&message)
        )),
        _ => Fail::Message(
            "interrupted, but Utilify couldn't be reached to stop the speed test; \
             try `utilifyctl speedtest cancel`"
                .into(),
        ),
    }
}

/// Follow the test on one line until it's done; Ctrl-C cancels it.
/// `run` is the test's number, from the start's answer: a cancel puts an
/// older run back on show, which this tells apart from the result.
fn wait_for_speed_test(ctx: &mut Ctx, run: u64) -> Result<()> {
    term::catch_stop();
    let tty = term::is_tty(libc::STDOUT_FILENO) && !ctx.json;
    let mut tick = 0;
    loop {
        // Ctrl-C, before or while asking how it's going: stop the test on
        // a fresh connection (the current one may have an answer pending).
        let polled = if term::stopped() {
            Err(Fail::Message("cancelled".into()))
        } else {
            ctx.snapshot(false, false)
        };
        if term::stopped() {
            return Err(cancel_speed_test(ctx));
        }
        let raw = polled?;
        let id = run;
        // Anything but this run, running or finished, means it was
        // cancelled (from the dashboard or another utilifyctl), which puts
        // the previous result back.
        if raw.speed_test_run != id || matches!(raw.speed_test, SpeedTestState::Idle) {
            if tty {
                print!("\r\x1b[K");
            }
            return Err(Fail::Message(
                "the speed test was cancelled before it finished".into(),
            ));
        }
        let s = display(raw.clone());
        match &s.speed_test {
            SpeedTestState::Running { .. } => {
                if tty {
                    let c = view::Ctx {
                        p: ctx.paint,
                        width: term::size().0,
                        tick,
                    };
                    let line = view::speed_test(&c, &s).get(1).cloned().unwrap_or_default();
                    let rates = view::speed_test(&c, &s).get(2).cloned().unwrap_or_default();
                    let whole = format!(
                        "{}{}",
                        line.trim_start(),
                        ctx.paint.dim(format!("  {}", rates.trim()))
                    );
                    // One row, never wrapped, so \r can redraw it.
                    print!("\r\x1b[K{}", term::fit(&whole, c.width.saturating_sub(1)));
                    let _ = std::io::stdout().flush();
                }
                tick += 1;
                std::thread::sleep(Duration::from_millis(120));
            }
            state => {
                if tty {
                    print!("\r\x1b[K");
                }
                if ctx.json {
                    // The data as Utilify sent it; cleaning is for the screen.
                    ctx.print_json(&raw.speed_test);
                } else {
                    ctx.print(view::speed_test(&ctx.view(), &s));
                }
                // A failure has been shown (or printed as the one JSON
                // document): exit 1 without saying it twice.
                return match state {
                    SpeedTestState::Failed { .. } => Err(Fail::Shown),
                    _ => Ok(()),
                };
            }
        }
    }
}

fn awake(ctx: &mut Ctx, action: Option<AwakeAction>, watch: bool) -> Result<()> {
    match action {
        None => show(ctx, Tab::Awake, watch),
        Some(AwakeAction::Start { mode }) => {
            // Look the apps up first when they're needed, so "Utilify isn't
            // running" keeps its own exit status.
            let wants_apps = mode
                .first()
                .is_some_and(|w| matches!(w.to_ascii_lowercase().as_str(), "app" | "while-app"));
            let apps = if wants_apps {
                Some(running_apps(ctx)?)
            } else {
                None
            };
            let mode = parse::awake_mode(&mode, || {
                apps.ok_or_else(|| "couldn't list the running apps".to_string())
            })?;
            ctx.act(Request::AwakeStart { mode })
        }
        Some(AwakeAction::Stop) => ctx.act(Request::AwakeStop),
        Some(AwakeAction::Extend { by }) => ctx.act(Request::AwakeExtend {
            minutes: parse::minutes(&by)?,
        }),
        Some(AwakeAction::Apps) => {
            let apps = running_apps(ctx)?;
            if ctx.json {
                ctx.print_json(&apps);
            } else {
                for a in apps {
                    println!(
                        "{}  {}",
                        term::pad_left(&a.pid.to_string(), 7),
                        term::clean(&a.name)
                    );
                }
            }
            Ok(())
        }
    }
}

fn running_apps(ctx: &mut Ctx) -> Result<Vec<ipc::RunningApp>> {
    match ctx.send(Request::RunningApps)? {
        Response::Apps { apps } => Ok(apps),
        other => Err(unexpected(other)),
    }
}

fn fans(ctx: &mut Ctx, action: Option<FansAction>, watch: bool) -> Result<()> {
    match action {
        None => show(ctx, Tab::Fans, watch),
        Some(FansAction::Use { preset }) => {
            let names: Vec<String> = ctx
                .snapshot(false, false)?
                .fans
                .presets
                .into_iter()
                .map(|p| p.name)
                .collect();
            ctx.act(Request::FanActivate {
                active: parse::preset(&preset, &names)?,
            })
        }
        Some(FansAction::Set { fan, control }) => {
            let wants_sensor = control
                .first()
                .is_some_and(|w| matches!(w.to_ascii_lowercase().as_str(), "curve" | "sensor"));
            let s = if wants_sensor {
                ctx.snapshot_with_sensors(false)?
            } else {
                ctx.snapshot(false, false)?
            };
            if s.fans.fans.is_empty() {
                return Err(Fail::Message("this Mac has no fans Utilify can see".into()));
            }
            let index = parse::fan(&fan, &s.fans.fans)?;
            let view = s
                .fans
                .fans
                .iter()
                .find(|f| f.index == index)
                .expect("found by parse::fan");
            let control: Control = parse::control(&control, view, &s.fans.sensors)?;
            ctx.act(Request::FanSetControl {
                fan: index,
                control,
            })
        }
        Some(FansAction::Save { name }) => ctx.act(Request::FanSavePreset { name }),
        Some(FansAction::Rename { from, to }) => ctx.act(Request::FanRenamePreset { from, to }),
        Some(FansAction::Delete { name }) => ctx.act(Request::FanDeletePreset { name }),
    }
}

fn health(ctx: &mut Ctx, action: Option<HealthAction>) -> Result<()> {
    if let Some(HealthAction::Fix { id }) = action {
        return ctx.act(Request::HealthFix { id });
    }
    if let Some(HealthAction::Run) = action {
        ctx.send(Request::RunHealth)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        // Wait for the run to start and finish.
        std::thread::sleep(Duration::from_millis(100));
        while ctx.snapshot(false, false)?.health.running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    show(ctx, Tab::Health, false)
}

/// Settings go through the app when it's running, so the UI follows; when
/// it isn't, straight to the file it reads at launch.
fn settings_cmd(ctx: &mut Ctx, action: SettingsAction) -> Result<()> {
    let running = ctx.client().is_ok();
    match action {
        SettingsAction::List => {
            let (s, sensors) = if running {
                let mut snap = ctx.snapshot(false, false)?;
                // Name the menu-bar sensor, which needs a reading.
                if snap.settings.menu_temp_sensor != utilify_core::sensors::CPU_AVERAGE_ID {
                    snap = ctx.snapshot_with_sensors(false)?;
                }
                (snap.settings, snap.fans.sensors)
            } else {
                offline_note(ctx);
                (load_settings(), Vec::new())
            };
            if ctx.json {
                ctx.print_json(&s);
            } else {
                let mut s = s;
                s.menu_temp_sensor = term::clean(&s.menu_temp_sensor);
                let sensors: Vec<_> = sensors
                    .into_iter()
                    .map(|mut x| {
                        x.id = term::clean(&x.id);
                        x.name = term::clean(&x.name);
                        x
                    })
                    .collect();
                ctx.print(view::settings(&ctx.view(), &s, &sensors));
            }
            Ok(())
        }
        SettingsAction::Get { key } => {
            let s = if running {
                ctx.snapshot(false, false)?.settings
            } else {
                load_settings()
            };
            let field =
                settings::find_field(&key).ok_or_else(|| Fail::Message(no_such_setting(&key)))?;
            let value = s.get_field(field.key).unwrap_or_default();
            if ctx.json {
                ctx.print_json(serde_json::json!({ "key": field.key, "value": value }));
            } else {
                println!("{}", term::clean(&value));
            }
            Ok(())
        }
        SettingsAction::Set { key, value } => {
            if settings::find_field(&key).is_none() {
                return Err(Fail::Message(no_such_setting(&key)));
            }
            // Closed: edit the file, unless Utilify started meanwhile.
            // A sensor is named as the app would take it: read the sensors
            // here (no app needed) to turn a name into its id and check it.
            let offline_value = if !running
                && settings::find_field(&key).is_some_and(|f| f.kind == settings::FieldKind::Sensor)
            {
                let readings = local_sensors();
                let id = parse::sensor_id(&value, &readings)?;
                let known = readings.is_empty()
                    || id == utilify_core::sensors::CPU_AVERAGE_ID
                    || readings.iter().any(|s| s.id == id);
                if !known {
                    return Err(Fail::Message(format!(
                        "no sensor called \"{}\" (`utilifyctl sensors` lists them)",
                        term::clean(&value)
                    )));
                }
                id
            } else {
                value.clone()
            };
            if !running
                && let Some(stored) =
                    edit_settings_file(|s| s.set_field(&key, &offline_value, |_| true))?
            {
                offline_note(ctx);
                let field = settings::find_field(&key).map_or(key.as_str(), |f| f.key);
                ctx.done(&format!("{field} = {stored}"));
                return Ok(());
            }
            {
                // Read the sensors first, so a sensor id can be checked.
                if settings::find_field(&key).is_some_and(|f| f.kind == settings::FieldKind::Sensor)
                {
                    let s = ctx.snapshot_with_sensors(false)?;
                    let id = parse::sensor_id(&value, &s.fans.sensors)?;
                    return ctx.act(Request::SetSetting { key, value: id });
                }
                ctx.act(Request::SetSetting { key, value })
            }
        }
        SettingsAction::Reset { yes } => {
            if !yes {
                return Err(Fail::Message(
                    "this puts every setting back to its default; run it again with --yes".into(),
                ));
            }
            if !running
                && edit_settings_file(|s| {
                    *s = Settings::default();
                    Ok(())
                })?
                .is_some()
            {
                offline_note(ctx);
                ctx.done("Settings reset to their defaults");
                return Ok(());
            }
            ctx.act(Request::ResetSettings)
        }
    }
}

/// Change `settings.json` while the app is closed: load, edit and save
/// under the settings lock ([`settings::lock`]), so two of these at once
/// don't lose a change. `None` when Utilify turned out to be running once
/// the lock was held: the change should go through it instead.
fn edit_settings_file<T>(
    edit: impl FnOnce(&mut Settings) -> std::result::Result<T, String>,
) -> Result<Option<T>> {
    let path = settings::default_path()
        .ok_or_else(|| Fail::Message("no data folder to keep settings in".into()))?;
    let lock = settings::lock(&path)
        .map_err(|e| Fail::Message(format!("can't lock the settings: {e}")))?;
    // The app holds the lock while it starts; once it's ours, either it
    // hasn't read the file yet, or it's answering on its socket.
    if Client::connect().is_ok() {
        return Ok(None);
    }
    let mut s = settings::load(&path);
    let out = edit(&mut s)?;
    settings::save(&path, &s).map_err(|e| Fail::Message(format!("saving failed: {e}")))?;
    drop(lock);
    Ok(Some(out))
}

/// The temperature sensors, read straight from the hardware, for when the
/// app isn't running to read them.
fn local_sensors() -> Vec<ipc::SensorView> {
    utilify_core::sensors::read_all()
        .into_iter()
        .map(|s| ipc::SensorView {
            id: s.id,
            name: s.name,
            group: s.group.label().into(),
            celsius: s.celsius,
        })
        .collect()
}

fn load_settings() -> Settings {
    settings::default_path()
        .as_deref()
        .map(settings::load)
        .unwrap_or_default()
}

fn offline_note(ctx: &Ctx) {
    if !ctx.json {
        eprintln!(
            "{}",
            ctx.paint
                .dim("Utilify isn't running: using settings.json, which it reads at launch.")
        );
    }
}

fn no_such_setting(key: &str) -> String {
    format!("no setting called \"{key}\" (`utilifyctl settings` lists them)")
}
