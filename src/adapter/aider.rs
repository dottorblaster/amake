use super::Adapter;
use super::sandbox::{apply_workdir, start_sandboxed_command};
use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub struct AiderAdapter;

impl Adapter for AiderAdapter {
    fn name(&self) -> &str {
        "aider"
    }

    fn clampdown_agent(&self) -> Option<&str> {
        None
    }

    fn build_command(
        &self,
        task: &Task,
        workdir: Option<&Path>,
        auto_approve: bool,
        sandbox: Option<&SandboxConfig>,
    ) -> Command {
        let mut cmd = Command::new("aider");

        let sandboxed = start_sandboxed_command(
            &mut cmd,
            self.clampdown_agent(),
            self.name(),
            sandbox,
            workdir,
        );

        if auto_approve {
            cmd.arg("--yes");
        }

        cmd.arg("--message").arg(&task.prompt);

        for file in &task.files {
            cmd.arg("--file").arg(file);
        }

        cmd.args(&task.extra_args);

        apply_workdir(&mut cmd, sandboxed, workdir);

        cmd
    }
}
