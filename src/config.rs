use crate::error::Error;
use crate::sandbox::SandboxConfig;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Defaults {
    pub tool: Option<String>,
    pub workdir: Option<PathBuf>,
    pub sandbox: Option<SandboxConfig>,
    pub timeout: Option<u64>,
    pub retry: Option<RetryConfig>,
    pub idle_warn: Option<u64>,
    pub idle_kill: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    Fixed,
    Linear,
    #[default]
    Exponential,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct RetryConfig {
    pub attempts: u32,
    pub backoff: BackoffStrategy,
    pub initial_delay: u64,
    pub max_delay: u64,
    pub on_timeout: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            attempts: 1,
            backoff: BackoffStrategy::Exponential,
            initial_delay: 1,
            max_delay: 30,
            on_timeout: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SandboxOrBool {
    Config(Box<SandboxConfig>),
    Disabled(bool),
}

#[derive(Debug, Deserialize)]
struct RawTask {
    tool: Option<String>,
    prompt: String,
    #[serde(default)]
    depends: Vec<String>,
    #[serde(default)]
    capture: bool,
    #[serde(default)]
    auto_approve: bool,
    #[serde(default)]
    files: Vec<PathBuf>,
    workdir: Option<PathBuf>,
    sandbox: Option<SandboxOrBool>,
    #[serde(default)]
    extra_args: Vec<String>,
    timeout: Option<u64>,
    retry: Option<RetryConfig>,
    idle_warn: Option<u64>,
    idle_kill: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub tool: Option<String>,
    pub prompt: String,
    pub depends: Vec<String>,
    pub capture: bool,
    pub auto_approve: bool,
    pub files: Vec<PathBuf>,
    pub workdir: Option<PathBuf>,
    pub sandbox: Option<Option<SandboxConfig>>,
    pub extra_args: Vec<String>,
    pub timeout: Option<u64>,
    pub retry: Option<RetryConfig>,
    pub idle_warn: Option<u64>,
    pub idle_kill: Option<u64>,
}

impl From<RawTask> for Task {
    fn from(raw: RawTask) -> Self {
        let sandbox = raw.sandbox.map(|s| match s {
            SandboxOrBool::Config(cfg) => Some(*cfg),
            SandboxOrBool::Disabled(false) => None,
            SandboxOrBool::Disabled(true) => Some(SandboxConfig::default()),
        });

        Self {
            tool: raw.tool,
            prompt: raw.prompt,
            depends: raw.depends,
            capture: raw.capture,
            auto_approve: raw.auto_approve,
            files: raw.files,
            workdir: raw.workdir,
            sandbox,
            extra_args: raw.extra_args,
            timeout: raw.timeout,
            retry: raw.retry,
            idle_warn: raw.idle_warn,
            idle_kill: raw.idle_kill,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    defaults: Defaults,
    #[serde(default)]
    tasks: BTreeMap<String, RawTask>,
    #[serde(default)]
    vars: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct Config {
    pub defaults: Defaults,
    pub tasks: BTreeMap<String, Task>,
    pub vars: BTreeMap<String, String>,
}

impl Config {
    pub fn from_str(s: &str, path: &Path) -> Result<Self, Error> {
        let raw: RawConfig = toml::from_str(s).map_err(|e| Error::ConfigParse {
            path: path.to_path_buf(),
            source: e,
        })?;

        let tasks: BTreeMap<String, Task> = raw
            .tasks
            .into_iter()
            .map(|(name, raw_task)| (name, Task::from(raw_task)))
            .collect();

        for (name, task) in &tasks {
            if let Some(retry) = &task.retry {
                validate_retry(name, retry)?;
            }
        }
        if let Some(retry) = &raw.defaults.retry {
            validate_retry("<defaults>", retry)?;
        }

        let vars = raw
            .vars
            .into_iter()
            .map(|(name, value)| {
                let resolved = substitute_commands(&name, &value)?;
                Ok((name, resolved))
            })
            .collect::<Result<BTreeMap<_, _>, Error>>()?;

        Ok(Self {
            defaults: raw.defaults,
            tasks,
            vars,
        })
    }

    pub fn load(path: &Path) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path).map_err(|e| Error::ConfigRead {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_str(&contents, path)
    }

    pub fn effective_tool(&self, task_name: &str) -> Result<String, Error> {
        let task = self
            .tasks
            .get(task_name)
            .ok_or_else(|| Error::UnknownTask(task_name.into()))?;

        task.tool
            .as_ref()
            .or(self.defaults.tool.as_ref())
            .cloned()
            .ok_or_else(|| Error::NoTool(task_name.into()))
    }

    pub fn effective_workdir(&self, task: &Task) -> Option<PathBuf> {
        task.workdir
            .as_ref()
            .or(self.defaults.workdir.as_ref())
            .cloned()
    }

    pub fn effective_sandbox(
        &self,
        task: &Task,
        force_sandbox: bool,
        no_sandbox: bool,
    ) -> Option<SandboxConfig> {
        if no_sandbox {
            return None;
        }

        let resolved = match &task.sandbox {
            Some(None) if !force_sandbox => None,

            Some(Some(cfg)) => Some(cfg.clone()),
            None => self.defaults.sandbox.clone(),
            Some(None) => Some(SandboxConfig::default()),
        };

        if force_sandbox && resolved.is_none() {
            Some(SandboxConfig::default())
        } else {
            resolved
        }
    }

    pub fn effective_timeout(&self, task: &Task) -> Option<Duration> {
        task.timeout
            .or(self.defaults.timeout)
            .map(Duration::from_secs)
    }

    pub fn effective_retry(&self, task: &Task) -> Option<RetryConfig> {
        task.retry.clone().or_else(|| self.defaults.retry.clone())
    }

    pub fn effective_idle_warn(&self, task: &Task) -> Option<Duration> {
        task.idle_warn
            .or(self.defaults.idle_warn)
            .map(Duration::from_secs)
    }

    pub fn effective_idle_kill(&self, task: &Task) -> Option<Duration> {
        task.idle_kill
            .or(self.defaults.idle_kill)
            .map(Duration::from_secs)
    }
}

fn validate_retry(scope: &str, retry: &RetryConfig) -> Result<(), Error> {
    if retry.attempts == 0 {
        return Err(Error::InvalidRetryConfig {
            task: scope.into(),
            reason: "attempts must be >= 1".into(),
        });
    }
    if retry.max_delay < retry.initial_delay {
        return Err(Error::InvalidRetryConfig {
            task: scope.into(),
            reason: format!(
                "max_delay ({}) must be >= initial_delay ({})",
                retry.max_delay, retry.initial_delay
            ),
        });
    }
    Ok(())
}

fn substitute_commands(name: &str, value: &str) -> Result<String, Error> {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'(') {
            chars.next();
            let mut cmd = String::new();
            let mut depth = 1;
            loop {
                match chars.next() {
                    Some('(') => {
                        depth += 1;
                        cmd.push('(');
                    }
                    Some(')') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        cmd.push(')');
                    }
                    Some(c) => cmd.push(c),
                    None => return Err(Error::VarCommandUnclosed { name: name.into() }),
                }
            }
            let output = run_shell(name, &cmd)?;
            result.push_str(output.trim_end_matches('\n'));
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

fn run_shell(var_name: &str, command: &str) -> Result<String, Error> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| Error::VarCommandFailed {
            name: var_name.into(),
            command: command.into(),
            status: -1,
            stderr: format!("failed to spawn sh: {e}"),
        })?;

    if !output.status.success() {
        return Err(Error::VarCommandFailed {
            name: var_name.into(),
            command: command.into(),
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn find_amakefile() -> Result<PathBuf, Error> {
    let cwd = std::env::current_dir()?;
    let mut dir = cwd.as_path();

    loop {
        let amakefile = dir.join("Amakefile");
        if amakefile.is_file() {
            return Ok(amakefile);
        }
        let alt = dir.join("amake.toml");
        if alt.is_file() {
            return Ok(alt);
        }
        dir = dir.parent().ok_or(Error::ConfigNotFound)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[tasks.hello]
prompt = "Say hello"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        assert!(cfg.tasks.contains_key("hello"));
        assert_eq!(cfg.tasks["hello"].prompt, "Say hello");
    }

    #[test]
    fn parse_full_config() {
        let toml = r#"
[defaults]
tool = "claude-code"
workdir = "./src"

[tasks.create-pr]
tool = "claude-code"
prompt = "Create a PR."
capture = true
auto_approve = true
depends = []
files = ["README.md"]
extra_args = ["--verbose"]

[tasks.create-pr.sandbox]
agent_policy = "deny"
agent_allow = ["api.github.com"]
memory = "4g"
cpus = "4"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let task = &cfg.tasks["create-pr"];
        assert_eq!(task.tool.as_deref(), Some("claude-code"));
        assert!(task.capture);
        assert!(task.auto_approve);
        assert_eq!(task.files, vec![PathBuf::from("README.md")]);

        let sandbox = task.sandbox.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(sandbox.agent_policy.as_deref(), Some("deny"));
        assert_eq!(sandbox.memory.as_deref(), Some("4g"));
    }

    #[test]
    fn sandbox_false_disables() {
        let toml = r#"
[defaults.sandbox]
agent_policy = "deny"

[tasks.nosandbox]
prompt = "No sandbox"
sandbox = false
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let task = &cfg.tasks["nosandbox"];
        // Task explicitly disabled sandbox
        assert_eq!(task.sandbox, Some(None));

        let effective = cfg.effective_sandbox(task, false, false);
        assert!(effective.is_none());
    }

    #[test]
    fn defaults_resolution() {
        let toml = r#"
[defaults]
tool = "aider"

[tasks.lint]
prompt = "Fix lint"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        assert_eq!(cfg.effective_tool("lint").unwrap(), "aider");
    }

    #[test]
    fn task_tool_overrides_default() {
        let toml = r#"
[defaults]
tool = "aider"

[tasks.lint]
tool = "claude-code"
prompt = "Fix lint"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        assert_eq!(cfg.effective_tool("lint").unwrap(), "claude-code");
    }

    #[test]
    fn no_tool_errors() {
        let toml = r#"
[tasks.lint]
prompt = "Fix lint"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        assert!(matches!(cfg.effective_tool("lint"), Err(Error::NoTool(_))));
    }

    #[test]
    fn force_sandbox_overrides_disabled() {
        let toml = r#"
[tasks.test]
prompt = "Test"
sandbox = false
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let task = &cfg.tasks["test"];
        let effective = cfg.effective_sandbox(task, true, false);
        assert!(effective.is_some());
    }

    #[test]
    fn no_sandbox_overrides_config() {
        let toml = r#"
[tasks.test]
prompt = "Test"

[tasks.test.sandbox]
agent_policy = "deny"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let task = &cfg.tasks["test"];
        let effective = cfg.effective_sandbox(task, false, true);
        assert!(effective.is_none());
    }

    #[test]
    fn parses_vars_table() {
        let toml = r#"
[vars]
greeting = "hello"
target = "world"

[tasks.t]
prompt = "x"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        assert_eq!(cfg.vars["greeting"], "hello");
        assert_eq!(cfg.vars["target"], "world");
    }

    #[test]
    fn vars_command_substitution() {
        let toml = r#"
[vars]
who = "$(echo alice)"
greeting = "hi $(echo there)!"

[tasks.t]
prompt = "x"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        assert_eq!(cfg.vars["who"], "alice");
        assert_eq!(cfg.vars["greeting"], "hi there!");
    }

    #[test]
    fn vars_command_substitution_failure_errors() {
        let toml = r#"
[vars]
broken = "$(false)"

[tasks.t]
prompt = "x"
"#;
        let result = Config::from_str(toml, Path::new("Amakefile"));
        assert!(matches!(result, Err(Error::VarCommandFailed { .. })));
    }

    #[test]
    fn vars_unclosed_paren_errors() {
        let toml = r#"
[vars]
broken = "$(echo hi"

[tasks.t]
prompt = "x"
"#;
        let result = Config::from_str(toml, Path::new("Amakefile"));
        assert!(matches!(result, Err(Error::VarCommandUnclosed { .. })));
    }

    #[test]
    fn inherits_default_sandbox() {
        let toml = r#"
[defaults.sandbox]
agent_policy = "deny"
memory = "8g"

[tasks.test]
prompt = "Test"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let task = &cfg.tasks["test"];
        let effective = cfg.effective_sandbox(task, false, false).unwrap();
        assert_eq!(effective.agent_policy.as_deref(), Some("deny"));
        assert_eq!(effective.memory.as_deref(), Some("8g"));
    }

    #[test]
    fn parses_full_retry_config() {
        let toml = r#"
[tasks.t]
prompt = "x"
timeout = 90

[tasks.t.retry]
attempts = 4
backoff = "linear"
initial_delay = 2
max_delay = 20
on_timeout = false
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let task = &cfg.tasks["t"];
        assert_eq!(task.timeout, Some(90));
        let retry = task.retry.as_ref().unwrap();
        assert_eq!(retry.attempts, 4);
        assert_eq!(retry.backoff, BackoffStrategy::Linear);
        assert_eq!(retry.initial_delay, 2);
        assert_eq!(retry.max_delay, 20);
        assert!(!retry.on_timeout);
    }

    #[test]
    fn partial_retry_table_fills_defaults() {
        let toml = r#"
[tasks.t]
prompt = "x"

[tasks.t.retry]
attempts = 3
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let retry = cfg.tasks["t"].retry.as_ref().unwrap();
        assert_eq!(retry.attempts, 3);
        assert_eq!(retry.backoff, BackoffStrategy::Exponential);
        assert_eq!(retry.initial_delay, 1);
        assert_eq!(retry.max_delay, 30);
        assert!(retry.on_timeout);
    }

    #[test]
    fn rejects_zero_attempts() {
        let toml = r#"
[tasks.t]
prompt = "x"

[tasks.t.retry]
attempts = 0
"#;
        let result = Config::from_str(toml, Path::new("Amakefile"));
        assert!(matches!(result, Err(Error::InvalidRetryConfig { .. })));
    }

    #[test]
    fn rejects_max_below_initial() {
        let toml = r#"
[tasks.t]
prompt = "x"

[tasks.t.retry]
attempts = 2
initial_delay = 10
max_delay = 5
"#;
        let result = Config::from_str(toml, Path::new("Amakefile"));
        assert!(matches!(result, Err(Error::InvalidRetryConfig { .. })));
    }

    #[test]
    fn task_timeout_overrides_default() {
        let toml = r#"
[defaults]
timeout = 30

[tasks.t]
prompt = "x"
timeout = 5
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let effective = cfg.effective_timeout(&cfg.tasks["t"]).unwrap();
        assert_eq!(effective, Duration::from_secs(5));
    }

    #[test]
    fn default_timeout_inherited_when_task_unset() {
        let toml = r#"
[defaults]
timeout = 42

[tasks.t]
prompt = "x"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let effective = cfg.effective_timeout(&cfg.tasks["t"]).unwrap();
        assert_eq!(effective, Duration::from_secs(42));
    }

    #[test]
    fn task_retry_overrides_default() {
        let toml = r#"
[defaults.retry]
attempts = 2
backoff = "fixed"

[tasks.t]
prompt = "x"

[tasks.t.retry]
attempts = 5
backoff = "exponential"
"#;
        let cfg = Config::from_str(toml, Path::new("Amakefile")).unwrap();
        let effective = cfg.effective_retry(&cfg.tasks["t"]).unwrap();
        assert_eq!(effective.attempts, 5);
        assert_eq!(effective.backoff, BackoffStrategy::Exponential);
    }
}
