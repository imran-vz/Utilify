//! Reading the words people type: durations, times of day, keep-awake
//! modes, fans, fan controls and presets.

use utilify_core::fans::{Active, Control};
use utilify_core::ipc::{AwakeMode, FanView, RunningApp, SensorView};

/// `30m`, `2h`, `1h30m`, `90` (minutes), `1.5h`.
pub fn minutes(s: &str) -> Result<u32, String> {
    let v = s.trim().to_ascii_lowercase().replace(' ', "");
    let bad = || format!("\"{s}\" isn't a length of time; try 30m, 2h or 1h30m");
    if v.is_empty() {
        return Err(bad());
    }
    if let Ok(n) = v.parse::<u32>() {
        return Ok(n);
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut rest = v.as_str();
    while !rest.is_empty() {
        let c = rest.chars().next().unwrap_or(' ');
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            rest = &rest[1..];
            continue;
        }
        let n: f64 = num.parse().map_err(|_| bad())?;
        num.clear();
        let (mult, len) = [
            ("hours", 60.0),
            ("hour", 60.0),
            ("hrs", 60.0),
            ("hr", 60.0),
            ("h", 60.0),
            ("minutes", 1.0),
            ("minute", 1.0),
            ("mins", 1.0),
            ("min", 1.0),
            ("m", 1.0),
        ]
        .iter()
        .find(|(unit, _)| rest.starts_with(unit))
        .map(|(unit, m)| (*m, unit.len()))
        .ok_or_else(bad)?;
        total += n * mult;
        rest = &rest[len..];
    }
    if !num.is_empty() {
        return Err(bad());
    }
    Ok(total.round() as u32)
}

/// `18:30`, `6:30pm`, `6pm`, `0630`, `noon`, `midnight`, as minutes after
/// midnight.
pub fn clock(s: &str) -> Result<u16, String> {
    let v = s.trim().to_ascii_lowercase().replace(' ', "");
    let bad = || format!("\"{s}\" isn't a time of day; try 18:30 or 6:30pm");
    match v.as_str() {
        "noon" => return Ok(12 * 60),
        "midnight" => return Ok(0),
        _ => {}
    }
    // Digits, a separator and am/pm only, so the slicing below can't land
    // inside a character.
    if !v
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ':' | '.' | 'a' | 'p' | 'm'))
    {
        return Err(bad());
    }
    let (body, pm) = match (v.strip_suffix("pm"), v.strip_suffix("am")) {
        (Some(b), _) => (b, Some(true)),
        (_, Some(b)) => (b, Some(false)),
        _ => (v.as_str(), None),
    };
    let (h, m): (u16, u16) = match body.split_once(':').or_else(|| body.split_once('.')) {
        Some((h, m)) => (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?),
        None if body.len() == 4 && pm.is_none() => (
            body[..2].parse().map_err(|_| bad())?,
            body[2..].parse().map_err(|_| bad())?,
        ),
        None => (body.parse().map_err(|_| bad())?, 0),
    };
    if m >= 60 {
        return Err(bad());
    }
    let h = match pm {
        Some(_) if !(1..=12).contains(&h) => return Err(bad()),
        Some(true) => h % 12 + 12,
        Some(false) => h % 12,
        None if h < 24 => h,
        None => return Err(bad()),
    };
    Ok(h * 60 + m)
}

/// A keep-awake mode from the words after `awake start`. `apps` resolves
/// `app <name>`.
pub fn awake_mode(
    words: &[String],
    apps: impl FnOnce() -> Result<Vec<RunningApp>, String>,
) -> Result<AwakeMode, String> {
    let first = words
        .first()
        .map(|w| w.to_ascii_lowercase())
        .unwrap_or_default();
    let rest = words.get(1..).unwrap_or_default().join(" ");
    match first.as_str() {
        "" | "indefinitely" | "indefinite" | "forever" | "on" | "∞" => Ok(AwakeMode::Indefinite),
        "for" => Ok(AwakeMode::For {
            minutes: minutes(&rest)?,
        }),
        "until" | "till" | "to" => Ok(AwakeMode::Until {
            minute_of_day: clock(&rest)?,
        }),
        "app" | "while-app" => {
            if rest.is_empty() {
                return Err("name the app: `awake start app Safari`".into());
            }
            Ok(AwakeMode::WhileApp {
                pid: app_pid(&rest, &apps()?)?,
            })
        }
        "download" | "downloads" | "downloading" | "file" | "while-downloading" => {
            Ok(AwakeMode::WhileFileDownloading)
        }
        "network" | "net" | "while-network-busy" => Ok(AwakeMode::WhileNetworkBusy),
        _ => match minutes(&words.join("")) {
            Ok(m) => Ok(AwakeMode::For { minutes: m }),
            Err(_) => Err(format!(
                "\"{}\" isn't a mode: use indefinitely, a length like 2h, until 18:30, \
                 app <name>, download or network",
                words.join(" ")
            )),
        },
    }
}

/// An app by pid, exact name, or the one name it starts with.
pub fn app_pid(query: &str, apps: &[RunningApp]) -> Result<i32, String> {
    if let Ok(pid) = query.parse::<i32>() {
        return Ok(pid);
    }
    let q = query.to_lowercase();
    if let Some(a) = apps.iter().find(|a| a.name.to_lowercase() == q) {
        return Ok(a.pid);
    }
    let matches: Vec<&RunningApp> = apps
        .iter()
        .filter(|a| a.name.to_lowercase().starts_with(&q))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.pid),
        [] => Err(format!(
            "no running app called \"{query}\" (`utilifyctl awake apps` lists them)"
        )),
        many => Err(format!(
            "\"{query}\" could be {}",
            many.iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A fan by index, or by its name (`left`, `right side`, `fan 2`).
pub fn fan(query: &str, fans: &[FanView]) -> Result<u8, String> {
    if let Ok(i) = query.parse::<u8>()
        && fans.iter().any(|f| f.index == i)
    {
        return Ok(i);
    }
    let q = query.to_lowercase();
    let found: Vec<&FanView> = fans
        .iter()
        .filter(|f| f.name.to_lowercase().starts_with(&q))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.index),
        _ => Err(format!(
            "no fan \"{query}\"; this Mac has {}",
            fans.iter()
                .map(|f| format!("{} ({})", f.index, f.name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `auto`, `3000` or `3000rpm`, `min`, `max`, or `curve <sensor> <from>
/// <to>` (temperatures in °C, the sensor by id or name).
pub fn control(words: &[String], fan: &FanView, sensors: &[SensorView]) -> Result<Control, String> {
    let first = words
        .first()
        .map(|w| w.to_ascii_lowercase())
        .unwrap_or_default();
    match first.as_str() {
        "auto" | "automatic" | "system" => Ok(Control::Auto),
        "min" | "minimum" => Ok(Control::Constant(fan.min)),
        "max" | "maximum" | "full" => Ok(Control::Constant(fan.max)),
        "curve" | "sensor" => {
            let [_, sensor, from, to] = words else {
                return Err("a curve needs a sensor and two temperatures: \
                            `curve cpu_core_average 50 85`"
                    .into());
            };
            let temp = |s: &str| {
                s.trim_end_matches("°c")
                    .trim_end_matches('c')
                    .trim_end_matches('°')
                    .parse::<f32>()
                    .map_err(|_| format!("\"{s}\" isn't a temperature"))
            };
            Ok(Control::SensorCurve {
                sensor_id: sensor_id(sensor, sensors)?,
                from_c: temp(&from.to_ascii_lowercase())?,
                to_c: temp(&to.to_ascii_lowercase())?,
            })
        }
        rpm => rpm
            .trim_end_matches("rpm")
            .trim()
            .parse::<f32>()
            .map(Control::Constant)
            .map_err(|_| {
                format!(
                    "\"{}\" isn't a control: use auto, a speed like 3000, min, max, \
                     or curve <sensor> <from °C> <to °C>",
                    words.join(" ")
                )
            }),
    }
}

/// A sensor by id, exact name, or the one name containing the words.
pub fn sensor_id(query: &str, sensors: &[SensorView]) -> Result<String, String> {
    if sensors.is_empty() || sensors.iter().any(|s| s.id == query) {
        return Ok(query.to_string());
    }
    let q = query.to_lowercase();
    if let Some(s) = sensors.iter().find(|s| s.name.to_lowercase() == q) {
        return Ok(s.id.clone());
    }
    let found: Vec<&SensorView> = sensors
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&q) || s.id.contains(&q))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!(
            "no sensor \"{query}\" (`utilifyctl sensors` lists them)"
        )),
        many => Err(format!(
            "\"{query}\" could be {}",
            many.iter()
                .take(6)
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `automatic`, `full-blast`, `custom`, or a preset's name.
pub fn preset(name: &str, presets: &[String]) -> Result<Active, String> {
    // Your own presets come first: "Max" or "Auto" can be a preset's name.
    let name = name.trim();
    if let Some(p) = presets.iter().find(|p| p.as_str() == name) {
        return Ok(Active::Preset(p.clone()));
    }
    let folded: Vec<&String> = presets
        .iter()
        .filter(|p| p.to_lowercase() == name.to_lowercase())
        .collect();
    match folded.as_slice() {
        [one] => return Ok(Active::Preset((*one).clone())),
        [] => {}
        many => {
            return Err(format!(
                "\"{name}\" could be {}; type the name exactly",
                many.iter()
                    .map(|p| format!("\"{p}\""))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ));
        }
    }
    Ok(
        match name.to_ascii_lowercase().replace(['-', '_'], " ").as_str() {
            "auto" | "automatic" => Active::Automatic,
            "full blast" | "full" | "max" | "fullblast" => Active::FullBlast,
            "custom" => Active::Custom,
            _ => Active::Preset(name.to_string()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn lengths_of_time() {
        assert_eq!(minutes("30m"), Ok(30));
        assert_eq!(minutes("2h"), Ok(120));
        assert_eq!(minutes("1h30m"), Ok(90));
        assert_eq!(minutes("1h 30 min"), Ok(90));
        assert_eq!(minutes("1.5h"), Ok(90));
        assert_eq!(minutes("45"), Ok(45));
        assert!(minutes("soon").is_err());
        assert!(minutes("3x").is_err());
        assert!(minutes("12").is_ok());
        assert!(minutes("1h3").is_err());
    }

    #[test]
    fn times_of_day() {
        assert_eq!(clock("18:30"), Ok(18 * 60 + 30));
        assert_eq!(clock("6:30pm"), Ok(18 * 60 + 30));
        assert_eq!(clock("6pm"), Ok(18 * 60));
        assert_eq!(clock("12am"), Ok(0));
        assert_eq!(clock("12pm"), Ok(12 * 60));
        assert_eq!(clock("0630"), Ok(6 * 60 + 30));
        assert_eq!(clock("noon"), Ok(720));
        assert!(clock("25:00").is_err());
        assert!(clock("13pm").is_err());
        assert!(clock("9:75").is_err());
        assert!(clock("a€").is_err());
        assert!(clock("1€30").is_err());
    }

    fn apps() -> Vec<RunningApp> {
        vec![
            RunningApp {
                pid: 10,
                name: "Safari".into(),
            },
            RunningApp {
                pid: 11,
                name: "Slack".into(),
            },
            RunningApp {
                pid: 12,
                name: "Xcode".into(),
            },
        ]
    }

    #[test]
    fn awake_modes_from_words() {
        let none = || Err("unused".to_string());
        assert_eq!(awake_mode(&[], none), Ok(AwakeMode::Indefinite));
        assert_eq!(
            awake_mode(&words("2h"), none),
            Ok(AwakeMode::For { minutes: 120 })
        );
        assert_eq!(
            awake_mode(&words("1h 30m"), none),
            Ok(AwakeMode::For { minutes: 90 })
        );
        assert_eq!(
            awake_mode(&words("for 45m"), none),
            Ok(AwakeMode::For { minutes: 45 })
        );
        assert_eq!(
            awake_mode(&words("until 6pm"), none),
            Ok(AwakeMode::Until {
                minute_of_day: 1080
            })
        );
        assert_eq!(
            awake_mode(&words("download"), none),
            Ok(AwakeMode::WhileFileDownloading)
        );
        assert_eq!(
            awake_mode(&words("app xcode"), || Ok(apps())),
            Ok(AwakeMode::WhileApp { pid: 12 })
        );
        assert!(awake_mode(&words("nonsense"), none).is_err());
    }

    #[test]
    fn apps_by_name_or_prefix() {
        assert_eq!(app_pid("safari", &apps()), Ok(10));
        assert_eq!(app_pid("Xc", &apps()), Ok(12));
        assert_eq!(app_pid("42", &apps()), Ok(42));
        let err = app_pid("s", &apps()).unwrap_err();
        assert!(err.contains("Safari") && err.contains("Slack"), "{err}");
        assert!(app_pid("Mail", &apps()).is_err());
    }

    fn fan_view(index: u8, name: &str) -> FanView {
        FanView {
            index,
            name: name.into(),
            rpm: 1200.0,
            min: 1200.0,
            max: 5800.0,
            manual: false,
            control: Control::Auto,
            control_label: "Auto".into(),
            target: None,
            issue: None,
        }
    }

    #[test]
    fn fans_and_controls() {
        let fans = [fan_view(0, "Left side"), fan_view(1, "Right side")];
        assert_eq!(fan("1", &fans), Ok(1));
        assert_eq!(fan("left", &fans), Ok(0));
        assert!(fan("7", &fans).is_err());
        let sensors = vec![SensorView {
            id: "cpu_core_average".into(),
            name: "CPU Core Average".into(),
            group: "CPU".into(),
            celsius: 50.0,
        }];
        let f = &fans[0];
        assert_eq!(control(&words("auto"), f, &sensors), Ok(Control::Auto));
        assert_eq!(
            control(&words("3000rpm"), f, &sensors),
            Ok(Control::Constant(3000.0))
        );
        assert_eq!(
            control(&words("max"), f, &sensors),
            Ok(Control::Constant(5800.0))
        );
        assert_eq!(
            control(&words("curve average 50 85°C"), f, &sensors),
            Ok(Control::SensorCurve {
                sensor_id: "cpu_core_average".into(),
                from_c: 50.0,
                to_c: 85.0
            })
        );
        assert!(control(&words("curve x"), f, &sensors).is_err());
        assert!(control(&words("loud"), f, &sensors).is_err());
    }

    #[test]
    fn presets_by_name() {
        assert_eq!(preset("auto", &[]), Ok(Active::Automatic));
        assert_eq!(preset("Full-Blast", &[]), Ok(Active::FullBlast));
        assert_eq!(preset("Quiet", &[]), Ok(Active::Preset("Quiet".into())));
        // A preset of yours wins over a built-in's alias.
        let mine = vec!["Max".to_string(), "Quiet".to_string()];
        assert_eq!(preset("Max", &mine), Ok(Active::Preset("Max".into())));
        assert_eq!(preset("max", &mine), Ok(Active::Preset("Max".into())));
        assert_eq!(preset("quiet", &mine), Ok(Active::Preset("Quiet".into())));
        assert_eq!(preset("full", &mine), Ok(Active::FullBlast));
        // Two of yours differing only in case: ask, don't guess.
        let twins = vec!["Auto".to_string(), "auto".to_string()];
        assert_eq!(preset("auto", &twins), Ok(Active::Preset("auto".into())));
        assert!(preset("AUTO", &twins).is_err());
    }
}
