use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_TICK: Duration = Duration::from_millis(125);
const CLEAR_LINE: &str = "\r\x1b[2K";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";

pub struct Activity {
    base: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            base: Instant::now(),
            last_ms: AtomicU64::new(0),
        })
    }

    pub fn mark_active(&self) {
        let ms = self.base.elapsed().as_millis() as u64;
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    pub fn idle_for(&self) -> Duration {
        let now = self.base.elapsed().as_millis() as u64;
        let last = self.last_ms.load(Ordering::Relaxed);
        Duration::from_millis(now.saturating_sub(last))
    }

    pub fn elapsed(&self) -> Duration {
        self.base.elapsed()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Warn,
    Kill,
}

/// Pure decision function for what the supervisor should do this tick.
/// `last_warn_ms` is 0 when no warning has been emitted yet.
pub fn next_action(
    idle: Duration,
    idle_warn: Option<Duration>,
    idle_kill: Option<Duration>,
    last_warn_ms: u64,
    now_ms: u64,
) -> Action {
    if let Some(kill_t) = idle_kill
        && idle >= kill_t
    {
        return Action::Kill;
    }
    if let Some(warn_t) = idle_warn
        && idle >= warn_t
    {
        let interval_ms = warn_t.as_millis() as u64;
        if last_warn_ms == 0 || now_ms.saturating_sub(last_warn_ms) >= interval_ms {
            return Action::Warn;
        }
    }
    Action::Nothing
}

/// Build the spinner line shown on stderr-TTY. Pure for testability.
pub fn format_spinner_line(
    frame: char,
    name: &str,
    elapsed: Duration,
    idle: Duration,
    idle_warn: Option<Duration>,
    idle_kill: Option<Duration>,
    color: bool,
) -> String {
    let elapsed_s = elapsed.as_secs();
    let core = if let Some(warn_t) = idle_warn
        && idle >= warn_t
    {
        format!(
            "{frame} task {name:?} running {elapsed_s}s [idle {}s]",
            idle.as_secs()
        )
    } else {
        format!("{frame} task {name:?} running {elapsed_s}s")
    };

    if !color {
        return core;
    }

    let red_threshold = idle_kill.map(|t| (t.as_millis() * 4 / 5) as u64);
    let yellow_threshold = idle_warn.map(|t| t.as_millis() as u64);
    let idle_ms = idle.as_millis() as u64;

    if let Some(rt) = red_threshold
        && idle_ms >= rt
    {
        return format!("{ANSI_RED}{core}{ANSI_RESET}");
    }
    if let Some(yt) = yellow_threshold
        && idle_ms >= yt
    {
        return format!("{ANSI_YELLOW}{core}{ANSI_RESET}");
    }
    core
}

struct ReporterState {
    spinner_active: bool,
}

fn reporter() -> &'static Mutex<ReporterState> {
    static REPORTER: OnceLock<Mutex<ReporterState>> = OnceLock::new();
    REPORTER.get_or_init(|| {
        Mutex::new(ReporterState {
            spinner_active: false,
        })
    })
}

pub fn stderr_is_tty() -> bool {
    io::stderr().is_terminal()
}

pub fn color_enabled() -> bool {
    stderr_is_tty() && std::env::var_os("NO_COLOR").is_none()
}

/// Print a status line to stderr, clearing any active spinner first so it
/// doesn't get tangled with the new line. Always ends with a newline.
pub fn status_line(msg: &str) {
    let state = reporter().lock().expect("reporter mutex poisoned");
    let mut err = io::stderr().lock();
    if state.spinner_active {
        let _ = err.write_all(CLEAR_LINE.as_bytes());
    }
    let _ = writeln!(err, "{msg}");
    let _ = err.flush();
}

fn write_spinner_frame(text: &str) {
    let mut state = reporter().lock().expect("reporter mutex poisoned");
    let mut err = io::stderr().lock();
    let _ = err.write_all(CLEAR_LINE.as_bytes());
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
    state.spinner_active = true;
}

fn clear_spinner() {
    let mut state = reporter().lock().expect("reporter mutex poisoned");
    if state.spinner_active {
        let mut err = io::stderr().lock();
        let _ = err.write_all(CLEAR_LINE.as_bytes());
        let _ = err.flush();
        state.spinner_active = false;
    }
}

pub struct SupervisorHandle {
    handle: Option<thread::JoinHandle<()>>,
    done: Arc<AtomicBool>,
}

impl Drop for SupervisorHandle {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            self.done.store(true, Ordering::Release);
            h.thread().unpark();
            let _ = h.join();
        }
        clear_spinner();
    }
}

/// Spawn the supervisor only if it has work to do. Returns None when there are
/// no idle thresholds AND stderr isn't a TTY (no spinner, no warnings).
pub fn spawn_supervisor(
    task_name: String,
    pid: i32,
    activity: Arc<Activity>,
    idle_warn: Option<Duration>,
    idle_kill: Option<Duration>,
    killed_for_idle: Arc<AtomicBool>,
) -> Option<SupervisorHandle> {
    let tty = stderr_is_tty();
    if !tty && idle_warn.is_none() && idle_kill.is_none() {
        return None;
    }
    let color = color_enabled();
    let done = Arc::new(AtomicBool::new(false));
    let done_thread = Arc::clone(&done);

    let handle = thread::spawn(move || {
        let mut frame_idx: usize = 0;
        let mut last_warn_ms: u64 = 0;

        loop {
            if done_thread.load(Ordering::Acquire) {
                break;
            }
            let idle = activity.idle_for();
            let elapsed = activity.elapsed();
            let now_ms = elapsed.as_millis() as u64;

            match next_action(idle, idle_warn, idle_kill, last_warn_ms, now_ms) {
                Action::Kill => {
                    killed_for_idle.store(true, Ordering::Release);
                    // SAFETY: SIGTERM to the child's pid. PID-reuse race is
                    // documented and matches the existing wait_timeout->kill path.
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                    break;
                }
                Action::Warn => {
                    last_warn_ms = now_ms;
                    if !tty {
                        status_line(&format!(
                            "⏱ task {:?} idle for {}s (PID {pid})",
                            task_name,
                            idle.as_secs()
                        ));
                    }
                }
                Action::Nothing => {}
            }

            if tty {
                let frame = SPINNER_FRAMES[frame_idx % SPINNER_FRAMES.len()];
                frame_idx = frame_idx.wrapping_add(1);
                let line = format_spinner_line(
                    frame, &task_name, elapsed, idle, idle_warn, idle_kill, color,
                );
                write_spinner_frame(&line);
            }

            thread::park_timeout(SPINNER_TICK);
        }
    });

    Some(SupervisorHandle {
        handle: Some(handle),
        done,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_idle_grows_when_no_marks() {
        let a = Activity::new();
        a.mark_active();
        thread::sleep(Duration::from_millis(120));
        assert!(a.idle_for() >= Duration::from_millis(100));
    }

    #[test]
    fn activity_under_contention_is_fresh() {
        let a = Activity::new();
        let mut handles = vec![];
        for _ in 0..4 {
            let a = Arc::clone(&a);
            handles.push(thread::spawn(move || {
                for _ in 0..10_000 {
                    a.mark_active();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Last mark just happened; idle should be tiny.
        assert!(a.idle_for() < Duration::from_millis(50));
    }

    #[test]
    fn next_action_nothing_when_under_warn() {
        let a = next_action(
            Duration::from_secs(5),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(600)),
            0,
            5_000,
        );
        assert_eq!(a, Action::Nothing);
    }

    #[test]
    fn next_action_warns_at_threshold() {
        let a = next_action(
            Duration::from_secs(60),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(600)),
            0,
            60_000,
        );
        assert_eq!(a, Action::Warn);
    }

    #[test]
    fn next_action_warn_rate_limited() {
        // Warn just emitted at 60s; another check at 65s should be quiet.
        let a = next_action(
            Duration::from_secs(65),
            Some(Duration::from_secs(60)),
            None,
            60_000,
            65_000,
        );
        assert_eq!(a, Action::Nothing);
    }

    #[test]
    fn next_action_warn_repeats_after_interval() {
        // Warn at 60s; another check at 121s (>60s after) re-warns.
        let a = next_action(
            Duration::from_secs(121),
            Some(Duration::from_secs(60)),
            None,
            60_000,
            121_000,
        );
        assert_eq!(a, Action::Warn);
    }

    #[test]
    fn next_action_kill_takes_priority_over_warn() {
        let a = next_action(
            Duration::from_secs(600),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(600)),
            0,
            600_000,
        );
        assert_eq!(a, Action::Kill);
    }

    #[test]
    fn next_action_no_thresholds_is_nothing() {
        let a = next_action(Duration::from_secs(9999), None, None, 0, 9_999_000);
        assert_eq!(a, Action::Nothing);
    }

    #[test]
    fn spinner_line_plain_under_warn() {
        let s = format_spinner_line(
            '⠋',
            "foo",
            Duration::from_secs(12),
            Duration::from_secs(5),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(600)),
            true,
        );
        assert!(s.contains("running 12s"));
        assert!(!s.contains("idle"));
        assert!(!s.contains("\x1b["));
    }

    #[test]
    fn spinner_line_yellow_at_warn() {
        let s = format_spinner_line(
            '⠋',
            "foo",
            Duration::from_secs(75),
            Duration::from_secs(60),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(600)),
            true,
        );
        assert!(s.contains("[idle 60s]"));
        assert!(s.contains(ANSI_YELLOW));
    }

    #[test]
    fn spinner_line_red_near_kill() {
        // 0.8 * 100 = 80, so idle=85 should be red.
        let s = format_spinner_line(
            '⠋',
            "foo",
            Duration::from_secs(120),
            Duration::from_secs(85),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(100)),
            true,
        );
        assert!(s.contains(ANSI_RED));
        assert!(!s.contains(ANSI_YELLOW));
    }

    #[test]
    fn spinner_line_no_color_when_disabled() {
        let s = format_spinner_line(
            '⠋',
            "foo",
            Duration::from_secs(120),
            Duration::from_secs(85),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(100)),
            false,
        );
        assert!(!s.contains("\x1b["));
        assert!(s.contains("[idle 85s]"));
    }
}
