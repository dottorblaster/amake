use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use tempfile::TempDir;

fn amake() -> Command {
    Command::cargo_bin("amake").unwrap()
}

fn setup_amakefile(dir: &TempDir, content: &str) {
    fs::write(dir.path().join("Amakefile"), content).unwrap();
}

// ── No subcommand ──

#[test]
fn no_subcommand_shows_usage() {
    amake()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

// ── Help ──

#[test]
fn help_flag() {
    amake()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("A task runner for AI CLI tools"));
}

// ── Adapters subcommand ──

#[test]
fn adapters_lists_builtins() {
    amake()
        .arg("adapters")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("claude-code")
                .and(predicate::str::contains("aider"))
                .and(predicate::str::contains("copilot")),
        );
}

// ── List subcommand ──

#[test]
fn list_no_amakefile_errors() {
    let dir = TempDir::new().unwrap();
    amake()
        .arg("list")
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Amakefile not found"));
}

#[test]
fn list_shows_tasks() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.hello]
prompt = "Say hello"

[tasks.build]
prompt = "Build the project"
"#,
    );

    amake()
        .args(["list", "-f", dir.path().join("Amakefile").to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("hello")
                .and(predicate::str::contains("build")),
        );
}

#[test]
fn list_empty_amakefile() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(&dir, "");

    amake()
        .args(["list", "-f", dir.path().join("Amakefile").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No tasks defined."));
}

// ── Run subcommand ──

#[test]
fn run_no_amakefile_errors() {
    let dir = TempDir::new().unwrap();
    amake()
        .args(["run", "hello"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Amakefile not found"));
}

#[test]
fn run_unknown_task_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.hello]
prompt = "Hi"
tool = "echo"
"#,
    );

    amake()
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "nonexistent",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown task"));
}

#[test]
fn run_no_tool_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.hello]
prompt = "Hi"
"#,
    );

    amake()
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "hello",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no tool specified"));
}

#[test]
fn run_missing_task_arg_errors() {
    amake().args(["run"]).assert().failure();
}

#[test]
fn run_dry_run_prints_command() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.greet]
prompt = "Hello world"
"#,
    );

    amake()
        .args([
            "run",
            "--dry-run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "greet",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[greet]")
                .and(predicate::str::contains("echo"))
                .and(predicate::str::contains("Hello world")),
        );
}

#[test]
fn run_dry_run_with_vars() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.greet]
prompt = "Hello {{vars.name}}"
"#,
    );

    amake()
        .args([
            "run",
            "--dry-run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--var",
            "name=World",
            "greet",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello World"));
}

#[test]
fn run_dry_run_missing_var_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.greet]
prompt = "Hello {{vars.name}}"
"#,
    );

    amake()
        .args([
            "run",
            "--dry-run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "greet",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unresolved variable"));
}

#[test]
fn run_dry_run_dependency_order() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.first]
prompt = "Step 1"

[tasks.second]
prompt = "Step 2"
depends = ["first"]
"#,
    );

    let output = amake()
        .args([
            "run",
            "--dry-run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "second",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let first_pos = stdout.find("[first]").expect("first task present");
    let second_pos = stdout.find("[second]").expect("second task present");
    assert!(
        first_pos < second_pos,
        "first should appear before second in dry-run output"
    );
}

#[test]
fn run_dry_run_cycle_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.a]
prompt = "A"
depends = ["b"]

[tasks.b]
prompt = "B"
depends = ["a"]
"#,
    );

    amake()
        .args([
            "run",
            "--dry-run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "a",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

#[test]
fn run_executes_echo_tool() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.greet]
tool = "echo"
prompt = "hello from amake"
"#,
    );

    amake()
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "greet",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello from amake"));
}

#[test]
fn run_task_failure_exits_nonzero() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.fail]
tool = "false"
prompt = ""
"#,
    );

    amake()
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "fail",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed"));
}

#[test]
fn run_keep_going_continues_after_failure() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.fail]
tool = "false"
prompt = ""

[tasks.ok]
tool = "echo"
prompt = "still running"
"#,
    );

    amake()
        .args([
            "run",
            "-k",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "fail",
            "ok",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("still running"))
        .stderr(predicate::str::contains("failed"));
}

// ── Config parsing errors ──

#[test]
fn invalid_toml_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(&dir, "this is not valid toml [[[");

    amake()
        .args(["list", "-f", dir.path().join("Amakefile").to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse"));
}

#[test]
fn run_bad_var_format_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.t]
tool = "echo"
prompt = "hi"
"#,
    );

    amake()
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--var",
            "no-equals-sign",
            "t",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("KEY=VALUE"));
}

// ── File discovery ──

#[test]
fn discovers_amake_toml() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("amake.toml"),
        r#"
[tasks.hello]
tool = "echo"
prompt = "found it"
"#,
    )
    .unwrap();

    amake()
        .args(["list"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn file_flag_overrides_discovery() {
    let dir = TempDir::new().unwrap();
    // Put an Amakefile in the dir (would be found by discovery)
    setup_amakefile(
        &dir,
        r#"
[tasks.discovered]
tool = "echo"
prompt = "wrong"
"#,
    );

    // Put a custom config elsewhere
    let custom = dir.path().join("custom.toml");
    fs::write(
        &custom,
        r#"
[tasks.custom]
tool = "echo"
prompt = "right"
"#,
    )
    .unwrap();

    amake()
        .args(["list", "-f", custom.to_str().unwrap()])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("custom")
                .and(predicate::str::contains("discovered").not()),
        );
}

// ── Sandbox flags ──

#[test]
fn sandbox_flag_without_clampdown_errors() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.t]
tool = "echo"
prompt = "hi"
"#,
    );

    // Only fails if clampdown is not installed, which is the expected CI case
    let result = amake()
        .args([
            "run",
            "--sandbox",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "t",
        ])
        .assert();

    // If clampdown happens to be installed, the task succeeds; otherwise it errors
    let output = result.get_output();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("clampdown not found"),
            "expected clampdown-not-found error, got: {stderr}"
        );
    }
}

#[test]
fn run_env_variable_interpolation() {
    let dir = TempDir::new().unwrap();
    setup_amakefile(
        &dir,
        r#"
[tasks.greet]
tool = "echo"
prompt = "Hello {{env.AMAKE_TEST_NAME}}"
"#,
    );

    amake()
        .env("AMAKE_TEST_NAME", "TestUser")
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "greet",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello TestUser"));
}

// ── Editor variable (--edit-var) ──

/// Helper: create a fake editor script that writes known content to the file it receives.
fn create_fake_editor(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let script = dir.path().join("fake-editor.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat > \"$1\" <<'AMAKE_EOF'\n{content}\nAMAKE_EOF\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn edit_var_populates_variable() {
    let dir = TempDir::new().unwrap();
    let editor = create_fake_editor(&dir, "world from editor");
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.greet]
prompt = "Hello {{vars.name}}"
"#,
    );

    amake()
        .env("EDITOR", editor.to_str().unwrap())
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--edit-var",
            "name",
            "greet",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello world from editor"));
}

#[test]
fn edit_var_strips_comment_lines() {
    let dir = TempDir::new().unwrap();
    // Editor that preserves the comment header and appends content
    let script = dir.path().join("fake-editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho '# this is a comment' >> \"$1\"\necho 'actual value' >> \"$1\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.t]
prompt = "[{{vars.x}}]"
"#,
    );

    amake()
        .env("EDITOR", script.to_str().unwrap())
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--edit-var",
            "x",
            "t",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[actual value]"));
}

#[test]
fn edit_var_dry_run() {
    let dir = TempDir::new().unwrap();
    let editor = create_fake_editor(&dir, "edited content");
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.t]
prompt = "got: {{vars.msg}}"
"#,
    );

    amake()
        .env("EDITOR", editor.to_str().unwrap())
        .args([
            "run",
            "--dry-run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--edit-var",
            "msg",
            "t",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("got: edited content"));
}

#[test]
fn edit_var_overrides_inline_var() {
    let dir = TempDir::new().unwrap();
    let editor = create_fake_editor(&dir, "from editor");
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.t]
prompt = "{{vars.x}}"
"#,
    );

    // --var first, then --edit-var should override
    amake()
        .env("EDITOR", editor.to_str().unwrap())
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--var",
            "x=from cli",
            "--edit-var",
            "x",
            "t",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("from editor"));
}

#[test]
fn edit_var_editor_failure_errors() {
    let dir = TempDir::new().unwrap();
    let script = dir.path().join("bad-editor.sh");
    fs::write(&script, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    setup_amakefile(
        &dir,
        r#"
[tasks.t]
tool = "echo"
prompt = "{{vars.x}}"
"#,
    );

    amake()
        .env("EDITOR", script.to_str().unwrap())
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--edit-var",
            "x",
            "t",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("editor"));
}

#[test]
fn edit_var_multiline_content() {
    let dir = TempDir::new().unwrap();
    let editor = create_fake_editor(&dir, "line one\nline two\nline three");
    setup_amakefile(
        &dir,
        r#"
[defaults]
tool = "echo"

[tasks.t]
prompt = "{{vars.body}}"
"#,
    );

    amake()
        .env("EDITOR", editor.to_str().unwrap())
        .args([
            "run",
            "-f",
            dir.path().join("Amakefile").to_str().unwrap(),
            "--edit-var",
            "body",
            "t",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("line one")
                .and(predicate::str::contains("line two"))
                .and(predicate::str::contains("line three")),
        );
}
