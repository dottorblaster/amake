mod aider;
mod claude_code;
mod copilot;
mod generic;
mod sandbox;

use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub use generic::GenericPassthrough;

pub trait Adapter: Send + Sync {
    fn name(&self) -> &str;

    fn clampdown_agent(&self) -> Option<&str>;

    fn build_command(
        &self,
        task: &Task,
        workdir: Option<&Path>,
        auto_approve: bool,
        sandbox: Option<&SandboxConfig>,
    ) -> Command;
}

pub struct AdapterRegistry {
    adapters: BTreeMap<String, Box<dyn Adapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        let mut adapters: BTreeMap<String, Box<dyn Adapter>> = BTreeMap::new();
        adapters.insert("claude-code".into(), Box::new(claude_code::ClaudeCodeAdapter));
        adapters.insert("aider".into(), Box::new(aider::AiderAdapter));
        adapters.insert("copilot".into(), Box::new(copilot::CopilotAdapter));
        Self { adapters }
    }

    pub fn get(&self, name: &str) -> Option<&dyn Adapter> {
        self.adapters.get(name).map(|a| a.as_ref())
    }

    pub fn builtin_names(&self) -> Vec<&str> {
        self.adapters.keys().map(|s| s.as_str()).collect()
    }

    pub fn resolve_or_generic(&self, name: &str) -> ResolvedAdapter<'_> {
        match self.get(name) {
            Some(a) => ResolvedAdapter::Builtin(a),
            None => ResolvedAdapter::Generic(GenericPassthrough::new(name)),
        }
    }
}

pub enum ResolvedAdapter<'a> {
    Builtin(&'a dyn Adapter),
    Generic(GenericPassthrough),
}

impl<'a> ResolvedAdapter<'a> {
    pub fn adapter(&self) -> &dyn Adapter {
        match self {
            Self::Builtin(a) => *a,
            Self::Generic(a) => a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Task;
    use std::path::PathBuf;

    fn make_task(prompt: &str) -> Task {
        Task {
            tool: None,
            prompt: prompt.into(),
            depends: vec![],
            capture: false,
            auto_approve: false,
            files: vec![],
            workdir: None,
            sandbox: None,
            extra_args: vec![],
        }
    }

    fn get_args(cmd: &Command) -> Vec<&std::ffi::OsStr> {
        cmd.get_args().collect()
    }

    #[test]
    fn claude_code_basic() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let task = make_task("Hello");
        let cmd = adapter.build_command(&task, None, false, None);
        assert_eq!(cmd.get_program(), "claude");
        let args = get_args(&cmd);
        assert_eq!(args, &["--print", "Hello"]);
    }

    #[test]
    fn claude_code_auto_approve() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let task = make_task("Hello");
        let cmd = adapter.build_command(&task, None, true, None);
        let args = get_args(&cmd);
        assert_eq!(args, &["--dangerously-skip-permissions", "--print", "Hello"]);
    }

    #[test]
    fn claude_code_with_files() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let mut task = make_task("Hello");
        task.files = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_eq!(args, &["--print", "--file", "a.rs", "--file", "b.rs", "Hello"]);
    }

    #[test]
    fn aider_basic() {
        let adapter = aider::AiderAdapter;
        let task = make_task("Fix it");
        let cmd = adapter.build_command(&task, None, true, None);
        assert_eq!(cmd.get_program(), "aider");
        let args = get_args(&cmd);
        assert_eq!(args, &["--yes", "--message", "Fix it"]);
    }

    #[test]
    fn aider_with_files_and_extra_args() {
        let adapter = aider::AiderAdapter;
        let mut task = make_task("Fix it");
        task.files = vec![PathBuf::from("src/main.rs")];
        task.extra_args = vec!["--model".into(), "gpt-4".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_eq!(args, &["--message", "Fix it", "--file", "src/main.rs", "--model", "gpt-4"]);
    }

    #[test]
    fn copilot_basic() {
        let adapter = copilot::CopilotAdapter;
        let task = make_task("Suggest");
        let cmd = adapter.build_command(&task, None, false, None);
        assert_eq!(cmd.get_program(), "gh");
        let args = get_args(&cmd);
        assert_eq!(args, &["copilot", "suggest", "-t", "shell", "Suggest"]);
    }

    #[test]
    fn generic_passthrough() {
        let adapter = GenericPassthrough::new("mytool");
        let mut task = make_task("Do stuff");
        task.extra_args = vec!["--fast".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        assert_eq!(cmd.get_program(), "mytool");
        let args = get_args(&cmd);
        assert_eq!(args, &["--fast", "Do stuff"]);
    }

    #[test]
    fn claude_code_sandboxed() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let task = make_task("Create PR");
        let sandbox = SandboxConfig {
            agent_policy: Some("deny".into()),
            agent_allow: vec!["api.github.com".into()],
            memory: Some("4g".into()),
            cpus: Some("4".into()),
            ..Default::default()
        };
        let workdir = PathBuf::from("/project");
        let cmd = adapter.build_command(&task, Some(&workdir), true, Some(&sandbox));

        assert_eq!(cmd.get_program(), "clampdown");
        let args: Vec<String> = get_args(&cmd).iter().map(|a| a.to_string_lossy().into()).collect();
        assert_eq!(
            args,
            &[
                "claude",
                "--agent-policy", "deny",
                "--agent-allow", "api.github.com",
                "--memory", "4g",
                "--cpus", "4",
                "--workdir", "/project",
                "--",
                "--dangerously-skip-permissions",
                "--print",
                "Create PR",
            ]
        );
    }

    #[test]
    fn unsupported_tool_sandbox_falls_back() {
        let adapter = aider::AiderAdapter;
        let task = make_task("Fix");
        let sandbox = SandboxConfig::default();
        let cmd = adapter.build_command(&task, None, true, Some(&sandbox));
        assert_eq!(cmd.get_program(), "aider");
    }

    #[test]
    fn registry_builtin_names() {
        let registry = AdapterRegistry::new();
        let names = registry.builtin_names();
        assert!(names.contains(&"claude-code"));
        assert!(names.contains(&"aider"));
        assert!(names.contains(&"copilot"));
    }

    #[test]
    fn registry_generic_fallback() {
        let registry = AdapterRegistry::new();
        let resolved = registry.resolve_or_generic("unknown-tool");
        assert_eq!(resolved.adapter().name(), "unknown-tool");
    }

    #[test]
    fn workdir_set_when_not_sandboxed() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let task = make_task("Hello");
        let workdir = PathBuf::from("/my/project");
        let cmd = adapter.build_command(&task, Some(&workdir), false, None);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/my/project")));
    }
}
