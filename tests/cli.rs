use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use tempfile::TempDir;

fn amake() -> Command {
    Command::cargo_bin("amake").unwrap()
}

fn copy_dir(src: std::path::PathBuf, dst: &std::path::Path) {
    for entry in fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_dir(entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

// ── Fixture-based test helpers ──

pub struct Scenario {
    _tmp: TempDir,
    pub home: std::path::PathBuf,
    pub project: std::path::PathBuf,
}

/// Root of the fixtures directory.
fn fixtures_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Copy a scenario into a temporary directory and return a `Scenario`.
/// The returned `Scenario` keeps the `TempDir` alive until the test ends.
/// Set `HOME` to `scenario.home` and run amake from `scenario.project`.
fn copy_scenario_temp(scenario_name: &str) -> Scenario {
    let tmp = TempDir::new().unwrap();
    let src = fixtures_root().join(format!("scenario-{scenario_name}"));
    if src.exists() {
        copy_dir(src, tmp.path());
    }
    let home = tmp.path().to_path_buf();
    let project = home.join("project");
    Scenario {
        _tmp: tmp,
        home,
        project,
    }
}

#[allow(dead_code)]
fn no_model_home() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let home = dir.path().to_path_buf();
    let config_dir = home.join(".config/amake");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[profile.amake-medium]\ntool = \"sh\"\n",
    )
    .unwrap();
    (dir, home)
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
    amake().arg("adapters").assert().success().stdout(
        predicate::str::contains("claude-code")
            .and(predicate::str::contains("aider"))
            .and(predicate::str::contains("copilot"))
            .and(predicate::str::contains("pi")),
    );
}

// ── List subcommand ──

#[test]
fn list_no_amakefile_errors() {
    let s = copy_scenario_temp("empty");
    amake()
        .arg("list")
        .current_dir(&s.home)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Amakefile not found"));
}

#[test]
fn list_shows_tasks() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello").and(predicate::str::contains("build")));
}

#[test]
fn list_empty_amakefile() {
    let s = copy_scenario_temp("empty");

    amake()
        .current_dir(&s.project)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No tasks defined."));
}

// ── Run subcommand ──

#[test]
fn run_no_amakefile_errors() {
    let s = copy_scenario_temp("empty");
    amake()
        .args(["run", "hello"])
        .current_dir(&s.home)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Amakefile not found"));
}

#[test]
fn run_unknown_task_errors() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown task"));
}

#[test]
fn run_no_tool_errors() {
    let s = copy_scenario_temp("no-tool");

    amake()
        .current_dir(&s.project)
        .arg("run")
        .arg("hello")
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
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "greet"])
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
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "--var", "name=World", "greet-vars"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello World"));
}

#[test]
fn run_dry_run_missing_var_errors() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "greet-vars"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unresolved variable"));
}

#[test]
fn run_dry_run_dependency_order() {
    let s = copy_scenario_temp("multi");

    let output = amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "second"])
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
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "a"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

#[test]
fn run_executes_echo_tool() {
    let s = copy_scenario_temp("tool-echo");

    amake()
        .current_dir(&s.project)
        .arg("run")
        .arg("greet")
        .assert()
        .success()
        .stdout(predicate::str::contains("hello from amake"));
}

#[test]
fn run_task_failure_exits_nonzero() {
    let s = copy_scenario_temp("false");

    amake()
        .current_dir(&s.project)
        .args(["run", "fail"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed"));
}

#[test]
fn run_keep_going_continues_after_failure() {
    let s = copy_scenario_temp("false");

    amake()
        .current_dir(&s.project)
        .args(["run", "-k", "fail", "ok"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("still running"))
        .stderr(predicate::str::contains("failed"));
}

// ── Config parsing errors ──

#[test]
fn invalid_toml_errors() {
    let s = copy_scenario_temp("invalid");

    amake()
        .current_dir(&s.project)
        .args(["list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse"));
}

#[test]
fn run_bad_var_format_errors() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--var", "no-equals-sign", "t"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("KEY=VALUE"));
}

// ── File discovery ──

#[test]
fn discovers_amake_toml() {
    let s = copy_scenario_temp("multi");
    fs::write(
        s.home.join("amake.toml"),
        r#"
[tasks.hello]
tool = "echo"
prompt = "found it"
"#,
    )
    .unwrap();

    amake()
        .args(["list"])
        .current_dir(&s.project)
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn file_flag_overrides_discovery() {
    let s = copy_scenario_temp("multi");
    // Put an Amakefile in the dir (would be found by discovery)

    // Put a custom config elsewhere
    let custom = s.home.join("custom.toml");
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
        .current_dir(&s.project)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("custom").and(predicate::str::contains("discovered").not()),
        );
}

// ── Sandbox flags ──

#[test]
fn sandbox_flag_without_clampdown_errors() {
    let s = copy_scenario_temp("multi");

    // Only fails if clampdown is not installed, which is the expected CI case
    let result = amake()
        .current_dir(&s.project)
        .args(["run", "--sandbox", "t"])
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
    let s = copy_scenario_temp("tool-echo");

    amake()
        .current_dir(&s.project)
        .env("AMAKE_TEST_NAME", "TestUser")
        .args(["run", "env-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello TestUser"));
}

#[test]
fn run_amakefile_var_interpolation() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "greet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello world"));
}

#[test]
fn run_amakefile_var_command_substitution() {
    let s = copy_scenario_temp("vars-cmd");

    amake()
        .current_dir(&s.project)
        .args(["run", "greet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello alice"));
}

#[test]
fn run_cli_var_overrides_amakefile_var() {
    let s = copy_scenario_temp("vars");

    amake()
        .current_dir(&s.project)
        .args(["run", "--var", "who=bob", "greet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello bob"));
}

// ── Editor variable (--edit-var) ──

/// Helper: create a fake editor script that writes known content to the file it receives.
fn create_fake_editor(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let script = dir.path().join("fake-editor.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ncat > \"$1\" <<'AMAKE_EOF'\n{content}\nAMAKE_EOF\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn edit_var_populates_variable() {
    let dir = TempDir::new().unwrap();
    let s = copy_scenario_temp("multi");
    let editor = create_fake_editor(&dir, "world from editor");

    amake()
        .current_dir(&s.project)
        .env("EDITOR", editor.to_str().unwrap())
        .args(["run", "--edit-var", "name", "greet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello world from editor"));
}

#[test]
fn edit_var_strips_comment_lines() {
    let s = copy_scenario_temp("multi");
    // Editor that preserves the comment header and appends content
    let script = s.home.join("fake-editor.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho '# this is a comment' >> \"$1\"\necho 'actual value' >> \"$1\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    amake()
        .current_dir(&s.project)
        .env("EDITOR", script.to_str().unwrap())
        .args(["run", "--edit-var", "x", "t"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[actual value]"));
}

#[test]
fn edit_var_dry_run() {
    let dir = TempDir::new().unwrap();
    let s = copy_scenario_temp("multi");
    let editor = create_fake_editor(&dir, "edited content");

    amake()
        .current_dir(&s.project)
        .env("EDITOR", editor.to_str().unwrap())
        .args(["run", "--dry-run", "--edit-var", "msg", "t"])
        .assert()
        .success()
        .stdout(predicate::str::contains("got: edited content"));
}

#[test]
fn edit_var_overrides_inline_var() {
    let dir = TempDir::new().unwrap();
    let s = copy_scenario_temp("multi");
    let editor = create_fake_editor(&dir, "from editor");

    // --var first, then --edit-var should override
    amake()
        .current_dir(&s.project)
        .env("EDITOR", editor.to_str().unwrap())
        .args(["run", "--var", "x=from cli", "--edit-var", "x", "t"])
        .assert()
        .success()
        .stdout(predicate::str::contains("from editor"));
}

#[test]
fn edit_var_editor_failure_errors() {
    let _dir = TempDir::new().unwrap();
    let s = copy_scenario_temp("multi");
    let script = s.home.join("bad-editor.sh");
    fs::write(&script, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    amake()
        .current_dir(&s.project)
        .env("EDITOR", script.to_str().unwrap())
        .args(["run", "--edit-var", "x", "t"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("editor"));
}

#[test]
fn edit_var_multiline_content() {
    let dir = TempDir::new().unwrap();
    let s = copy_scenario_temp("multi");
    let editor = create_fake_editor(&dir, "line one\nline two\nline three");

    amake()
        .current_dir(&s.project)
        .env("EDITOR", editor.to_str().unwrap())
        .args(["run", "--edit-var", "body", "t"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("line one")
                .and(predicate::str::contains("line two"))
                .and(predicate::str::contains("line three")),
        );
}

// ── Timeout / retry ──

#[test]
fn task_timeout_kills_runaway_child() {
    let s = copy_scenario_temp("multi");

    let start = std::time::Instant::now();
    amake()
        .current_dir(&s.project)
        .args(["run", "slow"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("timed out"));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "timeout should fire well before the 10s sleep completes"
    );
}

#[test]
fn task_retry_eventually_succeeds() {
    let _dir = TempDir::new().unwrap();
    let s = copy_scenario_temp("multi");
    let marker = s.home.join("marker");
    let _script = format!(
        "test -f {marker} || (touch {marker}; exit 1)",
        marker = marker.display()
    );

    amake()
        .current_dir(&s.project)
        .args(["run", "flaky"])
        .assert()
        .success()
        .stderr(predicate::str::contains("retrying"));
}

#[test]
fn retry_exhausted_reports_attempt_count() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "always-fails"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("after 2 attempts").and(predicate::str::contains("retrying")),
        );
}

#[test]
fn timeout_with_on_timeout_false_does_not_retry() {
    let s = copy_scenario_temp("multi");

    let start = std::time::Instant::now();
    amake()
        .current_dir(&s.project)
        .args(["run", "slow"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("timed out"));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "should not retry on timeout (would take 1s + backoff each time)"
    );
}

#[test]
fn dry_run_annotates_timeout_and_retry() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "t"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timeout 60s").and(predicate::str::contains("retry 3x")));
}

#[test]
fn rejects_zero_attempts() {
    let s = copy_scenario_temp("retry-zero");

    amake()
        .current_dir(&s.project)
        .args(["run", "t"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid retry config"));
}

// ── Stall detection / idle thresholds / closed stdin ──

#[test]
fn idle_kill_terminates_stalled_task() {
    let s = copy_scenario_temp("multi");

    let start = std::time::Instant::now();
    amake()
        .current_dir(&s.project)
        .args(["run", "stalled"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("killed after").and(predicate::str::contains("silence")));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(8),
        "idle_kill should fire well before the 60s sleep completes"
    );
}

#[test]
fn idle_warn_only_emits_warning_but_succeeds() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "slowish"])
        .assert()
        .success()
        .stderr(predicate::str::contains("idle for"));
}

#[test]
fn closed_stdin_makes_cat_exit_immediately() {
    // `cat` with no args reads from stdin. With Stdio::null, the read sees EOF
    // and the process exits cleanly. Without it, this test would hang forever.
    // `sh -c cat ""` invokes cat with no args ($0 set to "" but argv empty).
    let s = copy_scenario_temp("sh");

    let start = std::time::Instant::now();
    amake()
        .current_dir(&s.project)
        .args(["run", "cat"])
        .assert()
        .success();
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "closed stdin should make cat exit immediately on EOF"
    );
}

#[test]
fn no_spinner_when_stderr_is_piped() {
    let s = copy_scenario_temp("multi");

    // assert_cmd captures stderr non-TTY; spinner braille glyphs must not appear.
    amake()
        .current_dir(&s.project)
        .args(["run", "t"])
        .assert()
        .success()
        .stderr(predicate::str::contains("⠋").not());
}

#[test]
fn idle_kill_triggers_retry_when_on_timeout_true() {
    let s = copy_scenario_temp("multi");

    let start = std::time::Instant::now();
    amake()
        .current_dir(&s.project)
        .args(["run", "stalled"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("retrying").and(predicate::str::contains("went idle")));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "two attempts of ~1s idle-kill plus 1s backoff should fit well under 10s"
    );
}

// ── Model flag ──

#[test]
fn dry_run_emits_model_flag() {
    let s = copy_scenario_temp("claude");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "greet"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[greet]")
                .and(predicate::str::contains("claude"))
                .and(predicate::str::contains("--model"))
                .and(predicate::str::contains("sonnet")),
        );
}

#[test]
fn dry_run_inherits_default_model() {
    let s = copy_scenario_temp("claude");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "greet-opus"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--model").and(predicate::str::contains("opus")));
}

#[test]
fn dry_run_task_model_overrides_default() {
    let s = copy_scenario_temp("claude");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "greet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sonnet").and(predicate::str::contains("opus").not()));
}

#[test]
fn extra_args_model_overrides_config_model() {
    let s = copy_scenario_temp("claude");

    let output = amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "extra-args"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    // The extra_args --model should appear later in the rendered command
    // and therefore "win" against the config's --model.
    let sonnet_pos = stdout
        .find("sonnet")
        .expect("expected 'sonnet' to appear in dry-run output");
    let opus_pos = stdout
        .rfind("opus")
        .expect("expected 'opus' to appear in dry-run output");
    let last_model_pos = stdout
        .rfind("--model")
        .expect("expected '--model' to appear");
    // The last --model must be the extra_args one carrying "opus".
    let after_last = &stdout[last_model_pos..];
    assert!(
        after_last.contains("opus"),
        "expected last --model to carry 'opus', got tail: {after_last:?}"
    );
    // 'sonnet' must appear before 'opus' in the rendered command.
    assert!(
        sonnet_pos < opus_pos,
        "expected 'sonnet' to appear before 'opus', got:\n{stdout}"
    );
}

#[test]
fn dry_run_no_model_when_unset() {
    let s = copy_scenario_temp("multi");

    amake()
        .current_dir(&s.project)
        .args(["run", "--dry-run", "greet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--model").not());
}
