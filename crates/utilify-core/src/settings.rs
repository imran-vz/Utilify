//! Persisted preferences: `<data dir>/dev.utilify.app/settings.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Refresh intervals offered in the menu, in milliseconds.
pub const INTERVALS_MS: [u64; 4] = [500, 1000, 2000, 5000];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How often the counters are sampled and the icon redrawn.
    pub interval_ms: u64,
    /// Show the download/upload rates as text next to the icon.
    pub show_rates: bool,
    /// Show a temperature next to the icon...
    pub show_temp: bool,
    /// ...from this sensor (a [`crate::sensors::Sensor::id`]).
    pub menu_temp_sensor: String,
    /// Show the fastest fan's speed next to the icon.
    pub show_fan_rpm: bool,
    /// The storage bar turns yellow from this much used, in percent.
    pub storage_warn_pct: u8,
    /// ...and red from this much.
    pub storage_danger_pct: u8,
    /// Post a notification when storage crosses into red.
    pub storage_notify: bool,
    /// Keep-awake sessions let the display sleep (only the system stays up).
    pub awake_allow_display_sleep: bool,
    /// Keep-awake sessions let the Mac sleep when the lid closes. Blocking
    /// it only works on AC power.
    pub awake_allow_closed_display_sleep: bool,
    /// While display sleep is blocked, start the screen saver after
    /// [`Settings::awake_screen_saver_mins`] of inactivity.
    pub awake_screen_saver: bool,
    pub awake_screen_saver_mins: u16,
    /// Start an indefinite session when Utilify launches.
    pub awake_on_launch: bool,
    /// Post a notification when a session ends by itself.
    pub awake_notify_end: bool,
    /// Master switch for the [`crate::alerts`] notifications.
    pub notify_enabled: bool,
    /// Fans forced high while on battery.
    pub notify_fans_battery: bool,
    /// Battery low during a keep-awake session.
    pub notify_low_battery_awake: bool,
    /// Switched to battery while a session blocks sleep on lid close.
    pub notify_lid_battery: bool,
    /// A timed session is about to end.
    pub notify_awake_ending: bool,
    /// Serious or critical thermal pressure.
    pub notify_thermal: bool,
    /// The overheating guard put the fans back on automatic.
    pub notify_fan_overheat: bool,
    /// The fan helper went away while fans were manual.
    pub notify_fan_helper: bool,
    /// Another app took over the fans.
    pub notify_foreign_fans: bool,
    /// The network has been offline for a minute.
    pub notify_offline: bool,
    /// A speed test finished (the app posts this one itself).
    pub notify_speed_test: bool,
    /// "Low battery" for [`Settings::notify_low_battery_awake`], in percent.
    pub low_battery_pct: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            show_rates: true,
            show_temp: false,
            menu_temp_sensor: crate::sensors::CPU_AVERAGE_ID.into(),
            show_fan_rpm: false,
            storage_warn_pct: 50,
            storage_danger_pct: 80,
            storage_notify: true,
            awake_allow_display_sleep: false,
            awake_allow_closed_display_sleep: true,
            awake_screen_saver: false,
            awake_screen_saver_mins: 45,
            awake_on_launch: false,
            awake_notify_end: true,
            notify_enabled: true,
            notify_fans_battery: true,
            notify_low_battery_awake: true,
            notify_lid_battery: true,
            notify_awake_ending: false,
            notify_thermal: true,
            notify_fan_overheat: true,
            notify_fan_helper: true,
            notify_foreign_fans: true,
            notify_offline: false,
            notify_speed_test: true,
            low_battery_pct: 20,
        }
    }
}

impl Settings {
    /// The sampling interval, clamped to something sane if the file was
    /// edited by hand.
    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms.clamp(250, 60_000))
    }

    /// The storage thresholds as fractions (yellow, red), kept in order and
    /// in range if the file was edited by hand.
    pub fn storage_thresholds(&self) -> (f64, f64) {
        let danger = self.storage_danger_pct.clamp(2, 100);
        let warn = self.storage_warn_pct.clamp(1, danger - 1);
        (warn as f64 / 100.0, danger as f64 / 100.0)
    }

    /// What keep-awake sessions let the Mac do.
    pub fn awake_allow(&self) -> crate::awake::Allow {
        crate::awake::Allow {
            display_sleep: self.awake_allow_display_sleep,
            closed_display_sleep: self.awake_allow_closed_display_sleep,
        }
    }

    /// The screen saver delay, kept sane if the file was edited by hand.
    pub fn awake_screen_saver_after(&self) -> Duration {
        Duration::from_secs(self.awake_screen_saver_mins.clamp(1, 24 * 60) as u64 * 60)
    }

    /// Which [`crate::alerts`] notifications to post. Storage uses
    /// [`Settings::storage_notify`]; the low-battery threshold is kept in
    /// 5..=50 if the file was edited by hand.
    pub fn alert_prefs(&self) -> crate::alerts::AlertPrefs {
        let low = self.low_battery_pct.clamp(5, 50);
        crate::alerts::AlertPrefs {
            enabled: self.notify_enabled,
            fans_high_on_battery: self.notify_fans_battery,
            low_battery_awake: self.notify_low_battery_awake,
            lid_sleep_on_battery: self.notify_lid_battery,
            awake_ending_soon: self.notify_awake_ending,
            thermal_pressure: self.notify_thermal,
            fan_overheat: self.notify_fan_overheat,
            fan_helper_lost: self.notify_fan_helper,
            foreign_fan_control: self.notify_foreign_fans,
            network_offline: self.notify_offline,
            storage_red: self.storage_notify,
            speed_test: self.notify_speed_test,
            low_battery_pct: low,
            critical_battery_pct: (low / 2).min(10),
        }
    }

    /// Which colour the storage bar shows at `used` (0..=1).
    pub fn storage_level(&self, used: f64) -> StorageLevel {
        let (warn, danger) = self.storage_thresholds();
        if used >= danger {
            StorageLevel::Danger
        } else if used >= warn {
            StorageLevel::Warn
        } else {
            StorageLevel::Ok
        }
    }
}

/// Storage bar colour zones: green, yellow, red.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageLevel {
    Ok,
    Warn,
    Danger,
}

/// Fires once each time storage crosses into red. It re-arms only after
/// usage drops [`StorageAlarm::REARM_BELOW`] under the red threshold, so a
/// disk hovering at the line doesn't notify over and over. The first
/// reading never fires: an app that starts already in the red shows it in
/// the bar instead of notifying at every login.
#[derive(Clone, Debug, Default)]
pub struct StorageAlarm {
    armed: Option<bool>,
}

impl StorageAlarm {
    pub const REARM_BELOW: f64 = 0.02;

    /// Feed a reading; true when this one should notify.
    pub fn update(&mut self, used: f64, danger: f64) -> bool {
        let red = used >= danger;
        match self.armed {
            None => {
                self.armed = Some(!red);
                false
            }
            Some(true) if red => {
                self.armed = Some(false);
                true
            }
            Some(false) if used < danger - Self::REARM_BELOW => {
                self.armed = Some(true);
                false
            }
            _ => false,
        }
    }
}

/// What a setting holds, which says how a typed value is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Bool,
    /// One of [`INTERVALS_MS`].
    Interval,
    /// A whole percentage in `min..=max`.
    Percent { min: u8, max: u8 },
    /// Whole minutes in `min..=max`.
    Minutes { min: u16, max: u16 },
    /// A temperature sensor's id.
    Sensor,
}

/// One setting, as the CLI lists it: its `settings.json` key, the section
/// and wording the dashboard uses for it.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub key: &'static str,
    pub section: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
}

const fn field(key: &'static str, section: &'static str, label: &'static str, kind: FieldKind) -> Field {
    Field {
        key,
        section,
        label,
        kind,
    }
}

/// The yellow and red storage limits the dashboard's steppers stay within.
pub const STORAGE_PCT_MIN: u8 = 5;
pub const STORAGE_PCT_MAX: u8 = 95;

/// Every setting, in the dashboard's order.
pub const FIELDS: &[Field] = {
    use FieldKind::*;
    const MENU: &str = "Menu bar";
    const STORAGE: &str = "Storage";
    const AWAKE: &str = "Keep awake";
    const NOTIFY: &str = "Notifications";
    &[
        field("interval_ms", MENU, "Refresh every", Interval),
        field("show_rates", MENU, "Show speeds next to the icon", Bool),
        field("show_temp", MENU, "Show temperature next to the icon", Bool),
        field("menu_temp_sensor", MENU, "Temperature sensor shown", Sensor),
        field("show_fan_rpm", MENU, "Show the fastest fan's speed", Bool),
        field(
            "storage_warn_pct",
            STORAGE,
            "Bar turns yellow from (% used)",
            Percent {
                min: STORAGE_PCT_MIN,
                max: STORAGE_PCT_MAX - 1,
            },
        ),
        field(
            "storage_danger_pct",
            STORAGE,
            "Bar turns red from (% used)",
            Percent {
                min: STORAGE_PCT_MIN + 1,
                max: STORAGE_PCT_MAX,
            },
        ),
        field("storage_notify", STORAGE, "Notify in the red", Bool),
        field("awake_allow_display_sleep", AWAKE, "Allow display sleep", Bool),
        field(
            "awake_allow_closed_display_sleep",
            AWAKE,
            "Allow sleep with the lid closed",
            Bool,
        ),
        field("awake_screen_saver", AWAKE, "Allow the screen saver", Bool),
        field(
            "awake_screen_saver_mins",
            AWAKE,
            "Screen saver after (minutes)",
            Minutes { min: 5, max: 240 },
        ),
        field("awake_on_launch", AWAKE, "Start a session at launch", Bool),
        field("awake_notify_end", AWAKE, "Notify when a session ends", Bool),
        field("notify_enabled", NOTIFY, "Allow notifications", Bool),
        field("notify_fans_battery", NOTIFY, "Fans at full speed on battery", Bool),
        field(
            "notify_low_battery_awake",
            NOTIFY,
            "Low battery while kept awake",
            Bool,
        ),
        field(
            "low_battery_pct",
            NOTIFY,
            "Low battery at (%)",
            Percent { min: 15, max: 50 },
        ),
        field("notify_lid_battery", NOTIFY, "Lid sleep on battery", Bool),
        field("notify_awake_ending", NOTIFY, "Session ending soon", Bool),
        field("notify_thermal", NOTIFY, "Thermal pressure", Bool),
        field("notify_fan_overheat", NOTIFY, "Overheating", Bool),
        field("notify_fan_helper", NOTIFY, "Fan helper lost", Bool),
        field("notify_foreign_fans", NOTIFY, "Another app takes the fans", Bool),
        field("notify_offline", NOTIFY, "Network offline", Bool),
        field("notify_speed_test", NOTIFY, "Speed test finished", Bool),
    ]
};

/// The field for `key`, which may use dashes for underscores.
pub fn find_field(key: &str) -> Option<&'static Field> {
    let key = key.trim().replace('-', "_").to_ascii_lowercase();
    FIELDS.iter().find(|f| f.key == key)
}

/// `on`, `true`, `yes`, `1` and their opposites.
pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "enable" | "enabled" => Some(true),
        "off" | "false" | "no" | "0" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

/// `500ms`, `1s`, `2 s`, `5000`.
fn parse_interval(value: &str) -> Option<u64> {
    let v = value.trim().to_ascii_lowercase().replace(' ', "");
    if let Some(ms) = v.strip_suffix("ms") {
        return ms.parse().ok();
    }
    if let Some(s) = v.strip_suffix('s') {
        return s.parse::<f64>().ok().map(|s| (s * 1000.0).round() as u64);
    }
    v.parse().ok()
}

/// `45`, `45m`, `1h`, `1h30m`.
fn parse_minutes(value: &str) -> Option<u64> {
    let v = value.trim().to_ascii_lowercase().replace(' ', "");
    if let Ok(n) = v.parse() {
        return Some(n);
    }
    let (h, rest) = match v.split_once('h') {
        Some((h, rest)) => (h.parse::<u64>().ok()?, rest),
        None => (0, v.as_str()),
    };
    let m = match rest.strip_suffix("min").or_else(|| rest.strip_suffix('m')) {
        Some(m) => m.parse::<u64>().ok()?,
        None if rest.is_empty() => 0,
        None => return None,
    };
    // Checked: a huge number is out of range, not a wrapped small one.
    h.checked_mul(60)?.checked_add(m)
}

impl Settings {
    /// One setting's value as text: `on`, `1000`, `cpu_core_average`.
    pub fn get_field(&self, key: &str) -> Option<String> {
        let f = find_field(key)?;
        let json = serde_json::to_value(self).ok()?;
        Some(match json.get(f.key)? {
            serde_json::Value::Bool(b) => if *b { "on" } else { "off" }.into(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    /// Set one setting from text, held to the limits the dashboard keeps it
    /// in. `known_sensor` says whether a sensor id exists. Returns the value
    /// as stored.
    pub fn set_field(
        &mut self,
        key: &str,
        value: &str,
        known_sensor: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        let f = find_field(key).ok_or_else(|| format!("no setting called \"{key}\""))?;
        let mut json = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let new = match f.kind {
            FieldKind::Bool => serde_json::Value::Bool(
                parse_bool(value).ok_or_else(|| format!("{} takes on or off", f.key))?,
            ),
            FieldKind::Interval => {
                let ms = parse_interval(value)
                    .filter(|ms| INTERVALS_MS.contains(ms))
                    .ok_or_else(|| {
                        let choices: Vec<String> = INTERVALS_MS
                            .iter()
                            .map(|ms| crate::format::interval(*ms))
                            .collect();
                        format!("{} takes one of {}", f.key, choices.join(", "))
                    })?;
                ms.into()
            }
            FieldKind::Percent { min, max } => {
                let v: u8 = value
                    .trim()
                    .trim_end_matches('%')
                    .trim()
                    .parse()
                    .ok()
                    .filter(|v| (min..=max).contains(v))
                    .ok_or_else(|| format!("{} takes a percentage from {min} to {max}", f.key))?;
                match f.key {
                    "storage_warn_pct" if v >= self.storage_danger_pct => {
                        return Err(format!(
                            "yellow has to start below red ({}%)",
                            self.storage_danger_pct
                        ));
                    }
                    "storage_danger_pct" if v <= self.storage_warn_pct => {
                        return Err(format!(
                            "red has to start above yellow ({}%)",
                            self.storage_warn_pct
                        ));
                    }
                    _ => {}
                }
                v.into()
            }
            FieldKind::Minutes { min, max } => {
                let v = parse_minutes(value)
                    .filter(|v| (min as u64..=max as u64).contains(v))
                    .ok_or_else(|| format!("{} takes {min} to {max} minutes", f.key))?;
                v.into()
            }
            FieldKind::Sensor => {
                let id = value.trim();
                if !known_sensor(id) {
                    return Err(format!(
                        "no sensor called \"{id}\" (`utilifyctl sensors` lists them)"
                    ));
                }
                id.into()
            }
        };
        json[f.key] = new;
        *self = serde_json::from_value(json).map_err(|e| e.to_string())?;
        self.get_field(f.key)
            .ok_or_else(|| format!("couldn't read {} back", f.key))
    }
}

/// An exclusive lock on the settings (`.settings.lock` beside
/// `settings_path`), released on drop. `utilifyctl` holds it while it edits
/// `settings.json` with the app closed; the app holds it from loading its
/// settings until its socket is up. So an edit either lands before the app
/// reads the file, or finds the app answering and goes through it.
pub struct Lock {
    _file: std::fs::File,
}

pub fn lock(settings_path: &Path) -> std::io::Result<Lock> {
    use std::os::fd::AsRawFd;
    let dir = settings_path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".settings.lock"))?;
    // SAFETY: a valid fd; closing it (on drop) releases the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Lock { _file: file })
}

pub fn default_path() -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join(crate::APP_IDENTIFIER)
            .join("settings.json"),
    )
}

/// Load settings; a missing or unreadable file gives the defaults. One that
/// doesn't parse is moved aside first ([`crate::quarantine`]).
pub fn load(path: &Path) -> Settings {
    crate::load_json(path, "settings").unwrap_or_default()
}

/// Save atomically.
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    crate::write_atomic(path, &serde_json::to_vec_pretty(settings)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        assert_eq!(load(&path), Settings::default());
        let s = Settings {
            interval_ms: 2000,
            show_rates: false,
            show_temp: true,
            menu_temp_sensor: "gpu_max".into(),
            show_fan_rpm: true,
            storage_warn_pct: 60,
            storage_danger_pct: 90,
            storage_notify: false,
            awake_allow_display_sleep: true,
            awake_allow_closed_display_sleep: false,
            awake_screen_saver: true,
            awake_screen_saver_mins: 10,
            awake_on_launch: true,
            awake_notify_end: false,
            notify_enabled: false,
            notify_fans_battery: false,
            notify_low_battery_awake: false,
            notify_lid_battery: false,
            notify_awake_ending: true,
            notify_thermal: false,
            notify_fan_overheat: false,
            notify_fan_helper: false,
            notify_foreign_fans: false,
            notify_offline: true,
            notify_speed_test: false,
            low_battery_pct: 30,
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        std::fs::write(&path, r#"{"interval_ms": 500}"#).unwrap();
        assert_eq!(
            load(&path),
            Settings {
                interval_ms: 500,
                ..Default::default()
            }
        );
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        assert!(!path.exists(), "moved aside, not left to be overwritten");
        let aside = only_corrupt_copy(path.parent().unwrap());
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), "not json");
        assert!(crate::recovered_contains(&aside));
    }

    /// The one `*.corrupt-*` file in `dir`.
    pub(crate) fn only_corrupt_copy(dir: &Path) -> PathBuf {
        let found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(found.len(), 1, "{found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn storage_levels_follow_the_thresholds() {
        let s = Settings::default();
        assert_eq!(s.storage_level(0.49), StorageLevel::Ok);
        assert_eq!(s.storage_level(0.50), StorageLevel::Warn);
        assert_eq!(s.storage_level(0.79), StorageLevel::Warn);
        assert_eq!(s.storage_level(0.80), StorageLevel::Danger);
        // Out of order by hand: yellow stays below red.
        let odd = Settings {
            storage_warn_pct: 95,
            storage_danger_pct: 70,
            ..Default::default()
        };
        assert_eq!(odd.storage_thresholds(), (0.69, 0.70));
    }

    #[test]
    fn alarm_fires_once_per_crossing() {
        let mut a = StorageAlarm::default();
        assert!(!a.update(0.70, 0.80), "first reading never fires");
        assert!(a.update(0.81, 0.80));
        assert!(!a.update(0.82, 0.80), "still red: no repeat");
        assert!(!a.update(0.79, 0.80), "inside the re-arm margin");
        assert!(!a.update(0.80, 0.80));
        assert!(!a.update(0.77, 0.80), "re-armed");
        assert!(a.update(0.80, 0.80));

        let mut started_red = StorageAlarm::default();
        assert!(!started_red.update(0.90, 0.80));
        assert!(!started_red.update(0.91, 0.80));
    }

    #[test]
    fn alert_prefs_follow_the_settings() {
        use crate::alerts::{AlertKind, AlertPrefs};
        let s = Settings::default();
        assert_eq!(s.alert_prefs(), AlertPrefs::default());
        let off = Settings {
            storage_notify: false,
            notify_offline: true,
            low_battery_pct: 90,
            ..Default::default()
        };
        let p = off.alert_prefs();
        assert!(!p.allows(AlertKind::StorageRed));
        assert!(p.allows(AlertKind::NetworkOffline));
        assert_eq!((p.low_battery_pct, p.critical_battery_pct), (50, 10));
        let muted = Settings {
            notify_enabled: false,
            ..Default::default()
        };
        assert!(
            AlertKind::ALL
                .iter()
                .all(|&k| !muted.alert_prefs().allows(k))
        );
    }

    #[test]
    fn every_setting_is_listed_once() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), FIELDS.len());
        for k in keys {
            assert!(find_field(k).is_some(), "{k} has no field");
        }
    }

    #[test]
    fn fields_are_set_from_text_within_the_ui_limits() {
        let any = |_: &str| true;
        let mut s = Settings::default();
        assert_eq!(
            s.set_field("awake-allow-display-sleep", "on", any).as_deref(),
            Ok("on")
        );
        assert!(s.awake_allow_display_sleep);
        assert_eq!(s.get_field("awake_allow_display_sleep").as_deref(), Some("on"));
        assert!(s.set_field("show_rates", "maybe", any).is_err());

        assert_eq!(s.set_field("interval_ms", "500ms", any).as_deref(), Ok("500"));
        assert_eq!(s.set_field("interval_ms", "2s", any).as_deref(), Ok("2000"));
        assert!(s.set_field("interval_ms", "3s", any).is_err());

        assert_eq!(s.set_field("storage_danger_pct", "90%", any).as_deref(), Ok("90"));
        assert!(s.set_field("storage_warn_pct", "90", any).is_err(), "not below red");
        assert!(s.set_field("storage_danger_pct", "40", any).is_err(), "not above yellow");
        assert!(s.set_field("storage_danger_pct", "99", any).is_err());
        assert!(s.set_field("low_battery_pct", "10", any).is_err());

        assert_eq!(
            s.set_field("awake_screen_saver_mins", "1h30m", any).as_deref(),
            Ok("90")
        );
        assert!(s.set_field("awake_screen_saver_mins", "1", any).is_err());
        assert!(
            s.set_field("awake_screen_saver_mins", "307445734561825861h", any)
                .is_err()
        );

        assert!(s.set_field("menu_temp_sensor", "nope", |id| id == "gpu_max").is_err());
        assert_eq!(
            s.set_field("menu_temp_sensor", "gpu_max", |id| id == "gpu_max").as_deref(),
            Ok("gpu_max")
        );
        assert!(s.set_field("colour", "red", any).is_err());
    }

    #[test]
    fn interval_is_clamped() {
        let s = Settings {
            interval_ms: 1,
            ..Default::default()
        };
        assert_eq!(s.interval(), Duration::from_millis(250));
    }
}
