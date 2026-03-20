use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Amakefile not found — create an Amakefile or amake.toml in your project, or specify one with -f")]
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

    #[error("no tool specified for task {0:?} and no default tool set — add `tool` to the task or set [defaults] tool")]
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

    #[error("task {task:?} failed with exit code {code}")]
    TaskFailed { task: String, code: i32 },

    #[error("task {task:?} was terminated by a signal")]
    TaskSignaled { task: String },

    #[error("clampdown not found — install it from https://github.com/89luca89/clampdown or disable sandbox with --no-sandbox")]
    ClampdownNotFound,

    #[error("{0}")]
    Io(#[from] std::io::Error),
}
