use super::Adapter;
use super::sandbox::{apply_workdir, start_sandboxed_command};
use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub struct PiAdapter;

impl Adapter for PiAdapter {
    fn name(&self) -> &str {
        "pi"
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
        let mut cmd = Command::new("pi");

        let sandboxed = start_sandboxed_command(
            &mut cmd,
            self.clampdown_agent(),
            self.name(),
            sandbox,
            workdir,
        );

        if auto_approve {
            eprintln!("warning: auto_approve is set for pi but no known flag exists — ignoring");
        }

        cmd.arg("-p");

        // pi uses inline @<path> tokens in the prompt for context files.
        // Each is passed as a separate argv entry so paths containing
        // spaces are preserved by the kernel's argv boundary.
        for file in &task.files {
            cmd.arg(format!("@{}", file.display()));
        }

        if let Some(model) = &task.model {
            cmd.arg("--model").arg(model);
        }

        cmd.args(&task.extra_args);
        cmd.arg(&task.prompt);

        apply_workdir(&mut cmd, sandboxed, workdir);

        cmd
    }
}
