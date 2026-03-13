#![cfg(not(target_os = "windows"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use codex_core::REVIEW_PROMPT;
use codex_core::review_format::render_review_output_text;
use codex_protocol::protocol::ReviewCodeLocation;
use codex_protocol::protocol::ReviewFinding;
use codex_protocol::protocol::ReviewLineRange;
use codex_protocol::protocol::ReviewOutputEvent;
use core_test_support::load_sse_fixture_with_id_from_str;
use core_test_support::responses::mount_response_once;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::sse_response;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex_exec::test_codex_exec;
use pretty_assertions::assert_eq;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;
use tempfile::NamedTempFile;
use uuid::Uuid;

fn init_repo_with_uncommitted_change(repo: &Path) {
    let envs = [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ];

    let init_output = Command::new("git")
        .envs(envs)
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("git init");
    assert!(init_output.status.success(), "git init failed");

    Command::new("git")
        .envs(envs)
        .args(["config", "user.name", "Exec Review Test"])
        .current_dir(repo)
        .output()
        .expect("git config user.name");
    Command::new("git")
        .envs(envs)
        .args(["config", "user.email", "exec-review-test@example.com"])
        .current_dir(repo)
        .output()
        .expect("git config user.email");

    let file = repo.join("review-target.txt");
    std::fs::write(&file, "before\n").expect("write baseline file");

    let add_output = Command::new("git")
        .envs(envs)
        .args(["add", "."])
        .current_dir(repo)
        .output()
        .expect("git add baseline");
    assert!(add_output.status.success(), "git add baseline failed");

    let commit_output = Command::new("git")
        .envs(envs)
        .args(["commit", "-m", "baseline"])
        .current_dir(repo)
        .output()
        .expect("git commit baseline");
    assert!(commit_output.status.success(), "git commit baseline failed");

    std::fs::write(&file, "before\nafter\n").expect("write modified file");
}

fn print_command_output(stdout: &str, stderr: &str) {
    println!("stdout:\n{stdout}");
    println!("stderr:\n{stderr}");
}

fn assert_uncommitted_review_request_shape(
    request: &core_test_support::responses::ResponsesRequest,
) {
    let body = request.body_json();
    let input = body["input"].as_array().expect("input array");
    let texts: Vec<&str> = input
        .iter()
        .filter_map(|msg| msg.get("content").and_then(|content| content.as_array()))
        .flat_map(|content| content.iter())
        .filter_map(|entry| entry.get("text").and_then(|text| text.as_str()))
        .collect();

    assert!(
        texts.iter().any(|text| {
            *text
                == "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings."
        }),
        "expected uncommitted review prompt in request body"
    );
    assert_eq!(
        Some(REVIEW_PROMPT),
        body["instructions"].as_str(),
        "expected review rubric in instructions"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_command_emits_structured_review_to_stdout_and_last_message_file()
-> anyhow::Result<()> {
    let test = test_codex_exec();
    init_repo_with_uncommitted_change(test.cwd_path());

    let expected = ReviewOutputEvent {
        findings: vec![ReviewFinding {
            title: "Prefer helper".to_string(),
            body: "Use a helper to avoid duplicate logic.".to_string(),
            confidence_score: 0.91,
            priority: 1,
            code_location: ReviewCodeLocation {
                absolute_file_path: PathBuf::from("/tmp/review-target.txt"),
                line_range: ReviewLineRange { start: 1, end: 2 },
            },
        }],
        overall_correctness: "good".to_string(),
        overall_explanation: "Looks correct with one maintainability note.".to_string(),
        overall_confidence_score: 0.82,
    };
    let review_json = serde_json::to_string(&expected).expect("serialize review output");
    let sse_body = sse(vec![
        serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": review_json}]
            }
        }),
        serde_json::json!({
            "type": "response.completed",
            "response": {"id": "resp-structured"}
        }),
    ]);

    let server = start_mock_server().await;
    let request_log = mount_sse_once(&server, sse_body).await;
    let output_file = NamedTempFile::new()?;
    let expected_text = render_review_output_text(&expected);

    let mut cmd = test.cmd_with_server(&server);
    cmd.timeout(Duration::from_secs(20))
        .arg("review")
        .arg("--uncommitted")
        .arg("--output-last-message")
        .arg(output_file.path());

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    print_command_output(&stdout, &stderr);

    assert!(output.status.success(), "command failed: {}", output.status);
    assert_eq!(format!("{expected_text}\n"), stdout);
    assert_eq!(expected_text, std::fs::read_to_string(output_file.path())?);
    assert!(
        !stderr.contains("Warning: no last agent message"),
        "unexpected last-message warning in stderr"
    );

    let request = request_log.single_request();
    assert_uncommitted_review_request_shape(&request);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_command_emits_plain_text_fallback_to_stdout_and_last_message_file()
-> anyhow::Result<()> {
    let test = test_codex_exec();
    init_repo_with_uncommitted_change(test.cwd_path());

    let sse_body = sse(vec![
        serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "just plain text"}]
            }
        }),
        serde_json::json!({
            "type": "response.completed",
            "response": {"id": "resp-plain"}
        }),
    ]);

    let server = start_mock_server().await;
    mount_sse_once(&server, sse_body).await;
    let output_file = NamedTempFile::new()?;
    let expected_text = render_review_output_text(&ReviewOutputEvent {
        overall_explanation: "just plain text".to_string(),
        ..Default::default()
    });

    let mut cmd = test.cmd_with_server(&server);
    cmd.timeout(Duration::from_secs(20))
        .arg("review")
        .arg("--uncommitted")
        .arg("--output-last-message")
        .arg(output_file.path());

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    print_command_output(&stdout, &stderr);

    assert!(output.status.success(), "command failed: {}", output.status);
    assert_eq!(format!("{expected_text}\n"), stdout);
    assert_eq!(expected_text, std::fs::read_to_string(output_file.path())?);
    assert!(
        !stderr.contains("Warning: no last agent message"),
        "unexpected last-message warning in stderr"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_command_times_out_with_explicit_final_message_and_no_hang() -> anyhow::Result<()> {
    let test = test_codex_exec();
    init_repo_with_uncommitted_change(test.cwd_path());

    let server = start_mock_server().await;
    let sse_raw = r#"[
        {"type":"response.completed", "response": {"id": "__ID__"}}
    ]"#;
    let sse = load_sse_fixture_with_id_from_str(sse_raw, &Uuid::new_v4().to_string());
    mount_response_once(
        &server,
        sse_response(sse).set_delay(Duration::from_millis(400)),
    )
    .await;

    let output_file = NamedTempFile::new()?;
    let expected_text = "Review timed out waiting for reviewer progress. Please narrow the review scope and try again.";

    let mut cmd = test.cmd_with_server(&server);
    cmd.timeout(Duration::from_secs(30))
        .env("CODEX_REVIEW_IDLE_TIMEOUT_MS", "50")
        .arg("review")
        .arg("--uncommitted")
        .arg("--output-last-message")
        .arg(output_file.path());

    let started = Instant::now();
    let output = cmd.output()?;
    let elapsed = started.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    print_command_output(&stdout, &stderr);

    assert_eq!(
        Some(1),
        output.status.code(),
        "idle timeout should exit non-zero so automation can detect it"
    );
    assert!(
        elapsed < Duration::from_secs(8),
        "review command should time out quickly, took {elapsed:?}"
    );
    assert_eq!(format!("{expected_text}\n"), stdout);
    assert_eq!(expected_text, std::fs::read_to_string(output_file.path())?);
    assert!(
        stderr.contains("review delegate made no progress"),
        "expected idle-timeout diagnostic in stderr"
    );
    assert!(
        !stderr.contains("Warning: no last agent message"),
        "unexpected last-message warning in stderr"
    );

    Ok(())
}
