use crate::error::Error;
use crate::sandbox::SandboxConfig;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Defaults {
    pub tool: Option<String>,
    pub workdir: Option<PathBuf>,
    pub sandbox: Option<SandboxConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SandboxOrBool {
    Config(SandboxConfig),
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
}

impl From<RawTask> for Task {
    fn from(raw: RawTask) -> Self {
        let sandbox = raw.sandbox.map(|s| match s {
            SandboxOrBool::Config(cfg) => Some(cfg),
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
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    defaults: Defaults,
    #[serde(default)]
    tasks: BTreeMap<String, RawTask>,
}

#[derive(Debug)]
pub struct Config {
    pub defaults: Defaults,
    pub tasks: BTreeMap<String, Task>,
}

impl Config {
    pub fn from_str(s: &str, path: &Path) -> Result<Self, Error> {
        let raw: RawConfig =
            toml::from_str(s).map_err(|e| Error::ConfigParse {
                path: path.to_path_buf(),
                source: e,
            })?;

        let tasks = raw
            .tasks
            .into_iter()
            .map(|(name, raw_task)| (name, Task::from(raw_task)))
            .collect();

        Ok(Self {
            defaults: raw.defaults,
            tasks,
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
}
