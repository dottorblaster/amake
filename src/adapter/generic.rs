use super::sandbox::start_sandboxed_command;
use super::Adapter;
use crate::config::Task;
use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub struct GenericPassthrough {
    binary: String,
}

impl GenericPassthrough {
    pub fn new(name: &str) -> Self {
        Self {
            binary: name.to_string(),
        }
    }
}

impl Adapter for GenericPassthrough {
    fn name(&self) -> &str {
        &self.binary
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
        let mut cmd = Command::new(&self.binary);

        let sandboxed = start_sandboxed_command(
            &mut cmd,
            self.clampdown_agent(),
            self.name(),
            sandbox,
            workdir,
        );

        if auto_approve {
            eprintln!(
                "warning: auto_approve is set for unknown tool {:?} — no auto-approve flag known",
                self.binary
            );
        }

        cmd.args(&task.extra_args);
        cmd.arg(&task.prompt);

        if !sandboxed {
            if let Some(dir) = workdir {
                cmd.current_dir(dir);
            }
        }

        cmd
    }
}
