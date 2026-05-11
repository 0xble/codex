use clap::Args;
use clap::FromArgMatches;
use clap::Parser;
use clap::ValueEnum;
use codex_utils_cli::CliConfigOverrides;
use codex_utils_cli::SharedCliOptions;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    version,
    override_usage = "codex exec [OPTIONS] [PROMPT]\n       codex exec [OPTIONS] <COMMAND> [ARGS]"
)]
pub struct Cli {
    /// Action to perform. If omitted, runs a new non-interactive session.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Error out when config.toml contains fields that are not recognized by this version of Codex.
    #[arg(long = "strict-config", global = true, default_value_t = false)]
    pub strict_config: bool,

    #[clap(flatten)]
    pub shared: ExecSharedCliOptions,

    /// Allow running Codex outside a Git repository.
    #[arg(long = "skip-git-repo-check", global = true, default_value_t = false)]
    pub skip_git_repo_check: bool,

    /// Run without persisting session files to disk.
    #[arg(long = "ephemeral", global = true, default_value_t = false)]
    pub ephemeral: bool,

    /// Do not load `$CODEX_HOME/config.toml`; auth still uses `CODEX_HOME`.
    #[arg(long = "ignore-user-config", global = true, default_value_t = false)]
    pub ignore_user_config: bool,

    /// Do not load user or project execpolicy `.rules` files.
    #[arg(long = "ignore-rules", global = true, default_value_t = false)]
    pub ignore_rules: bool,

    /// Legacy compatibility trap for the removed `--full-auto` flag.
    #[arg(
        long = "full-auto",
        hide = true,
        global = true,
        default_value_t = false,
        conflicts_with = "dangerously_bypass_approvals_and_sandbox"
    )]
    pub removed_full_auto: bool,

    /// Path to a JSON Schema file describing the model's final response shape.
    #[arg(long = "output-schema", value_name = "FILE", global = true)]
    pub output_schema: Option<PathBuf>,

    #[clap(skip)]
    pub config_overrides: CliConfigOverrides,

    /// Specifies color settings for use in the output.
    #[arg(long = "color", value_enum, default_value_t = Color::Auto)]
    pub color: Color,

    /// Print events to stdout as JSONL.
    #[arg(
        long = "json",
        alias = "experimental-json",
        default_value_t = false,
        global = true
    )]
    pub json: bool,

    /// Specifies file where the last message from the agent should be written.
    #[arg(
        long = "output-last-message",
        short = 'o',
        value_name = "FILE",
        global = true
    )]
    pub last_message_file: Option<PathBuf>,

    /// Initial instructions for the agent. If not provided as an argument (or
    /// if `-` is used), instructions are read from stdin. If stdin is piped and
    /// a prompt is also provided, stdin is appended as a `<stdin>` block.
    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    pub prompt: Option<String>,
}

impl std::ops::Deref for Cli {
    type Target = SharedCliOptions;

    fn deref(&self) -> &Self::Target {
        &self.shared.0
    }
}

impl std::ops::DerefMut for Cli {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.shared.0
    }
}

impl Cli {
    pub fn removed_full_auto_warning(&self) -> Option<&'static str> {
        if self.removed_full_auto {
            return Some(
                "warning: `--full-auto` is deprecated; use `--sandbox workspace-write` instead.",
            );
        }

        None
    }
}

#[derive(Debug, Default)]
pub struct ExecSharedCliOptions(SharedCliOptions);

impl ExecSharedCliOptions {
    pub fn into_inner(self) -> SharedCliOptions {
        self.0
    }
}

impl std::ops::Deref for ExecSharedCliOptions {
    type Target = SharedCliOptions;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ExecSharedCliOptions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Args for ExecSharedCliOptions {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        mark_exec_global_args(SharedCliOptions::augment_args(cmd))
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        mark_exec_global_args(SharedCliOptions::augment_args_for_update(cmd))
    }
}

impl FromArgMatches for ExecSharedCliOptions {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        SharedCliOptions::from_arg_matches(matches).map(Self)
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        self.0.update_from_arg_matches(matches)
    }
}

fn mark_exec_global_args(cmd: clap::Command) -> clap::Command {
    cmd.mut_arg("model", |arg| arg.global(true))
        .mut_arg("dangerously_bypass_approvals_and_sandbox", |arg| {
            arg.global(true)
        })
        .mut_arg("bypass_hook_trust", |arg| arg.global(true))
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Resume a previous session by id or pick the most recent with --last.
    Resume(ResumeArgs),

    /// Run a code review against the current repository.
    Review(Box<ReviewArgs>),
}

#[derive(Args, Debug)]
struct ResumeArgsRaw {
    // Note: This is the direct clap shape. We reinterpret the positional when --last is set
    // so "codex resume --last <prompt>" treats the positional as a prompt, not a session id.
    /// Conversation/session id (UUID) or thread name. UUIDs take precedence if it parses.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Resume the most recent recorded session (newest) without specifying an id.
    #[arg(long = "last", default_value_t = false)]
    last: bool,

    /// Show all sessions (disables cwd filtering).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    /// Optional image(s) to attach to the prompt sent after resuming.
    #[arg(
        long = "image",
        short = 'i',
        value_name = "FILE",
        value_delimiter = ',',
        num_args = 1
    )]
    images: Vec<PathBuf>,

    /// Prompt to send after resuming the session. If `-` is used, read from stdin.
    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    prompt: Option<String>,
}

#[derive(Debug)]
pub struct ResumeArgs {
    /// Conversation/session id (UUID) or thread name. UUIDs take precedence if it parses.
    /// If omitted, use --last to pick the most recent recorded session.
    pub session_id: Option<String>,

    /// Resume the most recent recorded session (newest) without specifying an id.
    pub last: bool,

    /// Show all sessions (disables cwd filtering).
    pub all: bool,

    /// Optional image(s) to attach to the prompt sent after resuming.
    pub images: Vec<PathBuf>,

    /// Prompt to send after resuming the session. If `-` is used, read from stdin.
    pub prompt: Option<String>,
}

impl From<ResumeArgsRaw> for ResumeArgs {
    fn from(raw: ResumeArgsRaw) -> Self {
        // When --last is used without an explicit prompt, treat the positional as the prompt
        // (clap can’t express this conditional positional meaning cleanly).
        let (session_id, prompt) = if raw.last && raw.prompt.is_none() {
            (None, raw.session_id)
        } else {
            (raw.session_id, raw.prompt)
        };
        Self {
            session_id,
            last: raw.last,
            all: raw.all,
            images: raw.images,
            prompt,
        }
    }
}

impl Args for ResumeArgs {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        ResumeArgsRaw::augment_args(cmd)
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        ResumeArgsRaw::augment_args_for_update(cmd)
    }
}

impl FromArgMatches for ResumeArgs {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        ResumeArgsRaw::from_arg_matches(matches).map(Self::from)
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = ResumeArgsRaw::from_arg_matches(matches).map(Self::from)?;
        Ok(())
    }
}

#[derive(Args, Debug)]
pub struct ReviewArgs {
    /// Review staged, unstaged, and untracked changes.
    #[arg(
        long = "uncommitted",
        default_value_t = false,
        conflicts_with_all = ["base", "base_commit", "commit"]
    )]
    pub uncommitted: bool,

    /// Review changes against the given base branch.
    #[arg(
        long = "base",
        value_name = "BRANCH",
        conflicts_with_all = ["uncommitted", "base_commit", "commit"]
    )]
    pub base: Option<String>,

    /// Review changes since the given base commit.
    #[arg(
        long = "base-commit",
        value_name = "SHA",
        conflicts_with_all = ["uncommitted", "base", "commit"]
    )]
    pub base_commit: Option<String>,

    /// Review the changes introduced by a commit.
    #[arg(
        long = "commit",
        value_name = "SHA",
        conflicts_with_all = ["uncommitted", "base", "base_commit"]
    )]
    pub commit: Option<String>,

    /// Optional commit title to display in the review summary.
    #[arg(long = "title", value_name = "TITLE", requires = "commit")]
    pub commit_title: Option<String>,

    /// Limit the review to one or more paths.
    #[arg(long = "files", alias = "paths", value_name = "PATH", num_args = 1..)]
    pub files: Vec<PathBuf>,

    /// Read additional review paths from a newline-delimited file.
    #[arg(long = "pathspec-from-file", value_name = "FILE")]
    pub pathspec_from_file: Option<PathBuf>,

    /// Limit the review to files under the given directory.
    #[arg(long = "dir", value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    pub dir: Option<PathBuf>,

    /// Supplemental reviewer instructions to combine with a native review target.
    #[arg(long = "instructions", value_name = "TEXT")]
    pub instructions: Option<String>,

    /// Read supplemental reviewer instructions from a file.
    #[arg(long = "instructions-file", value_name = "FILE")]
    pub instructions_file: Option<PathBuf>,

    /// Write the structured review result to a JSON file.
    #[arg(long = "output-review-json", value_name = "FILE")]
    pub output_review_json: Option<PathBuf>,

    /// Interrupt the review if it runs longer than this duration, e.g. 300s or 5m.
    #[arg(long = "timeout", value_name = "DURATION", value_parser = parse_review_timeout_ms)]
    pub timeout_ms: Option<u64>,

    /// Exit with status 1 when the review reports findings.
    #[arg(long = "fail-on-findings", default_value_t = false)]
    pub fail_on_findings: bool,

    /// Custom review instructions. If `-` is used, read from stdin.
    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    pub prompt: Option<String>,
}

fn parse_review_timeout_ms(value: &str) -> Result<u64, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("timeout must not be empty".to_string());
    }
    let (number, multiplier) = if let Some(number) = trimmed.strip_suffix("ms") {
        (number, 1)
    } else if let Some(number) = trimmed.strip_suffix('s') {
        (number, 1_000)
    } else if let Some(number) = trimmed.strip_suffix('m') {
        (number, 60_000)
    } else {
        (trimmed, 1_000)
    };
    let amount = number.parse::<u64>().map_err(|_| {
        "timeout must be a positive integer with optional ms, s, or m suffix".to_string()
    })?;
    if amount == 0 {
        return Err("timeout must be greater than zero".to_string());
    }
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| "timeout is too large".to_string())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum Color {
    Always,
    Never,
    #[default]
    Auto,
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
