//! How `utilifyctl` talks to the running app: newline-delimited JSON over a
//! Unix socket at `<data dir>/dev.utilify.app/cli.sock`. Each line the
//! client sends is an [`Envelope`]; the app answers each with one
//! [`Response`] line. A connection can carry any number of requests, so a
//! live view keeps one open and polls [`Request::Snapshot`].
//!
//! The app applies every change through the same code paths as its menu and
//! dashboard, so whatever the CLI does shows up in the UI straight away.
//!
//! The socket is created mode 0600 in the user's own data directory, and the
//! server also checks the peer's uid, so only the same user can connect.

use crate::disk_history::DiskSample;
use crate::fans::{Active, Control, Preset};
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Bumped when a request or response changes incompatibly.
pub const PROTOCOL: u32 = 1;

/// Requests longer than this are refused, so a stray client can't make the
/// app buffer without limit.
const MAX_LINE: usize = 64 * 1024;

/// Longest socket path the OS takes (`sun_path`, with its NUL).
const SUN_PATH_MAX: usize = 104;

/// `<data dir>/dev.utilify.app/cli.sock`, or, when that's too long for a
/// socket (a very long home folder), the same name in the per-user temporary
/// folder, which only the user can open. Both ends pick the same one.
pub fn socket_path() -> Option<PathBuf> {
    let preferred = dirs::data_dir()?
        .join(crate::APP_IDENTIFIER)
        .join("cli.sock");
    if preferred.as_os_str().len() < SUN_PATH_MAX {
        return Some(preferred);
    }
    let short = user_temp_dir()?.join(format!("{}.cli.sock", crate::APP_IDENTIFIER));
    (short.as_os_str().len() < SUN_PATH_MAX).then_some(short)
}

/// The per-user temporary folder (`/var/folders/…/T/`), mode 0700.
#[cfg(target_os = "macos")]
fn user_temp_dir() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0u8; 1024];
    // SAFETY: confstr writes at most `buf.len()` bytes, NUL included.
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if n == 0 || n > buf.len() {
        return None;
    }
    buf.truncate(n - 1);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

#[cfg(not(target_os = "macos"))]
fn user_temp_dir() -> Option<PathBuf> {
    None
}

/// One request line.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Everything the dashboard shows. The disk history (up to 30 days of
    /// samples) and the temperature sensors (which the app only reads while
    /// something shows them) are opt-in.
    Snapshot {
        #[serde(default)]
        disk_history: bool,
        #[serde(default)]
        sensors: bool,
    },
    /// Change one setting, by its key in `settings.json`
    /// ([`crate::settings::FIELDS`]).
    SetSetting {
        key: String,
        value: String,
    },
    ResetSettings,
    AwakeStart {
        mode: AwakeMode,
    },
    AwakeStop,
    /// Add time to a timed session.
    AwakeExtend {
        minutes: u32,
    },
    /// The apps "While an app runs" can follow.
    RunningApps,
    /// Switch fans to Automatic, Full blast, Custom or a user preset.
    FanActivate {
        active: Active,
    },
    /// One fan's control, by its SMC index, as in the fan table.
    FanSetControl {
        fan: u8,
        control: Control,
    },
    FanSavePreset {
        name: String,
    },
    FanRenamePreset {
        from: String,
        to: String,
    },
    FanDeletePreset {
        name: String,
    },
    SpeedTestStart,
    SpeedTestCancel,
    /// Put the last result on the clipboard, as the Network tab's Copy does.
    SpeedTestCopy,
    RunHealth,
    /// Carry out a health check's suggested fix (`fix` in [`CheckView`]).
    HealthFix {
        id: String,
    },
    DismissProblem {
        id: String,
    },
    /// Silence one kind of notification for an hour, by its tag.
    SnoozeAlert {
        tag: String,
    },
    SetLaunchAtLogin {
        on: bool,
    },
    /// Open the dashboard, on a tab by name.
    OpenDashboard {
        tab: Option<String>,
    },
    OpenWorkbench,
    Quit,
}

/// A keep-awake mode, as the CLI asks for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AwakeMode {
    Indefinite,
    For {
        minutes: u32,
    },
    /// Until a local time of day, in minutes after midnight.
    Until {
        minute_of_day: u16,
    },
    WhileApp {
        pid: i32,
    },
    WhileFileDownloading,
    WhileNetworkBusy,
}

impl AwakeMode {
    pub fn from_mode(mode: crate::awake::Mode) -> Self {
        use crate::awake::Mode;
        match mode {
            Mode::Indefinite => AwakeMode::Indefinite,
            Mode::For(d) => AwakeMode::For {
                minutes: (d.as_secs() / 60) as u32,
            },
            Mode::Until(m) => AwakeMode::Until { minute_of_day: m },
            Mode::WhileApp(pid) => AwakeMode::WhileApp { pid },
            Mode::WhileFileDownloading => AwakeMode::WhileFileDownloading,
            Mode::WhileNetworkBusy => AwakeMode::WhileNetworkBusy,
        }
    }

    pub fn to_mode(self) -> crate::awake::Mode {
        use crate::awake::Mode;
        match self {
            AwakeMode::Indefinite => Mode::Indefinite,
            AwakeMode::For { minutes } => Mode::For(Duration::from_secs(minutes as u64 * 60)),
            AwakeMode::Until { minute_of_day } => Mode::Until(minute_of_day % 1440),
            AwakeMode::WhileApp { pid } => Mode::WhileApp(pid),
            AwakeMode::WhileFileDownloading => Mode::WhileFileDownloading,
            AwakeMode::WhileNetworkBusy => Mode::WhileNetworkBusy,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Snapshot {
        snapshot: Box<Snapshot>,
    },
    Apps {
        apps: Vec<RunningApp>,
    },
    /// A speed test is running: the one just started, or one already going.
    /// `run` is the number [`Snapshot::speed_test_run`] shows for it.
    SpeedTest {
        run: u64,
        message: String,
    },
    Error {
        message: String,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok { message: None }
    }

    pub fn done(message: impl Into<String>) -> Self {
        Response::Ok {
            message: Some(message.into()),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Response::Error {
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningApp {
    pub pid: i32,
    pub name: String,
}

/// What the dashboard shows, in one serializable piece.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub app_version: String,
    pub network: Network,
    pub storage: Storage,
    pub speed_test: SpeedTestState,
    /// Which run `speed_test` is about: each start gets the next number, and
    /// a cancel goes back to the run shown before (0 for none).
    pub speed_test_run: u64,
    pub awake: Awake,
    pub fans: Fans,
    pub power: Power,
    /// `nominal`, `fair`, `serious` or `critical`.
    pub thermal: String,
    pub problems: Vec<ProblemView>,
    pub health: HealthReport,
    pub settings: Settings,
    /// `None` where launch at login isn't available (before macOS 13, or
    /// unbundled).
    pub launch_at_login: Option<bool>,
    pub dashboard_open: bool,
    pub workbench_open: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Network {
    /// Bytes per second.
    pub down: f64,
    pub up: f64,
    pub link: Option<LinkView>,
    /// Share of the link in use by the busier direction, 0..=1.
    pub utilization: Option<f64>,
    /// Bytes since the app started.
    pub session_down: u64,
    pub session_up: u64,
    /// Rate samples (down, up), oldest first, one per refresh.
    pub history: Vec<[f64; 2]>,
    pub history_capacity: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkView {
    pub name: String,
    /// Bits per second; 0 when the driver doesn't say.
    pub capacity_bps: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Storage {
    pub disk: Option<DiskView>,
    /// `ok`, `warn` or `danger`, by the bar colour settings.
    pub level: Option<String>,
    /// Stored samples plus a live one; empty unless asked for.
    pub history: Vec<DiskSample>,
    /// The change in used space across the stored history, when there are
    /// two samples to compare: (bytes, over seconds).
    pub change: Option<(i64, u64)>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DiskView {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SpeedTestState {
    Idle,
    Running {
        elapsed_secs: f64,
    },
    Done {
        down_bps: u64,
        up_bps: u64,
        idle_latency_ms: Option<f64>,
        responsiveness_rpm: Option<f64>,
        interface: Option<String>,
        ago_secs: u64,
        /// What Copy puts on the clipboard.
        summary: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Awake {
    pub session: Option<AwakeSession>,
    /// All the time kept awake, across launches.
    pub total_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AwakeSession {
    pub mode: AwakeMode,
    /// `Indefinitely`, `2 hours`, `Until 18:30`.
    pub label: String,
    /// `Indefinitely`, `42:10 left`, `While Safari runs`.
    pub status: String,
    /// `Manual Activation` or `Started at Launch`.
    pub activation: String,
    pub subject: Option<String>,
    pub elapsed_secs: u64,
    /// For timed sessions.
    pub remaining_secs: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fans {
    /// Whether this Mac's fans can be set at all.
    pub control_supported: bool,
    /// `unknown`, `unsupported`, `not_installed`, `not_running`,
    /// `permission_denied`, `ready` or `broken`.
    pub helper: String,
    /// The helper's state in words.
    pub helper_detail: String,
    pub ready: bool,
    pub error: Option<String>,
    /// Another tool is holding the fans.
    pub foreign: bool,
    /// Set while the overheating guard has the fans on automatic.
    pub overheat: Option<OverheatView>,
    pub taking_over: bool,
    /// Seconds since the last reading.
    pub read_ago_secs: Option<f64>,
    pub active: Active,
    pub active_label: String,
    pub presets: Vec<Preset>,
    pub fans: Vec<FanView>,
    /// Empty unless asked for (or something else shows them).
    pub sensors: Vec<SensorView>,
    pub cpu_average: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OverheatView {
    pub sensor: String,
    pub celsius: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FanView {
    pub index: u8,
    pub name: String,
    pub rpm: f32,
    pub min: f32,
    pub max: f32,
    /// Held at a fixed speed by some tool rather than the system.
    pub manual: bool,
    pub control: Control,
    /// `Auto`, `3000 rpm`, `CPU Core Average 50–85°C`.
    pub control_label: String,
    /// What Utilify asked the helper for; `None` while automatic.
    pub target: Option<f32>,
    /// Why the control isn't in effect.
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SensorView {
    pub id: String,
    pub name: String,
    pub group: String,
    pub celsius: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Power {
    pub on_battery: bool,
    pub percent: Option<u8>,
    pub charging: bool,
    pub minutes_to_empty: Option<u32>,
    pub has_battery: bool,
    pub low_power_mode: bool,
}

impl From<crate::power::PowerState> for Power {
    fn from(p: crate::power::PowerState) -> Self {
        Power {
            on_battery: p.on_battery,
            percent: p.percent,
            charging: p.charging,
            minutes_to_empty: p.minutes_to_empty,
            has_battery: p.has_battery,
            low_power_mode: p.low_power_mode,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl From<crate::health::Health> for Status {
    fn from(h: crate::health::Health) -> Self {
        use crate::health::Health;
        match h {
            Health::Ok => Status::Ok,
            Health::Warn => Status::Warn,
            Health::Fail => Status::Fail,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProblemView {
    pub id: String,
    pub status: Status,
    pub title: String,
    pub detail: String,
    pub dismissable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthReport {
    pub checks: Vec<CheckView>,
    /// Seconds since the checks last ran.
    pub ago_secs: Option<u64>,
    pub running: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckView {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub detail: String,
    /// A fix the app can carry out, such as `reveal-data-dir`.
    pub fix: Option<String>,
}

/// Why a request didn't get an answer.
#[derive(Debug)]
pub enum ClientError {
    /// Nothing is listening: the app isn't running (or is too old to have
    /// the socket).
    NotRunning,
    Io(std::io::Error),
    /// The answer didn't parse.
    Protocol(String),
    /// The caller gave up waiting (Ctrl-C).
    Cancelled,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotRunning => write!(f, "{} isn't running", crate::PRODUCT_NAME),
            ClientError::Io(e) => write!(f, "talking to {} failed: {e}", crate::PRODUCT_NAME),
            ClientError::Protocol(e) => write!(
                f,
                "{} sent an answer this version can't read ({e}); update both",
                crate::PRODUCT_NAME
            ),
            ClientError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for ClientError {}

/// How long a client waits for an answer. The app answers from its main
/// thread; a stall this long means it's hung.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);
/// How often a waiting client checks whether its caller gave up.
const POLL: Duration = Duration::from_millis(100);

/// A connection to the app, kept open across requests.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// An answer was left unread (timeout, cancel): the next one on this
    /// connection would be the wrong one, so it's not used again.
    broken: bool,
}

impl Client {
    pub fn connect() -> Result<Self, ClientError> {
        let path = socket_path().ok_or(ClientError::NotRunning)?;
        Self::connect_to(&path)
    }

    pub fn connect_to(path: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                ClientError::NotRunning
            }
            _ => ClientError::Io(e),
        })?;
        // Short reads, so a wait can be given up on (see `request_until`).
        stream
            .set_read_timeout(Some(POLL))
            .map_err(ClientError::Io)?;
        stream
            .set_write_timeout(Some(ANSWER_TIMEOUT))
            .map_err(ClientError::Io)?;
        let writer = stream.try_clone().map_err(ClientError::Io)?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
            broken: false,
        })
    }

    pub fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        self.request_until(request, &|| false)
    }

    /// [`Client::request`], giving up with [`ClientError::Cancelled`] as soon
    /// as `cancelled` says so (checked every 100 ms), so Ctrl-C needn't wait
    /// for a slow answer.
    pub fn request_until(
        &mut self,
        request: Request,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Response, ClientError> {
        if self.broken {
            return Err(ClientError::Io(std::io::Error::other(
                "the connection was abandoned mid-answer",
            )));
        }
        let mut line = serde_json::to_vec(&Envelope {
            v: PROTOCOL,
            request,
        })
        .map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push(b'\n');
        if let Err(e) = self.writer.write_all(&line) {
            self.broken = true;
            return Err(ClientError::Io(e));
        }
        let deadline = std::time::Instant::now() + ANSWER_TIMEOUT;
        // Bytes read before a timeout stay in `answer`; reading resumes.
        let mut answer = Vec::new();
        loop {
            match self.reader.read_until(b'\n', &mut answer) {
                Ok(_) if answer.ends_with(b"\n") => break,
                Ok(_) => {
                    self.broken = true;
                    return Err(ClientError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the connection closed",
                    )));
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    if cancelled() {
                        self.broken = true;
                        return Err(ClientError::Cancelled);
                    }
                    if std::time::Instant::now() >= deadline {
                        self.broken = true;
                        return Err(ClientError::Io(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "no answer in 20 s",
                        )));
                    }
                }
                Err(e) => {
                    self.broken = true;
                    return Err(ClientError::Io(e));
                }
            }
        }
        serde_json::from_slice(&answer).map_err(|e| ClientError::Protocol(e.to_string()))
    }
}

/// Listen at `path`. A socket file left by a crash is replaced; one that
/// another running copy answers on is not (`AddrInUse`).
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another copy is already answering on it",
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// At most this many connections are served at once; more are told so and
/// closed.
const MAX_CONNECTIONS: usize = 16;
/// A connection that sends nothing for this long is closed. The live view
/// asks at least every 5 s.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// A client that stops reading its answers is dropped after this long.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Answer connections on their own threads until the listener fails.
/// `handle` runs once per request, on the connection's thread.
pub fn serve(listener: UnixListener, handle: impl Fn(Request) -> Response + Send + Sync + 'static) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let handle: Arc<dyn Fn(Request) -> Response + Send + Sync> = Arc::new(handle);
    let open = Arc::new(AtomicUsize::new(0));
    /// Counts a connection while its thread runs.
    struct Slot(Arc<AtomicUsize>);
    impl Drop for Slot {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let spawned = std::thread::Builder::new()
        .name("cli-server".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Accepting a CLI connection failed: {e}");
                        continue;
                    }
                };
                if !same_user(&stream) {
                    log::warn!("Refused a CLI connection from another user");
                    continue;
                }
                if stream.set_read_timeout(Some(IDLE_TIMEOUT)).is_err()
                    || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
                {
                    continue;
                }
                let mut stream = stream;
                if open.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                    open.fetch_sub(1, Ordering::Relaxed);
                    log::warn!("Refused a CLI connection: {MAX_CONNECTIONS} already open");
                    let _ = write_line(
                        &mut stream,
                        &Response::error("too many utilifyctl connections are open"),
                    );
                    continue;
                }
                let slot = Slot(open.clone());
                let handle = handle.clone();
                let spawned =
                    std::thread::Builder::new()
                        .name("cli-conn".into())
                        .spawn(move || {
                            let _slot = slot;
                            if let Err(e) = connection(stream, &*handle) {
                                log::debug!("CLI connection ended: {e}");
                            }
                        });
                if let Err(e) = spawned {
                    log::warn!("Starting a CLI connection thread failed: {e}");
                }
            }
        });
    if let Err(e) = spawned {
        log::error!("Starting the CLI server failed: {e}");
    }
}

fn same_user(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: a valid socket fd and two valid out pointers.
    let ok = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0;
    // SAFETY: a plain query.
    ok && uid == unsafe { libc::geteuid() }
}

fn connection(stream: UnixStream, handle: &dyn Fn(Request) -> Response) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(());
        }
        let response = if line.len() > MAX_LINE {
            let r = Response::error("request too long");
            write_line(&mut writer, &r)?;
            return Ok(());
        } else {
            answer(&line, handle)
        };
        write_line(&mut writer, &response)?;
    }
}

/// One request line's answer.
pub fn answer(line: &[u8], handle: &dyn Fn(Request) -> Response) -> Response {
    #[derive(Deserialize)]
    struct Version {
        v: u32,
    }
    match serde_json::from_slice::<Version>(line) {
        Ok(Version { v }) if v != PROTOCOL => {
            return Response::error(format!(
                "this {} speaks protocol {PROTOCOL}, the CLI {v}; update both",
                crate::PRODUCT_NAME
            ));
        }
        Ok(_) => {}
        Err(e) => return Response::error(format!("unreadable request: {e}")),
    }
    match serde_json::from_slice::<Envelope>(line) {
        Ok(env) => handle(env.request),
        Err(e) => Response::error(format!(
            "this {} doesn't know that request ({e}); update it",
            crate::PRODUCT_NAME
        )),
    }
}

fn write_line(w: &mut UnixStream, r: &Response) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(r).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    w.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_as_tagged_json() {
        let env = Envelope {
            v: PROTOCOL,
            request: Request::AwakeStart {
                mode: AwakeMode::For { minutes: 90 },
            },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert_eq!(
            json,
            r#"{"v":1,"cmd":"awake_start","mode":{"mode":"for","minutes":90}}"#
        );
        let back: Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request, env.request);
        let snap: Envelope = serde_json::from_str(r#"{"v":1,"cmd":"snapshot"}"#).unwrap();
        assert_eq!(
            snap.request,
            Request::Snapshot {
                disk_history: false,
                sensors: false
            }
        );
    }

    #[test]
    fn answers_refuse_other_versions_and_unknown_requests() {
        let handle = |_: Request| Response::ok();
        let refused = |line: &str| match answer(line.as_bytes(), &handle) {
            Response::Error { message } => message,
            other => panic!("{other:?}"),
        };
        assert!(refused(r#"{"v":2,"cmd":"quit"}"#).contains("protocol"));
        assert!(refused(r#"{"v":1,"cmd":"self_destruct"}"#).contains("doesn't know"));
        assert!(refused("garbage").contains("unreadable"));
        assert!(matches!(
            answer(br#"{"v":1,"cmd":"quit"}"#, &handle),
            Response::Ok { .. }
        ));
    }

    #[test]
    fn awake_modes_convert_both_ways() {
        use crate::awake::Mode;
        for m in [
            Mode::Indefinite,
            Mode::For(Duration::from_secs(45 * 60)),
            Mode::Until(18 * 60 + 30),
            Mode::WhileApp(42),
            Mode::WhileFileDownloading,
            Mode::WhileNetworkBusy,
        ] {
            assert_eq!(AwakeMode::from_mode(m).to_mode(), m);
        }
    }

    #[test]
    fn a_slow_answer_can_be_given_up_on() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cli.sock");
        serve(bind(&path).unwrap(), |_| {
            std::thread::sleep(Duration::from_millis(1500));
            Response::ok()
        });
        let mut c = Client::connect_to(&path).unwrap();
        let stop = AtomicBool::new(false);
        let started = std::time::Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                stop.store(true, Ordering::Relaxed);
            });
            let r = c.request_until(Request::Quit, &|| stop.load(Ordering::Relaxed));
            assert!(matches!(r, Err(ClientError::Cancelled)), "{r:?}");
        });
        assert!(started.elapsed() < Duration::from_millis(800));
        // The unread answer would be out of step: the client won't reuse it.
        assert!(c.request(Request::Quit).is_err());
    }

    #[test]
    fn connections_past_the_limit_are_told_and_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cli.sock");
        serve(bind(&path).unwrap(), |_| Response::ok());
        let held: Vec<UnixStream> = (0..MAX_CONNECTIONS)
            .map(|_| UnixStream::connect(&path).unwrap())
            .collect();
        // Let the server count them.
        std::thread::sleep(Duration::from_millis(200));
        let mut extra = Client::connect_to(&path).unwrap();
        match extra.request(Request::Quit) {
            Ok(Response::Error { message }) => assert!(message.contains("too many")),
            other => panic!("{other:?}"),
        }
        drop(held);
        std::thread::sleep(Duration::from_millis(200));
        let mut again = Client::connect_to(&path).unwrap();
        assert!(matches!(
            again.request(Request::Quit),
            Ok(Response::Ok { .. })
        ));
    }

    #[test]
    fn client_and_server_talk_over_a_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cli.sock");
        assert!(matches!(
            Client::connect_to(&path),
            Err(ClientError::NotRunning)
        ));
        // A stale file from a crash is replaced.
        std::fs::write(&path, "").unwrap();
        let listener = bind(&path).unwrap();
        serve(listener, |req| match req {
            Request::DismissProblem { id } => Response::done(format!("dismissed {id}")),
            _ => Response::error("no"),
        });
        // A second server isn't allowed to take over.
        assert_eq!(
            bind(&path).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        let mut c = Client::connect_to(&path).unwrap();
        for _ in 0..2 {
            match c
                .request(Request::DismissProblem { id: "x".into() })
                .unwrap()
            {
                Response::Ok { message } => assert_eq!(message.as_deref(), Some("dismissed x")),
                other => panic!("{other:?}"),
            }
        }
        assert!(matches!(
            c.request(Request::Quit).unwrap(),
            Response::Error { .. }
        ));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
