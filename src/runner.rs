use crate::adapter::AdapterRegistry;
use crate::config::Config;
use crate::error::Error;
use crate::template;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read as _;
use std::process::Stdio;

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
    pub vars: BTreeMap<String, String>,
}

pub fn run(config: &Config, targets: &[String], opts: &RunOptions) -> Result<(), Error> {
    let order = resolve_execution_order(config, targets)?;

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

        let cmd = adapter.build_command(
            &rendered_task,
            workdir.as_deref(),
            task.auto_approve,
            sandbox.as_ref(),
        );

        if opts.dry_run {
            print_command(task_name, &cmd);
            continue;
        }

        eprintln!("▶ running task: {task_name} (tool: {tool})");

        let cmd_string = format_command(&cmd);

        match execute_task(task_name, cmd, task.capture)? {
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
}

fn execute_task(
    task_name: &str,
    mut cmd: std::process::Command,
    capture: bool,
) -> Result<TaskResult, Error> {
    if capture {
        cmd.stdout(Stdio::piped());
    }
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        eprintln!("✗ failed to start task {task_name:?}: {e}");
        e
    })?;

    let stdout = if capture {
        let mut output = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut output)?;
        }
        Some(output)
    } else {
        None
    };

    let mut stderr_output = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        stderr.read_to_string(&mut stderr_output)?;
    }

    // Always forward stderr so the user sees it in real-time context
    if !stderr_output.is_empty() {
        eprint!("{stderr_output}");
    }

    let status = child.wait()?;

    if status.success() {
        Ok(TaskResult::Success(stdout))
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
        .map(|a| {
            let s = a.to_string_lossy();
            if s.contains(' ') || s.contains('"') || s.is_empty() {
                format!("{s:?}")
            } else {
                s.into_owned()
            }
        })
        .collect();

    format!("{program} {}", args.join(" "))
}

fn print_command(task_name: &str, cmd: &std::process::Command) {
    println!("[{task_name}] {}", format_command(cmd));
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
}
