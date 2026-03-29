# amake

A task runner for AI CLI tools. Think `make`, but for dispatching prompts to things like Claude Code, Aider, GitHub Copilot, etc.

You define tasks in a TOML file, each with a prompt and a tool. `amake run <task>` figures out the right CLI invocation and runs it. Tasks can depend on each other, pass captured output downstream, and optionally run inside a [clampdown](https://github.com/89luca89/clampdown) sandbox.

## Install

```
cargo install --path .
```

You'll need Rust 2024 edition (1.85+).

## Quick start

Create an `Amakefile` in your project root:

```toml
[defaults]
tool = "claude-code"

[tasks.create-pr]
prompt = """
Create a PR for the current branch.
Write a clear description based on the diff.
"""
auto_approve = true

[tasks.review]
depends = ["create-pr"]
prompt = "Review the changes in the current branch. Be concise."
```

Then:

```
amake run review
```

This runs `create-pr` first (because `review` depends on it), then `review`. Both use `claude-code` since that's the default.

## Amakefile format

The config file can be called `Amakefile` or `amake.toml`. amake searches upward from the current directory to find it, same as make does.

### Defaults

```toml
[defaults]
tool = "claude-code"    # used when a task doesn't specify its own
workdir = "./src"       # optional, sets cwd for all tasks
```

### Tasks

```toml
[tasks.refactor]
tool = "aider"                          # override the default tool
prompt = "Refactor error handling to use thiserror."
files = ["src/main.rs", "src/error.rs"] # context files passed to the tool
auto_approve = true                     # tool-specific "don't ask" flag
extra_args = ["--model", "gpt-4"]       # passed verbatim to the tool CLI
```

A task can depend on other tasks and use their captured output:

```toml
[tasks.describe]
prompt = "Describe what this repo does in one paragraph."
capture = true

[tasks.review]
depends = ["describe"]
prompt = """
Here's a summary of the repo:
{{tasks.describe.stdout}}

Now review the open PR for correctness.
"""
```

### Template variables

Prompts support `{{...}}` placeholders from three sources:

| Syntax | Source |
|---|---|
| `{{tasks.foo.stdout}}` | Captured output from task `foo` (must be in `depends`, must have `capture = true`) |
| `{{vars.name}}` | Passed via `--var name=value` or `--edit-var name` on the CLI |
| `{{env.HOME}}` | Read from the environment |

Missing variables are a hard error — no silent empty strings.

### Editor-based variables

For longer or multiline content, use `--edit-var` instead of `--var`. This opens your `$VISUAL` or `$EDITOR` (falling back to `vi`) with a temporary file. Write the value, save, and close — the content becomes the variable's value.

```
amake run review-pr --edit-var focus
```

The temporary file starts with comment lines (prefixed `#`) that are stripped from the final value. If you specify multiple `--edit-var` flags, each one opens the editor sequentially, one at a time.

If both `--var name=value` and `--edit-var name` are given, the editor value wins.

## Built-in adapters

```
$ amake adapters
aider
claude-code
copilot
```

| Adapter | Binary | Auto-approve | Notes |
|---|---|---|---|
| `claude-code` | `claude` | `--dangerously-skip-permissions` | Prompt via `--print` |
| `aider` | `aider` | `--yes` | Prompt via `--message` |
| `copilot` | `gh` | (none) | Runs `gh copilot suggest -t shell` |

If the tool name doesn't match a built-in, amake treats it as a bare binary and passes `extra_args` + prompt as a positional arg. Good enough for most things:

```toml
[tasks.lint]
tool = "openclaw"
prompt = "Fix all clippy warnings."
extra_args = ["--auto-fix"]
```

## Sandbox support

Tasks can run inside [clampdown](https://github.com/89luca89/clampdown) for filesystem/network isolation:

```toml
[tasks.untrusted-refactor]
prompt = "Refactor the auth module."
auto_approve = true

[tasks.untrusted-refactor.sandbox]
agent_policy = "deny"
agent_allow = ["api.github.com"]
memory = "4g"
cpus = "4"
tripwire = true
protect = [".env.production"]
```

This wraps the invocation through clampdown instead of calling the tool directly:

```
clampdown claude --agent-policy deny --agent-allow api.github.com \
  --memory 4g --cpus 4 --tripwire --protect .env.production \
  --workdir /path/to/project -- --dangerously-skip-permissions --print "Refactor the auth module."
```

You can set sandbox defaults for all tasks:

```toml
[defaults.sandbox]
agent_policy = "deny"
memory = "8g"
gitconfig = true
```

A task can opt out with `sandbox = false`.

CLI flags `--sandbox` and `--no-sandbox` override everything — useful for testing.

Currently only `claude-code` is supported by clampdown. Other tools fall back to running without the sandbox (with a warning).

### Sandbox options

`agent_policy`, `agent_allow`, `pod_policy`, `memory`, `cpus`, `protect`, `mask`, `unmask`, `gitconfig`, `gh`, `ssh`, `tripwire`, `extra_args` — these map directly to clampdown flags. See [clampdown's docs](https://github.com/89luca89/clampdown) for details.

## CLI reference

```
amake run <TASKS>... [OPTIONS]

  --dry-run              Print commands without running them
  -k, --keep-going       Don't stop on first failure
  --var <KEY=VALUE>      Set a template variable (repeatable)
  --edit-var <NAME>      Open $EDITOR to input a variable value (repeatable)
  -f, --file <PATH>      Explicit Amakefile path
  --sandbox              Force sandbox for all tasks
  --no-sandbox           Disable sandbox for all tasks

amake list               Show all tasks
amake adapters           Show built-in adapter names
```

`--dry-run` still resolves templates, so it's useful for checking that your variables and dependencies are wired up right.

## A bigger example

```toml
[defaults]
tool = "claude-code"

[defaults.sandbox]
agent_policy = "deny"
agent_allow = ["api.github.com"]
memory = "8g"
gitconfig = true
gh = true

[tasks.create-pr]
prompt = """
Create a PR for the current branch.
Write a clear description based on the diff.
"""
capture = true
auto_approve = true

[tasks.review-pr]
tool = "copilot"
depends = ["create-pr"]
prompt = """
Review the PR:
{{tasks.create-pr.stdout}}

Focus on: {{vars.focus}}
"""

[tasks.refactor]
tool = "aider"
prompt = "Refactor error handling to use thiserror."
files = ["src/main.rs", "src/error.rs"]
auto_approve = true
extra_args = ["--model", "gpt-4"]
```

```
amake run review-pr --var focus="security and error handling"
```

This runs `create-pr` first, captures its output, then passes it into `review-pr`'s prompt.
