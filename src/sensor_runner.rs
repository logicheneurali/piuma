//! Generic sensor runner — executes sensor sources and returns their values.

use crate::config::SensorSource;
use crate::graphs;
use crate::notifications;
use crate::sensors;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What one sensor run produced: the text for the bar and — for the
/// graphable sources — the very same reading as a number with its
/// Y-scale policy (see `graphs::scale_of`). One read per tick feeds
/// both consumers: no second pass over /proc, no extra command spawn,
/// and the stateful CPU delta can only be sampled once.
#[derive(Clone, Debug)]
pub struct Read {
    pub text: String,
    pub graph: Option<(f64, graphs::Scale)>,
}

/// A reading without a graph sample.
fn plain(text: String) -> Read {
    Read {
        text,
        graph: None,
    }
}

impl Read {
    /// A read that never ran (its reply raced a config reload, or a
    /// command's first result has not landed yet): empty text and no
    /// sample, so a graph column repeats its last value instead of
    /// taking a neighbour's reading.
    pub fn missing() -> Read {
        plain(String::new())
    }
}

/// A numeric reading: text for the bar, value + the source's scale
/// policy for the graph (`None` scale = source not graphable).
fn measured(text: String, source: &SensorSource, value: f64) -> Read {
    Read {
        text,
        graph: graphs::scale_of(source).map(|scale| (value, scale)),
    }
}

/// An optional reading rendered as `N/A` when absent.
fn optional(reading: Option<(String, f64)>, source: &SensorSource) -> Read {
    match reading {
        Some((text, value)) => {
            // perc_ — the bar text carries the `%` symbol (an answer
            // that already ends in it is not doubled); the number that
            // feeds the graph keeps its bare value.
            let text = if is_percent(source) && !text.ends_with('%') {
                format!("{text}%")
            } else {
                text
            };
            measured(text, source, value)
        }
        None => plain("N/A".to_string()),
    }
}

/// Whether a source's reading is a percentage: the family whose bar
/// text carries the `%` symbol.
fn is_percent(source: &SensorSource) -> bool {
    matches!(
        source,
        SensorSource::PercCpu {}
            | SensorSource::PercGpu {}
            | SensorSource::PercMem {}
            | SensorSource::PercSwap {}
            | SensorSource::PercDisk {}
            | SensorSource::PercBattery {}
            | SensorSource::PercBacklight {}
            | SensorSource::PercVolume { .. }
            | SensorSource::VolumeMeter { .. }
    )
}

/// A numeric sample out of a text answer (the user-authored sources):
/// the WHOLE trimmed answer must be a finite number, optionally
/// followed by one digit-free suffix ("55%", "49°C", "1.9 GB"). A
/// timestamp, an IP, a version, a locale decimal, "N/A" or NaN/inf
/// parse to nothing — a wrong point would ride the Auto axis for the
/// whole window, a refused one just repeats the previous value.
fn strict_number(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Ok(value) = text.parse::<f64>() {
        return value.is_finite().then_some(value);
    }
    // The first char outside the number grammar ends the number: the
    // head must parse as finite and the suffix must carry no digit, so
    // "16:45:30" (suffix holds digits) is refused while "55%" and
    // "49°C" pass.
    let cut =
        text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E')))?;
    let (head, tail) = text.split_at(cut);
    if tail.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    head.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A text reading that may carry the graph's sample: the graph takes
/// the number only when the whole answer is one (`strict_number`),
/// anything else stays plain text with no graph.
fn maybe_measured(text: String, source: &SensorSource) -> Read {
    match strict_number(&text) {
        Some(value) => measured(text, source, value),
        None => plain(text),
    }
}

/// The floor of a command's run budget: the bar polls at 1 Hz, so a
/// command has at least this long even with no interval.
const COMMAND_TIMEOUT_MIN: u64 = 5;
/// The ceiling: a hung command holds no other reading up (see
/// `spawn_refresh`), but it should not linger for hours either.
const COMMAND_TIMEOUT_MAX: u64 = 60;

/// The wall-clock budget of one command run: the sensor's interval when
/// that is longer than the floor, so a legitimate slow command under a
/// long interval (a 9 s whois with `interval: 20`) is not killed.
/// `interval: 0` and the short ones keep the historical 5 s.
fn command_timeout(interval: u64) -> u64 {
    interval.clamp(COMMAND_TIMEOUT_MIN, COMMAND_TIMEOUT_MAX)
}

/// The command source's shell run within `timeout` seconds, output
/// trimmed. A non-zero exit — the timeout included — reads as `ERROR`.
fn run_command(command: &str, timeout: u64) -> String {
    match std::process::Command::new("timeout")
        .args(["-k", "1", &timeout.to_string(), "sh", "-c", command])
        .output()
    {
        Ok(out) if out.status.success() => String::from(String::from_utf8_lossy(&out.stdout).trim()),
        _ => "ERROR".to_string(),
    }
}

/// Cached runs of `command` sources, keyed by the command line alone —
/// two sensors sharing a command share the result. The stored instant is
/// the run's START, so a slow run does not stretch the period: the next
/// one is due `interval` after the previous began. Failures are cached
/// too (no retry storm).
static COMMAND_RUNS: OnceLock<Mutex<HashMap<String, (Instant, Read)>>> = OnceLock::new();

/// The cached read for `key` and whether a new run is due (`ttl` 0 =
/// always; `ttl` elapsed since the last run's start = due).
fn command_snapshot(key: &str, ttl: u64) -> (Option<Read>, bool) {
    let Some(map) = COMMAND_RUNS.get_or_init(Default::default).lock().ok() else {
        return (None, true);
    };
    match map.get(key) {
        Some((at, read)) => {
            let due = ttl == 0 || at.elapsed() >= Duration::from_secs(ttl);
            (Some(read.clone()), due)
        }
        None => (None, true),
    }
}

fn command_store(key: &str, at: Instant, read: &Read) {
    let Ok(mut map) = COMMAND_RUNS.get_or_init(Default::default).lock() else {
        return;
    };
    // Bounded: the map only grows with distinct commands across config
    // reloads; clearing on overflow is cheaper than a stale-entry sweep.
    if map.len() > 64 {
        map.clear();
    }
    map.insert(key.to_string(), (at, read.clone()));
}

/// Keys with a command still running: a due key keeps its previous value
/// instead of forking a second copy of the same run.
static RUNNING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

/// A claim on a key, released when the run ends (a panic included).
struct Running(String);

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut set) = RUNNING.get_or_init(Default::default).lock() {
            set.remove(&self.0);
        }
    }
}

/// Start `job` for `key` unless a run is still in flight; the job stores
/// its own result. Detached on purpose: the pass returns the last cached
/// value at once, so a slow — or hung — command cannot hold up the tick,
/// the UI or any other sensor, not even another command's run.
fn spawn_refresh(key: &str, job: impl FnOnce() + Send + 'static) -> bool {
    let Ok(mut set) = RUNNING.get_or_init(Default::default).lock() else {
        return false;
    };
    if set.len() > 64 {
        set.clear();
    }
    if !set.insert(key.to_string()) {
        return false;
    }
    let claim = Running(key.to_string());
    std::thread::spawn(move || {
        let _claim = claim;
        job();
    });
    true
}

/// The stdout lines of every list sensor's last run, keyed by command:
/// the hover popup reads them without forking again. Bounded like the
/// command cache (cleared on overflow, never swept).
static LIST_LINES: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();

fn list_store(command: &str, lines: Vec<String>) {
    let Ok(mut map) = LIST_LINES.get_or_init(Default::default).lock() else {
        return;
    };
    if map.len() > 64 {
        map.clear();
    }
    map.insert(command.to_string(), lines);
}

/// The lines a list sensor's command printed last (empty until it ran).
pub fn list_lines(command: &str) -> Vec<String> {
    LIST_LINES
        .get_or_init(Default::default)
        .lock()
        .ok()
        .and_then(|map| map.get(command).cloned())
        .unwrap_or_default()
}

/// Run a sensor source and return its value (text + optional sample).
/// `hypr` is the tick's shared compositor snapshot (`None` = the read
/// was not taken; the Hyprland-backed sources show their fallback).
pub fn run(source: &SensorSource, hypr: Option<&crate::hypr::Snapshot>) -> Read {
    match source {
        // The user-authored text sources: the answer feeds the graph
        // sample when the whole thing is a number (see `strict_number`).
        // shortcut: the 5s cap (coreutils `timeout`) keeps a command that
        // never exits — `--follow` streams, listeners, hung pipelines —
        // from freezing every reading for good: `read_pending` never
        // clears without a reply, so no further pass would ever run.
        // Raise the cap when a legitimate command needs longer.
        SensorSource::Command { command, interval } => {
            let ttl = interval.unwrap_or(0);
            let (last, due) = command_snapshot(command, ttl);
            if due {
                let (cmd, source, budget) =
                    (command.clone(), source.clone(), command_timeout(ttl));
                spawn_refresh(command, move || {
                    let at = Instant::now();
                    let read = maybe_measured(run_command(&cmd, budget), &source);
                    command_store(&cmd, at, &read);
                });
            }
            // The pass never waits: the run lands in the cache and the
            // next tick shows it (empty until the first one finishes).
            last.unwrap_or_else(Read::missing)
        }
        // A list: the bar shows the first line (or the sensor's name),
        // the hover popup shows the rest (see `list_lines`).
        SensorSource::List { command, interval, .. } => {
            let ttl = interval.unwrap_or(0);
            // Own cache key: a `command` sensor on the same line keeps the
            // whole output, this one only its first line.
            let key = format!("list:{command}");
            let (last, due) = command_snapshot(&key, ttl);
            if due {
                let (cmd, budget, cache_key) =
                    (command.clone(), command_timeout(ttl), key.clone());
                spawn_refresh(&key, move || {
                    let at = Instant::now();
                    let lines: Vec<String> =
                        run_command(&cmd, budget).lines().map(str::to_string).collect();
                    let read = plain(lines.first().cloned().unwrap_or_default());
                    list_store(&cmd, lines);
                    command_store(&cache_key, at, &read);
                });
            }
            last.unwrap_or_else(Read::missing)
        }
        SensorSource::EnvVar { env_var } => {
            maybe_measured(std::env::var(env_var).unwrap_or("N/A".to_string()), source)
        }
        SensorSource::StaticText { static_text } => maybe_measured(static_text.clone(), source),
        // perc_ — percentage values
        SensorSource::PercCpu {} => optional(sensors::cpu_load(), source),
        SensorSource::PercGpu {} => optional(sensors::gpu_load(), source),
        SensorSource::PercMem {} => optional(sensors::mem_perc(), source),
        SensorSource::PercSwap {} => optional(sensors::swap_perc(), source),
        SensorSource::PercDisk {} => optional(sensors::disk_usage(), source),
        SensorSource::PercBattery {} => optional(sensors::battery_perc(), source),
        SensorSource::PercBacklight {} => optional(sensors::backlight_perc(), source),
        SensorSource::PercVolume { device } => optional(sensors::audio_volume(device), source),
        SensorSource::VolumeMeter { device } => optional(sensors::audio_volume(device), source),
        // used_ — numeric consumption (graphs plot percent-of-total)
        SensorSource::UsedMem {} => optional(sensors::mem_used(), source),
        SensorSource::UsedSwap {} => optional(sensors::swap_used(), source),
        SensorSource::UsedDisk {} => optional(sensors::disk_used(), source),
        SensorSource::UsedBattery {} => optional(sensors::battery_used(), source),
        SensorSource::UsedNetdown { interface } => optional(sensors::net_down(interface), source),
        SensorSource::UsedNetup { interface } => optional(sensors::net_up(interface), source),
        // total_ — full values (no numeric category to graph)
        SensorSource::TotalMem {} => plain(sensors::mem_total()),
        SensorSource::TotalSwap {} => plain(sensors::swap_total()),
        SensorSource::TotalDisk {} => plain(sensors::disk_total()),
        SensorSource::TotalBattery {} => plain(sensors::battery_total()),
        // detail_ — used/total ratios
        SensorSource::DetailMem {} => plain(sensors::mem_usage()),
        SensorSource::DetailSwap {} => plain(sensors::swap_detail()),
        SensorSource::DetailDisk {} => plain(sensors::disk_detail()),
        // temp_ — temperatures
        SensorSource::TempCpu {} => optional(sensors::cpu_temp(), source),
        SensorSource::TempGpu {} => optional(sensors::gpu_temp(), source),
        // everything else (no numeric category)
        SensorSource::NetInfo {} => plain(sensors::net_info()),
        // Wi-Fi: the text names the state; the signal percentage is the
        // numeric sample the icon (and an opted graph) derives from.
        SensorSource::Wifi {} => match sensors::wifi_status() {
            Some(status) => {
                let text = status.label();
                match (status.connected, status.signal) {
                    (true, Some(signal)) => measured(text, source, f64::from(signal)),
                    _ => plain(text),
                }
            }
            None => plain("N/A".to_string()),
        },
        SensorSource::PowerProfile {} => plain(sensors::power_profile()),
        SensorSource::Bluetooth {} => plain(sensors::bluetooth_info()),
        SensorSource::Keyboard {} => plain(sensors::keyboard_info()),
        SensorSource::Uptime {} => plain(sensors::uptime()),
        SensorSource::NetState { interface } => plain(sensors::network_info(interface)),
        SensorSource::Clock { format } => {
            plain(match format.as_ref().map(|f| f.as_str()) {
                Some("date") => sensors::get_date(),
                _ => sensors::get_time(),
            })
        }
        // The calendar sensor's bar text: a full strftime format (its
        // hover popup lives in main.rs, see `calendar_hover_sync`).
        SensorSource::Calendar { format } => plain(sensors::clock_formatted(
            format.as_deref().unwrap_or("%H:%M:%S"),
        )),
        // Both ride the tick's shared compositor snapshot — no fork
        // of their own (see `hypr.rs`).
        SensorSource::Window {} => plain(sensors::window_title(hypr)),
        SensorSource::Workspace {} => plain(sensors::workspace_name(hypr)),
        // The dots carry no text: the view draws them from the snapshot.
        SensorSource::WorkspaceDots { .. } => plain(String::new()),
        // MPRIS state is read by the same background pass; no player
        // yields an empty value and the view hides the sensor.
        SensorSource::Media {} => plain(crate::mpris::display_value()),
        // A trigger, not a reading: its value is empty, the name/icon is
        // what the bar draws (the click locks, see `main::update`).
        SensorSource::Lock {} => plain(String::new()),
        // Also a trigger: the state text comes from `apply_read`
        // (the runner cannot see the panel's toggle).
        SensorSource::Caffeine {} => plain("off".to_string()),
        SensorSource::Power {} => plain(String::new()),
        // The active theme's name comes from `apply_read` (only the bar
        // reads `theme.json`), like the caffeine toggle.
        SensorSource::Theme {} => plain("Custom".to_string()),
        SensorSource::Dashboard {} => plain(String::new()),
        // Tray items carry no text: the icons come from the shared tray
        // state (see `tray.rs`), rendered by the view directly.
        SensorSource::Tray {} => plain(String::new()),
        // Newest active notification's summary (see `notifications.rs`).
        SensorSource::Notifications {} => plain(notifications::display_value()),
        // Taskbar icons carry no text: the slots come from
        // `windows::task_icons()`, built per tick in `update`.
        SensorSource::Taskbar {} => plain(String::new()),
        // A layout sensor's text is the engine's state (armed/applied/
        // just saved): computed in `apply_read` from the same snapshot,
        // where the panel's state lives.
        SensorSource::Layout { .. } => plain(String::new()),
    }
}

/// One background data pass for a tick: the shared compositor
/// snapshot (`None` when no Hyprland-backed sensor asked for it) plus
/// every sensor's reading, in config order — the exact sequence the
/// tick used to run inline on the UI thread.
#[derive(Clone, Debug)]
pub struct Reads {
    pub hypr: Option<crate::hypr::Snapshot>,
    pub values: Vec<Read>,
    /// The dashboard's snapshot (media, power profile, volume,
    /// brightness, Bluetooth, network, uptime) while the card is open;
    /// `None` otherwise — a closed dashboard reads nothing.
    pub dashboard: Option<crate::dashboard::Probe>,
}

/// Read every source off the UI thread (the tick spawns this on a
/// worker, see `spawn_read`): one pass forks the compositor documents
/// every Hyprland-backed source shares and then each sensor in turn —
/// but duplicated sensors share one run (see the cache below).
pub fn read_all(sources: &[SensorSource], want_hypr: bool, want_dashboard: bool) -> Reads {
    let needs_hypr = want_hypr
        || sources.iter().any(|s| {
            matches!(
                s,
                SensorSource::Window {}
                    | SensorSource::Workspace {}
                    | SensorSource::WorkspaceDots { .. }
                    | SensorSource::Taskbar {}
                    // The layout engine matches and saves against the same
                    // snapshot `apply_read` receives.
                    | SensorSource::Layout { .. }
            )
        });
    let hypr = needs_hypr.then(crate::hypr::snapshot);
    // One run per DISTINCT source per pass: two `df` disk sensors, two
    // `date` clocks and the like showed the same numbers when each
    // forked its own copy — they now clone the first reading. The key
    // is the source's serialized form (deterministic field order).
    let mut seen: HashMap<String, Read> = HashMap::new();
    let values = sources
        .iter()
        .map(|s| {
            let Ok(key) = serde_json::to_string(s) else {
                // No key, no dedup: run it on its own.
                return run(s, hypr.as_ref());
            };
            if let Some(hit) = seen.get(&key) {
                return hit.clone();
            }
            let read = run(s, hypr.as_ref());
            seen.insert(key, read.clone());
            read
        })
        .collect();
    let dashboard = want_dashboard.then(crate::dashboard::probe);
    Reads { hypr, values, dashboard }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// perc_ readings carry the `%` symbol in their bar text; the
    /// number that feeds the graph keeps its bare value, other sources
    /// and the `N/A` fallback don't change, and an answer that already
    /// ends in `%` is not doubled.
    #[test]
    fn percent_readings_carry_the_symbol() {
        let cpu = optional(Some(("5".into(), 5.5)), &SensorSource::PercCpu {});
        assert_eq!(cpu.text, "5%");
        assert_eq!(cpu.graph.map(|(v, _)| v), Some(5.5), "the sample stays bare");

        let mem = optional(Some(("5".into(), 5.0)), &SensorSource::UsedMem {});
        assert_eq!(mem.text, "5");

        assert_eq!(optional(None, &SensorSource::PercMem {}).text, "N/A");

        let vol = optional(
            Some(("70%".into(), 70.0)),
            &SensorSource::PercVolume {
                device: String::new(),
            },
        );
        assert_eq!(vol.text, "70%", "already suffixed: no doubling");
    }

    /// The graph sample of a text source is the whole answer as a
    /// finite number: clean numbers and single-suffix ones pass,
    /// ambiguous or non-numeric answers record nothing — and the bar
    /// text never changes either way.
    #[test]
    fn text_sources_graph_only_whole_numbers() {
        for (text, expected) in [
            ("42", Some(42.0)),
            (" 42  ", Some(42.0)),
            ("-3.5", Some(-3.5)),
            ("1e3", Some(1000.0)),
            ("55%", Some(55.0)),
            ("49°C", Some(49.0)),
            ("1.9 GB", Some(1.9)),
            ("16:45:30", None),
            ("192.168.1.1", None),
            ("1.2.3", None),
            ("1,5", None),
            ("N/A", None),
            ("ERROR", None),
            ("nan", None),
            ("inf", None),
            ("3 g 4", None),
            ("exit code 5", None),
        ] {
            assert_eq!(strict_number(text), expected, "{text:?}");
        }

        let source = SensorSource::Command {
            command: String::new(),
            interval: None,
        };
        let read = maybe_measured("49°C".into(), &source);
        assert_eq!(read.graph.map(|(v, _)| v), Some(49.0));
        assert_eq!(read.text, "49°C", "the bar text stays as-is");
        let read = maybe_measured("hello".into(), &source);
        assert!(read.graph.is_none());
        assert_eq!(read.text, "hello");
    }

    /// Poll the command cache until `key` has a reading (the runs are
    /// detached, so the test waits on the result, never on `run`).
    fn wait_read(key: &str) -> Read {
        for _ in 0..300 {
            if let (Some(read), _) = command_snapshot(key, 0) {
                return read;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no reading for {key:?}");
    }

    /// A list sensor's bar text is the first line, and every line is kept
    /// for the hover popup. The pass returns before the command runs.
    #[test]
    fn list_sensor_shows_the_first_line_and_keeps_the_rest() {
        let command = "printf 'one\\ntwo\\nthree\\n'";
        let source = SensorSource::List {
            command: command.to_string(),
            interval: None,
            max_lines: 2,
        };
        assert_eq!(run(&source, None).text, "", "the pass never waits");
        assert_eq!(wait_read(&format!("list:{command}")).text, "one");
        assert_eq!(
            list_lines(command),
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
        assert!(list_lines("piuma-never-ran").is_empty());
    }

    /// A slow command (here half a second) does not delay the pass: the
    /// first read is empty, the value lands in the cache afterwards. A
    /// hung one is cut by `command_timeout`, which follows the interval.
    #[test]
    fn a_slow_command_never_blocks_the_pass() {
        let command = "sleep 0.5; echo done # piuma-test-slow";
        let source = SensorSource::Command {
            command: command.to_string(),
            interval: Some(60),
        };
        let started = Instant::now();
        let first = run(&source, None);
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "the pass must not wait for the command"
        );
        assert_eq!(first.text, "", "no reading yet on the first pass");
        assert_eq!(wait_read(command).text, "done");
    }

    /// The run budget is the interval when longer than the floor, never
    /// below 5 s nor above the ceiling (a hung command would otherwise
    /// linger for the whole interval).
    #[test]
    fn command_budget_follows_the_interval_within_bounds() {
        assert_eq!(command_timeout(0), 5);
        assert_eq!(command_timeout(1), 5);
        assert_eq!(command_timeout(20), 20);
        assert_eq!(command_timeout(3600), 60);
    }

    /// A command with an interval reuses its last reading until the
    /// interval elapses; interval 0 is always due. The stored instant is
    /// the run's start, so the period does not stretch with the run.
    #[test]
    fn interval_commands_reuse_their_last_reading() {
        let command = "piuma-test-interval-command"; // unique to this test
        assert!(
            command_snapshot(command, 60).0.is_none(),
            "nothing cached yet"
        );
        command_store(command, Instant::now(), &plain("value".to_string()));
        let (read, due) = command_snapshot(command, 60);
        assert_eq!(read.map(|read| read.text), Some("value".to_string()));
        assert!(!due);
        assert!(command_snapshot(command, 0).1, "interval 0 is always due");
        assert!(
            command_snapshot("piuma-test-other-command", 60).0.is_none(),
            "the cache is per command"
        );
    }

    /// Duplicated sensors share one reading per pass (no second fork
    /// for a second `df`/`date`); distinct sources keep their own.
    /// Static texts run without forking, so the test stays compositor-
    /// and command-free.
    #[test]
    fn duplicated_sources_share_one_reading() {
        let sources = [
            SensorSource::StaticText {
                static_text: "same".into(),
            },
            SensorSource::StaticText {
                static_text: "same".into(),
            },
            SensorSource::StaticText {
                static_text: "other".into(),
            },
        ];
        let reads = read_all(&sources, false, false);
        assert_eq!(reads.values.len(), 3);
        assert_eq!(reads.values[0].text, "same");
        assert_eq!(
            reads.values[1].text, reads.values[0].text,
            "the duplicate clones the first reading"
        );
        assert_eq!(reads.values[2].text, "other");
    }
}
