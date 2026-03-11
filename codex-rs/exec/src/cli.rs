use clap::Args;
use clap::FromArgMatches;
use clap::Parser;
use clap::ValueEnum;
use codex_utils_cli::CliConfigOverrides;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version)]
pub struct Cli {
    /// Action to perform. If omitted, runs a new non-interactive session.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Optional image(s) to attach to the initial prompt.
    #[arg(
        long = "image",
        short = 'i',
        value_name = "FILE",
        value_delimiter = ',',
        num_args = 1..
    )]
    pub images: Vec<PathBuf>,

    /// Model the agent should use.
    #[arg(long, short = 'm', global = true)]
    pub model: Option<String>,

    /// Use open-source provider.
    #[arg(long = "oss", default_value_t = false)]
    pub oss: bool,

    /// Specify which local provider to use (lmstudio or ollama).
    /// If not specified with --oss, will use config default or show selection.
    #[arg(long = "local-provider")]
    pub oss_provider: Option<String>,

    /// Select the sandbox policy to use when executing model-generated shell
    /// commands.
    #[arg(long = "sandbox", short = 's', value_enum)]
    pub sandbox_mode: Option<codex_utils_cli::SandboxModeCliArg>,

    /// Configuration profile from config.toml to specify default options.
    #[arg(long = "profile", short = 'p')]
    pub config_profile: Option<String>,

    /// Convenience alias for low-friction sandboxed automatic execution (-a on-request, --sandbox workspace-write).
    #[arg(long = "full-auto", default_value_t = false, global = true)]
    pub full_auto: bool,

    /// Skip all confirmation prompts and execute commands without sandboxing.
    /// EXTREMELY DANGEROUS. Intended solely for running in environments that are externally sandboxed.
    #[arg(
        long = "dangerously-bypass-approvals-and-sandbox",
        alias = "yolo",
        default_value_t = false,
        global = true,
        conflicts_with = "full_auto"
    )]
    pub dangerously_bypass_approvals_and_sandbox: bool,

    /// Tell the agent to use the specified directory as its working root.
    #[clap(long = "cd", short = 'C', value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Allow running Codex outside a Git repository.
    #[arg(long = "skip-git-repo-check", global = true, default_value_t = false)]
    pub skip_git_repo_check: bool,

    /// Additional directories that should be writable alongside the primary workspace.
    #[arg(long = "add-dir", value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    pub add_dir: Vec<PathBuf>,

    /// Run without persisting session files to disk.
    #[arg(long = "ephemeral", global = true, default_value_t = false)]
    pub ephemeral: bool,

    /// Path to a JSON Schema file describing the model's final response shape.
    #[arg(long = "output-schema", value_name = "FILE")]
    pub output_schema: Option<PathBuf>,

    /// Start a fresh session with the provided session/thread id (UUID).
    #[arg(long = "session-id", alias = "thread-id", value_name = "SESSION_ID")]
    pub session_id: Option<String>,

    #[clap(skip)]
    pub config_overrides: CliConfigOverrides,

    /// Specifies color settings for use in the output.
    #[arg(long = "color", value_enum, default_value_t = Color::Auto)]
    pub color: Color,

    /// Force cursor-based progress updates in exec mode.
    #[arg(long = "progress-cursor", default_value_t = false)]
    pub progress_cursor: bool,

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
    /// if `-` is used), instructions are read from stdin.
    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    pub prompt: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Resume a previous session by id or pick the most recent with --last.
    Resume(ResumeArgs),

    /// Run a code review against the current repository.
    Review(ReviewArgs),
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

#[derive(Parser, Debug)]
pub struct ReviewArgs {
    /// Review staged, unstaged, and untracked changes.
    #[arg(
        long = "uncommitted",
        default_value_t = false,
        conflicts_with_all = ["staged", "base", "commit", "instructions", "prompt"]
    )]
    pub uncommitted: bool,

    /// Review staged changes only.
    #[arg(
        long = "staged",
        default_value_t = false,
        conflicts_with_all = ["uncommitted", "base", "commit", "instructions", "prompt"]
    )]
    pub staged: bool,

    /// Review changes against the given base branch.
    #[arg(
        long = "base",
        value_name = "BRANCH",
        conflicts_with_all = ["uncommitted", "staged", "commit", "instructions", "prompt"]
    )]
    pub base: Option<String>,

    /// Review the changes introduced by a commit.
    #[arg(
        long = "commit",
        value_name = "SHA",
        conflicts_with_all = ["uncommitted", "staged", "base", "instructions", "prompt"]
    )]
    pub commit: Option<String>,

    /// Optional commit title to display in the review summary.
    #[arg(long = "title", value_name = "TITLE", requires = "commit")]
    pub commit_title: Option<String>,

    /// Restrict review findings to the given repo-relative paths.
    ///
    /// You can either repeat the flag (`--paths a --paths b`) or pass
    /// multiple paths after a single flag (`--paths a b`).
    ///
    /// For multi-path custom reviews, prefer `--instructions`; if you use the
    /// positional prompt instead, separate it with `--`.
    #[arg(
        long = "paths",
        value_name = "PATH",
        num_args = 1..,
        action = clap::ArgAction::Append,
        conflicts_with = "pathspec_from_file"
    )]
    pub paths: Vec<String>,

    /// Read review pathspecs from a file, one path per line.
    #[arg(
        long = "pathspec-from-file",
        value_name = "FILE",
        value_hint = clap::ValueHint::FilePath,
        conflicts_with = "paths"
    )]
    pub pathspec_from_file: Option<PathBuf>,

    /// Custom review instructions, as a flag instead of the positional prompt.
    #[arg(
        long = "instructions",
        value_name = "PROMPT",
        conflicts_with_all = ["uncommitted", "staged", "base", "commit", "prompt"],
        value_hint = clap::ValueHint::Other
    )]
    pub instructions: Option<String>,

    /// Custom review instructions. If `-` is used, read from stdin.
    #[arg(value_name = "PROMPT", value_hint = clap::ValueHint::Other)]
    pub prompt: Option<String>,
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
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn resume_parses_prompt_after_global_flags() {
        const PROMPT: &str = "echo resume-with-global-flags-after-subcommand";
        let cli = Cli::parse_from([
            "codex-exec",
            "resume",
            "--last",
            "--json",
            "--model",
            "gpt-5.2-codex",
            "--dangerously-bypass-approvals-and-sandbox",
            "--skip-git-repo-check",
            "--ephemeral",
            PROMPT,
        ]);

        assert!(cli.ephemeral);
        let Some(Command::Resume(args)) = cli.command else {
            panic!("expected resume command");
        };
        let effective_prompt = args.prompt.clone().or_else(|| {
            if args.last {
                args.session_id.clone()
            } else {
                None
            }
        });
        assert_eq!(effective_prompt.as_deref(), Some(PROMPT));
    }

    #[test]
    fn resume_accepts_output_last_message_flag_after_subcommand() {
        const PROMPT: &str = "echo resume-with-output-file";
        let cli = Cli::parse_from([
            "codex-exec",
            "resume",
            "session-123",
            "-o",
            "/tmp/resume-output.md",
            PROMPT,
        ]);

        assert_eq!(
            cli.last_message_file,
            Some(PathBuf::from("/tmp/resume-output.md"))
        );
        let Some(Command::Resume(args)) = cli.command else {
            panic!("expected resume command");
        };
        assert_eq!(args.session_id.as_deref(), Some("session-123"));
        assert_eq!(args.prompt.as_deref(), Some(PROMPT));
    }

    #[test]
    fn review_accepts_staged_flag() {
        let cli = Cli::parse_from(["codex-exec", "review", "--staged"]);

        let Some(Command::Review(args)) = cli.command else {
            panic!("expected review command");
        };
        assert!(args.staged);
        assert!(!args.uncommitted);
        assert_eq!(args.base, None);
        assert_eq!(args.commit, None);
        assert_eq!(args.prompt, None);
    }

    #[test]
    fn review_accepts_paths_flag() {
        let cli = Cli::parse_from([
            "codex-exec",
            "review",
            "--uncommitted",
            "--paths",
            "src/lib.rs",
            "--paths",
            "src/main.rs",
        ]);

        let Some(Command::Review(args)) = cli.command else {
            panic!("expected review command");
        };
        assert!(args.uncommitted);
        assert_eq!(args.paths, vec!["src/lib.rs", "src/main.rs"]);
        assert_eq!(args.pathspec_from_file, None);
    }

    #[test]
    fn review_accepts_multiple_paths_after_single_flag() {
        let cli = Cli::parse_from([
            "codex-exec",
            "review",
            "--uncommitted",
            "--paths",
            "src/lib.rs",
            "src/main.rs",
        ]);

        let Some(Command::Review(args)) = cli.command else {
            panic!("expected review command");
        };
        assert!(args.uncommitted);
        assert_eq!(args.paths, vec!["src/lib.rs", "src/main.rs"]);
        assert_eq!(args.pathspec_from_file, None);
    }

    #[test]
    fn review_accepts_multiple_paths_with_prompt_separator() {
        let cli = Cli::parse_from([
            "codex-exec",
            "review",
            "--paths",
            "src/lib.rs",
            "src/main.rs",
            "--",
            "focus on ergonomics",
        ]);

        let Some(Command::Review(args)) = cli.command else {
            panic!("expected review command");
        };
        assert!(!args.uncommitted);
        assert_eq!(args.paths, vec!["src/lib.rs", "src/main.rs"]);
        assert_eq!(args.prompt.as_deref(), Some("focus on ergonomics"));
    }

    #[test]
    fn review_accepts_instructions_flag_with_multiple_paths() {
        let cli = Cli::parse_from([
            "codex-exec",
            "review",
            "--paths",
            "src/lib.rs",
            "src/main.rs",
            "--instructions",
            "focus on ergonomics",
        ]);

        let Some(Command::Review(args)) = cli.command else {
            panic!("expected review command");
        };
        assert_eq!(args.paths, vec!["src/lib.rs", "src/main.rs"]);
        assert_eq!(args.instructions.as_deref(), Some("focus on ergonomics"));
        assert_eq!(args.prompt, None);
    }

    #[test]
    fn review_rejects_instructions_with_uncommitted_target() {
        let err = Cli::try_parse_from([
            "codex-exec",
            "review",
            "--uncommitted",
            "--instructions",
            "focus on ergonomics",
        ])
        .expect_err("expected clap to reject mixed review target and instructions");

        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn review_accepts_pathspec_from_file_flag() {
        let cli = Cli::parse_from([
            "codex-exec",
            "review",
            "--uncommitted",
            "--pathspec-from-file",
            "review-files.txt",
        ]);

        let Some(Command::Review(args)) = cli.command else {
            panic!("expected review command");
        };
        assert!(args.uncommitted);
        assert!(args.paths.is_empty());
        assert_eq!(
            args.pathspec_from_file,
            Some(PathBuf::from("review-files.txt"))
        );
    }

    #[test]
    fn parses_requested_session_id_for_fresh_session() {
        let cli = Cli::parse_from([
            "codex-exec",
            "--session-id",
            "123e4567-e89b-12d3-a456-426614174000",
            "hello",
        ]);

        assert_eq!(
            cli.session_id.as_deref(),
            Some("123e4567-e89b-12d3-a456-426614174000")
        );
        assert_eq!(cli.prompt.as_deref(), Some("hello"));
        assert!(cli.command.is_none());
    }
}
