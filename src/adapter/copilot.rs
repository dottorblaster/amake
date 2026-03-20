use super::sandbox::start_sandboxed_command;
use super::Adapter;
use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub struct CopilotAdapter;

impl Adapter for CopilotAdapter {
    fn name(&self) -> &str {
        "copilot"
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
        let mut cmd = Command::new("gh");

        let sandboxed = start_sandboxed_command(
            &mut cmd,
            self.clampdown_agent(),
            self.name(),
            sandbox,
            workdir,
        );

        if auto_approve {
            eprintln!("warning: auto_approve is set for copilot but no known flag exists — ignoring");
        }

        cmd.args(["copilot", "suggest", "-t", "shell"]);
        cmd.arg(&task.prompt);
        cmd.args(&task.extra_args);

        if !sandboxed {
            if let Some(dir) = workdir {
                cmd.current_dir(dir);
            }
        }

        cmd
    }
}
