use crate::sandbox::SandboxConfig;
use std::path::Path;
use std::process::Command;

pub fn start_sandboxed_command(
    cmd: &mut Command,
    agent_name: Option<&str>,
    adapter_name: &str,
    sandbox: Option<&SandboxConfig>,
    workdir: Option<&Path>,
) -> bool {
    let Some(sandbox) = sandbox else {
        return false;
    };

    let Some(agent) = agent_name else {
        eprintln!(
            "warning: sandbox requested for tool {:?} but clampdown does not support it — running without sandbox",
            adapter_name
        );
        return false;
    };

    *cmd = Command::new("clampdown");
    cmd.arg(agent);

    for arg in sandbox.to_args() {
        cmd.arg(arg);
    }

    if let Some(dir) = workdir {
        cmd.arg("--workdir").arg(dir);
    }

    cmd.arg("--");
    true
}

/// Sets `current_dir` on `cmd` when not running inside a sandbox.
/// When sandboxed, `clampdown --workdir` handles the directory instead.
pub fn apply_workdir(cmd: &mut Command, sandboxed: bool, workdir: Option<&Path>) {
    if !sandboxed
        && let Some(dir) = workdir
    {
        cmd.current_dir(dir);
    }
}
