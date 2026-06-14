mod aider;
mod claude_code;
mod copilot;
mod generic;
mod pi;
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
        adapters.insert(
            "claude-code".into(),
            Box::new(claude_code::ClaudeCodeAdapter),
        );
        adapters.insert("aider".into(), Box::new(aider::AiderAdapter));
        adapters.insert("copilot".into(), Box::new(copilot::CopilotAdapter));
        adapters.insert("pi".into(), Box::new(pi::PiAdapter));
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
            timeout: None,
            retry: None,
            idle_warn: None,
            idle_kill: None,
            model: None,
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
        assert_eq!(
            args,
            &["--dangerously-skip-permissions", "--print", "Hello"]
        );
    }

    #[test]
    fn claude_code_with_files() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let mut task = make_task("Hello");
        task.files = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_eq!(
            args,
            &["--print", "--file", "a.rs", "--file", "b.rs", "Hello"]
        );
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
        assert_eq!(
            args,
            &[
                "--message",
                "Fix it",
                "--file",
                "src/main.rs",
                "--model",
                "gpt-4"
            ]
        );
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

    // -- pi adapter tests --

    #[test]
    fn pi_basic() {
        let adapter = pi::PiAdapter;
        let task = make_task("Hello");
        let cmd = adapter.build_command(&task, None, false, None);
        assert_eq!(cmd.get_program(), "pi");
        let args = get_args(&cmd);
        assert_eq!(args, &["-p", "Hello"]);
    }

    #[test]
    fn pi_auto_approve() {
        let adapter = pi::PiAdapter;
        let task = make_task("Hello");
        // auto_approve=true emits a warning but passes no extra flags
        let cmd = adapter.build_command(&task, None, true, None);
        let args = get_args(&cmd);
        assert_eq!(args, &["-p", "Hello"]);
    }

    #[test]
    fn pi_with_files() {
        let adapter = pi::PiAdapter;
        let mut task = make_task("Hello");
        task.files = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_eq!(args, &["-p", "@a.rs", "@b.rs", "Hello"]);
    }

    #[test]
    fn pi_with_files_and_extra_args() {
        let adapter = pi::PiAdapter;
        let mut task = make_task("Hi");
        task.files = vec![PathBuf::from("src/main.rs")];
        task.extra_args = vec!["--model".into(), "foo".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_eq!(args, &["-p", "@src/main.rs", "--model", "foo", "Hi"]);
    }

    #[test]
    fn pi_sandbox_falls_back() {
        let adapter = pi::PiAdapter;
        let task = make_task("Fix");
        let sandbox = SandboxConfig::default();
        let cmd = adapter.build_command(&task, None, true, Some(&sandbox));
        // pi has no clampdown agent, so it falls back to running directly
        assert_eq!(cmd.get_program(), "pi");
    }

    #[test]
    fn pi_workdir_set_when_not_sandboxed() {
        let adapter = pi::PiAdapter;
        let task = make_task("Hello");
        let workdir = PathBuf::from("/my/project");
        let cmd = adapter.build_command(&task, Some(&workdir), false, None);
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/my/project")));
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
        let args: Vec<String> = get_args(&cmd)
            .iter()
            .map(|a| a.to_string_lossy().into())
            .collect();
        assert_eq!(
            args,
            &[
                "claude",
                "--agent-policy",
                "deny",
                "--agent-allow",
                "api.github.com",
                "--memory",
                "4g",
                "--cpus",
                "4",
                "--workdir",
                "/project",
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
        assert!(names.contains(&"pi"));
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

    // -- model flag --

    fn assert_contains_pair(args: &[&std::ffi::OsStr], key: &str, value: &str) {
        let mut iter = args.iter();
        let mut found = false;
        while let Some(a) = iter.next() {
            if a.to_string_lossy() == key
                && iter.next().map(|v| v.to_string_lossy() == value).unwrap_or(false)
            {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "expected {key} {value:?} in args, got {:?}",
            args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>()
        );
    }

    fn assert_does_not_contain(args: &[&std::ffi::OsStr], key: &str) {
        for a in args {
            assert_ne!(
                a.to_string_lossy(),
                key,
                "unexpected {key} in args: {:?}",
                args
            );
        }
    }

    #[test]
    fn claude_code_model_set() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let mut task = make_task("Hello");
        task.model = Some("sonnet".into());
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_contains_pair(&args, "--model", "sonnet");
    }

    #[test]
    fn claude_code_model_unset() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let task = make_task("Hello");
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_does_not_contain(&args, "--model");
    }

    #[test]
    fn claude_code_model_before_extra_args() {
        let adapter = claude_code::ClaudeCodeAdapter;
        let mut task = make_task("Hello");
        task.model = Some("sonnet".into());
        task.extra_args = vec!["--model".into(), "opus".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        // The config's --model comes first; the extra_args --model comes later and wins.
        let pos_sonnet = args
            .iter()
            .position(|a| a.to_string_lossy() == "--model")
            .expect("first --model");
        assert_eq!(args[pos_sonnet + 1], "sonnet");
        assert_eq!(args[pos_sonnet + 2], "--model");
        assert_eq!(args[pos_sonnet + 3], "opus");
    }

    #[test]
    fn aider_model_set() {
        let adapter = aider::AiderAdapter;
        let mut task = make_task("Fix it");
        task.model = Some("gpt-4".into());
        let cmd = adapter.build_command(&task, None, true, None);
        let args = get_args(&cmd);
        assert_contains_pair(&args, "--model", "gpt-4");
    }

    #[test]
    fn aider_model_unset() {
        let adapter = aider::AiderAdapter;
        let task = make_task("Fix it");
        let cmd = adapter.build_command(&task, None, true, None);
        let args = get_args(&cmd);
        assert_does_not_contain(&args, "--model");
    }

    #[test]
    fn aider_model_before_extra_args() {
        let adapter = aider::AiderAdapter;
        let mut task = make_task("Fix it");
        task.model = Some("gpt-4".into());
        task.extra_args = vec!["--model".into(), "opus".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        let pos = args
            .iter()
            .position(|a| a.to_string_lossy() == "--model")
            .unwrap();
        assert_eq!(args[pos + 1], "gpt-4");
        assert_eq!(args[pos + 2], "--model");
        assert_eq!(args[pos + 3], "opus");
    }

    #[test]
    fn copilot_model_set() {
        let adapter = copilot::CopilotAdapter;
        let mut task = make_task("Suggest");
        task.model = Some("gpt-4o".into());
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_contains_pair(&args, "--model", "gpt-4o");
    }

    #[test]
    fn copilot_model_unset() {
        let adapter = copilot::CopilotAdapter;
        let task = make_task("Suggest");
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_does_not_contain(&args, "--model");
    }

    #[test]
    fn copilot_model_before_extra_args() {
        let adapter = copilot::CopilotAdapter;
        let mut task = make_task("Suggest");
        task.model = Some("gpt-4o".into());
        task.extra_args = vec!["--model".into(), "haiku".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        // For copilot, the prompt sits between the config --model and the
        // extra_args --model, so the extra-args value lands later in argv and
        // wins. Find the LAST --model occurrence and confirm it carries "haiku".
        let last_pos = args
            .iter()
            .rposition(|a| a.to_string_lossy() == "--model")
            .unwrap();
        assert_eq!(args[last_pos + 1], "haiku");
        // The config value still appears earlier in argv.
        let first_pos = args
            .iter()
            .position(|a| a.to_string_lossy() == "--model")
            .unwrap();
        assert_eq!(args[first_pos + 1], "gpt-4o");
    }

    #[test]
    fn pi_model_set() {
        let adapter = pi::PiAdapter;
        let mut task = make_task("Hello");
        task.model = Some("claude".into());
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_contains_pair(&args, "--model", "claude");
    }

    #[test]
    fn pi_model_unset() {
        let adapter = pi::PiAdapter;
        let task = make_task("Hello");
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_does_not_contain(&args, "--model");
    }

    #[test]
    fn pi_model_before_extra_args() {
        let adapter = pi::PiAdapter;
        let mut task = make_task("Hi");
        task.files = vec![PathBuf::from("src/main.rs")];
        task.model = Some("claude".into());
        task.extra_args = vec!["--model".into(), "gpt-4".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        // --model from config comes after the @file tokens but before extra_args
        let pos = args
            .iter()
            .position(|a| a.to_string_lossy() == "--model")
            .unwrap();
        assert_eq!(args[pos + 1], "claude");
        assert_eq!(args[pos + 2], "--model");
        assert_eq!(args[pos + 3], "gpt-4");
    }

    #[test]
    fn generic_passthrough_model_set() {
        let adapter = GenericPassthrough::new("mytool");
        let mut task = make_task("Do stuff");
        task.model = Some("sonnet".into());
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_contains_pair(&args, "--model", "sonnet");
    }

    #[test]
    fn generic_passthrough_model_unset() {
        let adapter = GenericPassthrough::new("mytool");
        let task = make_task("Do stuff");
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        assert_does_not_contain(&args, "--model");
    }

    #[test]
    fn generic_passthrough_model_before_extra_args() {
        let adapter = GenericPassthrough::new("mytool");
        let mut task = make_task("Do stuff");
        task.model = Some("sonnet".into());
        task.extra_args = vec!["--model".into(), "opus".into()];
        let cmd = adapter.build_command(&task, None, false, None);
        let args = get_args(&cmd);
        let pos = args
            .iter()
            .position(|a| a.to_string_lossy() == "--model")
            .unwrap();
        assert_eq!(args[pos + 1], "sonnet");
        assert_eq!(args[pos + 2], "--model");
        assert_eq!(args[pos + 3], "opus");
    }
}
