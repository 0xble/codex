use clap::Parser;
use codex_tui::Cli;

// Fork-only regression test. The `--session-id` flag and its
// `session_id_override` field let external wrappers keep their session name
// aligned with the Codex thread id.
#[test]
fn session_id_flag_sets_override() {
    let cli = Cli::try_parse_from([
        "codex",
        "--session-id",
        "00000000-0000-0000-0000-000000000001",
    ])
    .expect("--session-id must be a valid top-level flag");
    assert_eq!(
        cli.session_id_override.as_deref(),
        Some("00000000-0000-0000-0000-000000000001"),
    );
}

#[test]
fn session_id_flag_default_is_none() {
    let cli = Cli::try_parse_from(["codex"]).expect("base codex must parse without args");
    assert!(cli.session_id_override.is_none());
}
