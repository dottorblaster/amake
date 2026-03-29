use super::sandbox::{apply_workdir, start_sandboxed_command};
use super::Adapter;
use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub struct ClaudeCodeAdapter;

impl Adapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn clampdown_agent(&self) -> Option<&str> {
        Some("claude")
    }

    fn build_command(
        &self,
        task: &Task,
        workdir: Option<&Path>,
        auto_approve: bool,
        sandbox: Option<&SandboxConfig>,
    ) -> Command {
        let mut cmd = Command::new("claude");

        let sandboxed = start_sandboxed_command(
            &mut cmd,
            self.clampdown_agent(),
            self.name(),
            sandbox,
            workdir,
        );

        if auto_approve {
            cmd.arg("--dangerously-skip-permissions");
        }

        cmd.arg("--print");

        for file in &task.files {
            cmd.arg("--file").arg(file);
        }

        cmd.args(&task.extra_args);
        cmd.arg(&task.prompt);

        apply_workdir(&mut cmd, sandboxed, workdir);

        cmd
    }
}
