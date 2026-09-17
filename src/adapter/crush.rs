use super::Adapter;
use super::sandbox::{apply_workdir, start_sandboxed_command};
use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub struct CrushAdapter;

impl Adapter for CrushAdapter {
    fn name(&self) -> &str {
        "crush"
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
        let mut cmd = Command::new("crush");

        let sandboxed = start_sandboxed_command(
            &mut cmd,
            self.clampdown_agent(),
            self.name(),
            sandbox,
            workdir,
        );

        if auto_approve {
            eprintln!("warning: auto_approve is set for crush but no known flag exists — ignoring");
        }

        if !task.files.is_empty() {
            eprintln!(
                "warning: crush adapter does not support context files — ignoring {} file(s)",
                task.files.len()
            );
        }

        if let Some(model) = &task.model {
            cmd.arg("--model").arg(model);
        }

        cmd.args(&task.extra_args);
        cmd.arg("run");
        cmd.arg(&task.prompt);

        apply_workdir(&mut cmd, sandboxed, workdir);

        cmd
    }
}
