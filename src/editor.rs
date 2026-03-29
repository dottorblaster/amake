use crate::error::Error;
use std::process::{Command, Stdio};

/// Opens the user's preferred text editor to collect a variable value.
///
/// The editor is chosen from `$VISUAL`, then `$EDITOR`, falling back to `vi`.
/// A temporary file is created with a comment header explaining what to do.
/// After the editor exits, the file content (minus comment lines) is returned.
pub fn edit_variable(var_name: &str) -> Result<String, Error> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());

    let dir = std::env::temp_dir();
    let path = dir.join(format!("amake-{}-{var_name}.txt", std::process::id()));

    let header = format!(
        "# Enter the value for variable: {var_name}\n\
         # Lines starting with '#' will be stripped.\n\
         # Save and close the editor when done.\n"
    );
    std::fs::write(&path, &header)?;

    let status = Command::new(&editor)
        .arg(&path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::EditorFailed {
            reason: format!("failed to launch editor {editor:?}: {e}"),
        })?;

    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err(Error::EditorFailed {
            reason: format!(
                "editor exited with {}",
                status
                    .code()
                    .map(|c| format!("code {c}"))
                    .unwrap_or_else(|| "a signal".into())
            ),
        });
    }

    let contents = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);

    let value: String = contents
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    Ok(value)
}
