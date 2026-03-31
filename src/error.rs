use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Amakefile not found — create an Amakefile or amake.toml in your project, or specify one with -f"
    )]
    ConfigNotFound,

    #[error("failed to read {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("unknown task {0:?} — run `amake list` to see available tasks")]
    UnknownTask(String),

    #[error(
        "no tool specified for task {0:?} and no default tool set — add `tool` to the task or set [defaults] tool"
    )]
    NoTool(String),

    #[error("dependency cycle detected: {0}")]
    DependencyCycle(String),

    #[error("task {task:?} references {{{{tasks.{dependency}.stdout}}}} but {reason}")]
    InvalidTaskReference {
        task: String,
        dependency: String,
        reason: String,
    },

    #[error("unresolved variable {{{{{variable}}}}} in task {task:?} — {hint}")]
    UnresolvedVariable {
        task: String,
        variable: String,
        hint: String,
    },

    #[error("{}", format_task_failed(.task, .code, .command, .stderr_tail))]
    TaskFailed {
        task: String,
        code: i32,
        command: Option<String>,
        stderr_tail: Option<String>,
    },

    #[error("{}", format_task_signaled(.task, .command, .stderr_tail))]
    TaskSignaled {
        task: String,
        command: Option<String>,
        stderr_tail: Option<String>,
    },

    #[error("editor failed: {reason}")]
    EditorFailed { reason: String },

    #[error(
        "clampdown not found — install it from https://github.com/89luca89/clampdown or disable sandbox with --no-sandbox"
    )]
    ClampdownNotFound,

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

fn format_task_failed(
    task: &str,
    code: &i32,
    command: &Option<String>,
    stderr_tail: &Option<String>,
) -> String {
    let mut msg = format!("task {task:?} failed with exit code {code}");
    if let Some(cmd) = command {
        msg.push_str(&format!("\n  command: {cmd}"));
    }
    if let Some(stderr) = stderr_tail
        && !stderr.is_empty()
    {
        msg.push_str(&format!(
            "\n  stderr:\n    {}",
            stderr.replace('\n', "\n    ")
        ));
    }
    msg
}

fn format_task_signaled(
    task: &str,
    command: &Option<String>,
    stderr_tail: &Option<String>,
) -> String {
    let mut msg = format!("task {task:?} was terminated by a signal");
    if let Some(cmd) = command {
        msg.push_str(&format!("\n  command: {cmd}"));
    }
    if let Some(stderr) = stderr_tail
        && !stderr.is_empty()
    {
        msg.push_str(&format!(
            "\n  stderr:\n    {}",
            stderr.replace('\n', "\n    ")
        ));
    }
    msg
}
