use crate::adapter::AdapterRegistry;
use crate::config::{BackoffStrategy, Config, RetryConfig};
use crate::error::Error;
use crate::render::{self, Assets, StreamingRenderer};
use crate::template;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Write as _};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use wait_timeout::ChildExt;

#[derive(Clone)]
enum RenderMode {
    Off,
    On(Arc<Assets>),
}

pub fn resolve_execution_order(config: &Config, targets: &[String]) -> Result<Vec<String>, Error> {
    for target in targets {
        if !config.tasks.contains_key(target) {
            return Err(Error::UnknownTask(target.clone()));
        }
    }

    let mut needed: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = targets.iter().cloned().collect();

    while let Some(name) = queue.pop_front() {
        if needed.contains(&name) {
            continue;
        }
        let task = config
            .tasks
            .get(&name)
            .ok_or_else(|| Error::UnknownTask(name.clone()))?;
        needed.insert(name);
        for dep in &task.depends {
            queue.push_back(dep.clone());
        }
    }

    let mut in_degree: BTreeMap<&str, usize> = needed
        .iter()
        .map(|name| {
            let deps_count = config.tasks[name]
                .depends
                .iter()
                .filter(|d| needed.contains(*d))
                .count();
            (name.as_str(), deps_count)
        })
        .collect();

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(&name, _)| name)
        .collect();

    let mut order = Vec::with_capacity(needed.len());

    while let Some(name) = queue.pop_front() {
        order.push(name.to_string());

        let dependents: Vec<&str> = needed
            .iter()
            .filter(|n| config.tasks[*n].depends.iter().any(|d| d == name))
            .map(|n| n.as_str())
            .collect();

        for dep in dependents {
            if let Some(deg) = in_degree.get_mut(dep) {
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(dep);
                }
            }
        }
    }

    if order.len() != needed.len() {
        let remaining: Vec<&str> = needed
            .iter()
            .filter(|n| !order.contains(n))
            .map(|n| n.as_str())
            .collect();
        return Err(Error::DependencyCycle(remaining.join(" -> ")));
    }

    Ok(order)
}

fn check_clampdown() -> Result<(), Error> {
    which("clampdown").ok_or(Error::ClampdownNotFound)
}

fn which(binary: &str) -> Option<()> {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(binary))
                .find(|path| path.is_file())
        })
        .map(|_| ())
}

pub struct RunOptions {
    pub dry_run: bool,
    pub keep_going: bool,
    pub force_sandbox: bool,
    pub no_sandbox: bool,
    pub no_format: bool,
    pub vars: BTreeMap<String, String>,
}

pub fn run(config: &Config, targets: &[String], opts: &RunOptions) -> Result<(), Error> {
    let order = resolve_execution_order(config, targets)?;

    let render_mode = if render::should_render(opts.no_format) {
        RenderMode::On(Arc::new(Assets::load()))
    } else {
        RenderMode::Off
    };

    let capture_flags: BTreeMap<String, bool> = config
        .tasks
        .iter()
        .map(|(name, task)| (name.clone(), task.capture))
        .collect();

    let registry = AdapterRegistry::new();
    let mut task_outputs: BTreeMap<String, String> = BTreeMap::new();
    let mut sandbox_checked = false;
    let mut failures: Vec<String> = Vec::new();

    for task_name in &order {
        let task = &config.tasks[task_name];
        let tool = config.effective_tool(task_name)?;
        let workdir = config.effective_workdir(task);
        let sandbox = config.effective_sandbox(task, opts.force_sandbox, opts.no_sandbox);
        let timeout = config.effective_timeout(task);
        let retry = config.effective_retry(task);

        if sandbox.is_some() && !sandbox_checked {
            check_clampdown()?;
            sandbox_checked = true;
        }

        let rendered_prompt = template::render(
            &task.prompt,
            task_name,
            &task_outputs,
            &opts.vars,
            &task.depends,
            &capture_flags,
        )?;

        let mut rendered_task = task.clone();
        rendered_task.prompt = rendered_prompt;

        let resolved = registry.resolve_or_generic(&tool);
        let adapter = resolved.adapter();

        let workdir_ref = workdir.as_deref();
        let sandbox_ref = sandbox.as_ref();
        let auto_approve = task.auto_approve;
        let cmd_builder =
            || adapter.build_command(&rendered_task, workdir_ref, auto_approve, sandbox_ref);

        if opts.dry_run {
            let cmd = cmd_builder();
            print_command(task_name, &cmd, timeout, retry.as_ref());
            continue;
        }

        eprintln!("▶ running task: {task_name} (tool: {tool})");

        let cmd_string = format_command(&cmd_builder());

        let (result, attempts) = execute_with_retry(
            task_name,
            cmd_builder,
            task.capture,
            timeout,
            retry.as_ref(),
            &render_mode,
        )?;

        match result {
            TaskResult::Success(output) => {
                if let Some(stdout) = output {
                    task_outputs.insert(task_name.clone(), stdout);
                }
            }
            TaskResult::Failed(code, stderr_tail) => {
                if opts.keep_going {
                    eprintln!("✗ task {task_name:?} failed (exit code {code}), continuing...");
                    failures.push(task_name.clone());
                } else {
                    return Err(Error::TaskFailed {
                        task: task_name.clone(),
                        code,
                        attempts,
                        command: Some(cmd_string),
                        stderr_tail: Some(stderr_tail),
                    });
                }
            }
            TaskResult::Signaled(stderr_tail) => {
                if opts.keep_going {
                    eprintln!("✗ task {task_name:?} was killed by a signal, continuing...");
                    failures.push(task_name.clone());
                } else {
                    return Err(Error::TaskSignaled {
                        task: task_name.clone(),
                        attempts,
                        command: Some(cmd_string),
                        stderr_tail: Some(stderr_tail),
                    });
                }
            }
            TaskResult::TimedOut(stderr_tail) => {
                let timeout_secs = timeout.map(|d| d.as_secs()).unwrap_or(0);
                if opts.keep_going {
                    eprintln!(
                        "✗ task {task_name:?} timed out after {timeout_secs}s, continuing..."
                    );
                    failures.push(task_name.clone());
                } else {
                    return Err(Error::TaskTimeout {
                        task: task_name.clone(),
                        timeout_secs,
                        attempts,
                        command: Some(cmd_string),
                        stderr_tail: Some(stderr_tail),
                    });
                }
            }
        }
    }

    if !failures.is_empty() {
        eprintln!(
            "\n✗ {} task(s) failed: {}",
            failures.len(),
            failures.join(", ")
        );
        return Err(Error::TaskFailed {
            task: failures.join(", "),
            code: 1,
            attempts: 1,
            command: None,
            stderr_tail: None,
        });
    }

    Ok(())
}

enum TaskResult {
    Success(Option<String>),
    Failed(i32, String),
    Signaled(String),
    TimedOut(String),
}

fn execute_with_retry(
    task_name: &str,
    cmd_builder: impl Fn() -> std::process::Command,
    capture: bool,
    timeout: Option<Duration>,
    retry: Option<&RetryConfig>,
    render_mode: &RenderMode,
) -> Result<(TaskResult, u32), Error> {
    let max_attempts = retry.map(|r| r.attempts).unwrap_or(1).max(1);
    let on_timeout_retry = retry.map(|r| r.on_timeout).unwrap_or(true);

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let cmd = cmd_builder();
        let result = execute_attempt(task_name, cmd, capture, timeout, render_mode)?;

        let should_retry = match &result {
            TaskResult::Success(_) => false,
            TaskResult::TimedOut(_) => on_timeout_retry && attempt < max_attempts,
            TaskResult::Failed(_, _) | TaskResult::Signaled(_) => attempt < max_attempts,
        };

        if !should_retry {
            return Ok((result, attempt));
        }

        let cfg = retry.expect("retry must be Some when attempt < max_attempts");
        let delay = compute_backoff(cfg, attempt);
        let kind = describe_failure(&result, timeout);
        eprintln!(
            "⟲ task {task_name:?} {kind} (attempt {attempt}/{max_attempts}), retrying in {}s...",
            delay.as_secs()
        );
        std::thread::sleep(delay);
    }
}

fn describe_failure(result: &TaskResult, timeout: Option<Duration>) -> String {
    match result {
        TaskResult::Failed(code, _) => format!("failed (exit {code})"),
        TaskResult::Signaled(_) => "was killed by a signal".to_string(),
        TaskResult::TimedOut(_) => {
            let secs = timeout.map(|d| d.as_secs()).unwrap_or(0);
            format!("timed out after {secs}s")
        }
        TaskResult::Success(_) => unreachable!("Success doesn't trigger retry"),
    }
}

fn compute_backoff(cfg: &RetryConfig, attempt: u32) -> Duration {
    let secs = match cfg.backoff {
        BackoffStrategy::Fixed => cfg.initial_delay,
        BackoffStrategy::Linear => cfg.initial_delay.saturating_mul(attempt as u64),
        BackoffStrategy::Exponential => {
            let exp = (attempt - 1).min(63);
            cfg.initial_delay.saturating_mul(2u64.saturating_pow(exp))
        }
    };
    Duration::from_secs(secs.min(cfg.max_delay))
}

fn execute_attempt(
    task_name: &str,
    mut cmd: std::process::Command,
    capture: bool,
    timeout: Option<Duration>,
    render_mode: &RenderMode,
) -> Result<TaskResult, Error> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        eprintln!("✗ failed to start task {task_name:?}: {e}");
        e
    })?;

    let stderr_handle = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || -> std::io::Result<String> {
            let reader = BufReader::new(stderr);
            let mut accumulated = String::new();
            for line in reader.lines() {
                let line = line?;
                eprintln!("{line}");
                accumulated.push_str(&line);
                accumulated.push('\n');
            }
            Ok(accumulated)
        })
    });

    let stdout_handle = child.stdout.take().map(|stdout| {
        let render_mode = render_mode.clone();
        let want_capture = capture;
        std::thread::spawn(move || -> std::io::Result<Option<String>> {
            let mut accumulated = if want_capture {
                Some(String::new())
            } else {
                None
            };
            let stdout_lock = std::io::stdout().lock();
            let mut renderer = match render_mode {
                RenderMode::On(assets) => Some(StreamingRenderer::new(stdout_lock, assets)),
                RenderMode::Off => None,
            };
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = line?;
                if let Some(r) = renderer.as_mut() {
                    r.push_line(&line);
                } else {
                    println!("{line}");
                }
                if let Some(s) = accumulated.as_mut() {
                    s.push_str(&line);
                    s.push('\n');
                }
            }
            if let Some(r) = renderer.as_mut() {
                r.finish();
            }
            // Best-effort flush so any buffered ANSI lands before the next task's banner.
            let _ = std::io::stdout().flush();
            Ok(accumulated)
        })
    });

    let (status, timed_out) = match timeout {
        Some(d) => match child.wait_timeout(d)? {
            Some(s) => (s, false),
            None => {
                let _ = child.kill();
                let s = child.wait()?;
                (s, true)
            }
        },
        None => (child.wait()?, false),
    };

    let stderr_output = match stderr_handle {
        Some(handle) => handle.join().expect("stderr reader thread panicked")?,
        None => String::new(),
    };

    let stdout_output = match stdout_handle {
        Some(handle) => handle.join().expect("stdout reader thread panicked")?,
        None => None,
    };

    if timed_out {
        return Ok(TaskResult::TimedOut(stderr_tail(&stderr_output, 20)));
    }

    if status.success() {
        Ok(TaskResult::Success(stdout_output))
    } else {
        let tail = stderr_tail(&stderr_output, 20);
        match status.code() {
            Some(code) => Ok(TaskResult::Failed(code, tail)),
            None => Ok(TaskResult::Signaled(tail)),
        }
    }
}

/// Return the last `n` lines of `s`, trimmed.
fn stderr_tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

fn format_command(cmd: &std::process::Command) -> String {
    let program = cmd.get_program().to_string_lossy();
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| shell_quote(&a.to_string_lossy()))
        .collect();

    format!("{program} {}", args.join(" "))
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '=' | ':' | ','));
    if safe {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn print_command(
    task_name: &str,
    cmd: &std::process::Command,
    timeout: Option<Duration>,
    retry: Option<&RetryConfig>,
) {
    let mut annotations = Vec::new();
    if let Some(t) = timeout {
        annotations.push(format!("timeout {}s", t.as_secs()));
    }
    if let Some(r) = retry
        && r.attempts > 1
    {
        let backoff = match r.backoff {
            BackoffStrategy::Fixed => "fixed",
            BackoffStrategy::Linear => "linear",
            BackoffStrategy::Exponential => "exponential",
        };
        annotations.push(format!("retry {}x {backoff}", r.attempts));
    }
    let suffix = if annotations.is_empty() {
        String::new()
    } else {
        format!("  ({})", annotations.join(", "))
    };
    println!("[{task_name}] {}{suffix}", format_command(cmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::Path;

    fn parse_config(toml: &str) -> Config {
        Config::from_str(toml, Path::new("Amakefile")).unwrap()
    }

    #[test]
    fn simple_order() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
[tasks.b]
prompt = "B"
depends = ["a"]
"#,
        );
        let order = resolve_execution_order(&cfg, &["b".into()]).unwrap();
        assert_eq!(order, &["a", "b"]);
    }

    #[test]
    fn diamond_deps() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
[tasks.b]
prompt = "B"
depends = ["a"]
[tasks.c]
prompt = "C"
depends = ["a"]
[tasks.d]
prompt = "D"
depends = ["b", "c"]
"#,
        );
        let order = resolve_execution_order(&cfg, &["d".into()]).unwrap();
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let c_pos = order.iter().position(|x| x == "c").unwrap();
        let d_pos = order.iter().position(|x| x == "d").unwrap();
        assert!(a_pos < b_pos);
        assert!(a_pos < c_pos);
        assert!(b_pos < d_pos);
        assert!(c_pos < d_pos);
    }

    #[test]
    fn cycle_detected() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
depends = ["b"]
[tasks.b]
prompt = "B"
depends = ["a"]
"#,
        );
        let result = resolve_execution_order(&cfg, &["a".into()]);
        assert!(matches!(result, Err(Error::DependencyCycle(_))));
    }

    #[test]
    fn unknown_task_error() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
"#,
        );
        let result = resolve_execution_order(&cfg, &["nonexistent".into()]);
        assert!(matches!(result, Err(Error::UnknownTask(_))));
    }

    #[test]
    fn no_deps_single_task() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
"#,
        );
        let order = resolve_execution_order(&cfg, &["a".into()]).unwrap();
        assert_eq!(order, &["a"]);
    }

    #[test]
    fn multiple_targets() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
[tasks.b]
prompt = "B"
"#,
        );
        let order = resolve_execution_order(&cfg, &["a".into(), "b".into()]).unwrap();
        assert!(order.contains(&"a".to_string()));
        assert!(order.contains(&"b".to_string()));
    }

    #[test]
    fn unknown_dep_error() {
        let cfg = parse_config(
            r#"
[tasks.a]
prompt = "A"
depends = ["nonexistent"]
"#,
        );
        let result = resolve_execution_order(&cfg, &["a".into()]);
        assert!(matches!(result, Err(Error::UnknownTask(_))));
    }

    fn retry(backoff: BackoffStrategy, initial: u64, max: u64) -> RetryConfig {
        RetryConfig {
            attempts: 5,
            backoff,
            initial_delay: initial,
            max_delay: max,
            on_timeout: true,
        }
    }

    #[test]
    fn backoff_fixed_is_constant() {
        let cfg = retry(BackoffStrategy::Fixed, 2, 60);
        assert_eq!(compute_backoff(&cfg, 1), Duration::from_secs(2));
        assert_eq!(compute_backoff(&cfg, 4), Duration::from_secs(2));
    }

    #[test]
    fn backoff_linear_scales_with_attempt() {
        let cfg = retry(BackoffStrategy::Linear, 3, 60);
        assert_eq!(compute_backoff(&cfg, 1), Duration::from_secs(3));
        assert_eq!(compute_backoff(&cfg, 2), Duration::from_secs(6));
        assert_eq!(compute_backoff(&cfg, 3), Duration::from_secs(9));
    }

    #[test]
    fn backoff_exponential_doubles() {
        let cfg = retry(BackoffStrategy::Exponential, 1, 60);
        assert_eq!(compute_backoff(&cfg, 1), Duration::from_secs(1));
        assert_eq!(compute_backoff(&cfg, 2), Duration::from_secs(2));
        assert_eq!(compute_backoff(&cfg, 3), Duration::from_secs(4));
        assert_eq!(compute_backoff(&cfg, 4), Duration::from_secs(8));
    }

    #[test]
    fn backoff_caps_at_max_delay() {
        let cfg = retry(BackoffStrategy::Exponential, 1, 5);
        assert_eq!(compute_backoff(&cfg, 1), Duration::from_secs(1));
        assert_eq!(compute_backoff(&cfg, 2), Duration::from_secs(2));
        assert_eq!(compute_backoff(&cfg, 3), Duration::from_secs(4));
        assert_eq!(compute_backoff(&cfg, 4), Duration::from_secs(5));
        assert_eq!(compute_backoff(&cfg, 30), Duration::from_secs(5));
    }
}
