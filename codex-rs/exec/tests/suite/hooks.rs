#![cfg(not(target_os = "windows"))]

use codex_state::LogQuery;
use codex_state::SqliteConfig;
use codex_state::StateRuntime;
use codex_utils_absolute_path::AbsolutePathBuf;
use core_test_support::responses;
use core_test_support::test_codex_exec::test_codex_exec;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_hook_trust_bypass_runs_session_start_hook() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let marker_path = test.home_path().join("session-start-ran");
    let command = format!("touch {}", marker_path.display());
    std::fs::write(
        test.home_path().join("hooks.json"),
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": command,
                    }],
                }],
            },
        }))?,
    )?;

    let server = responses::start_mock_server().await;
    let body = responses::sse(vec![
        responses::ev_response_created("response_1"),
        responses::ev_assistant_message("response_1", "done"),
        responses::ev_completed("response_1"),
    ]);
    responses::mount_sse_once(&server, body).await;

    test.cmd_with_server(&server)
        .arg("--skip-git-repo-check")
        .arg("--dangerously-bypass-hook-trust")
        .arg("run the session start hook")
        .assert()
        .success();

    assert!(marker_path.exists(), "session start hook did not run");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_flushes_async_hook_metadata_without_inference_or_log_env() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let marker = test.home_path().join("async-hook-ran");
    std::fs::write(
        test.home_path().join("hooks.json"),
        serde_json::to_vec_pretty(&json!({"hooks": {
            "SessionStart": [{"hooks": [{
                "type": "command", "async": true,
                "command": format!("touch '{}'; printf '%s' '{{\"hookSpecificOutput\":{{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"private-async-output\"}}}}'", marker.display()),
            }]}],
            "UserPromptSubmit": [{"hooks": [{
                "type": "command",
                "command": format!("while [ ! -f '{}' ]; do sleep 0.01; done; sleep 0.05; printf '%s' '{{\"decision\":\"block\",\"reason\":\"private-sync-output\"}}'", marker.display()),
                "timeout": 5,
            }]}],
        }}))?,
    )?;
    test.cmd()
        .env_remove("RUST_LOG")
        .arg("--skip-git-repo-check")
        .arg("--dangerously-bypass-hook-trust")
        .arg("-c")
        .arg("features.hooks=true")
        .arg("-c")
        .arg("openai_base_url=\"http://127.0.0.1:9/v1\"")
        .arg("Block this audit before inference.")
        .assert()
        .success();

    let state = StateRuntime::init(
        SqliteConfig::from_sqlite_home(AbsolutePathBuf::try_from(test.home_path())?),
        "openai".to_string(),
    )
    .await?;
    let query = |search: &str| LogQuery {
        module_like: vec!["codex_hooks%".to_string()],
        search: Some(search.to_string()),
        ..Default::default()
    };
    assert_eq!(state.query_logs(&query("hook started")).await?.len(), 1);
    assert_eq!(state.query_logs(&query("hook completed")).await?.len(), 1);
    for field in [
        "execution_id=",
        "handler_id=",
        "event_name=",
        "source_path=",
        "status=Completed",
        "duration_ms=",
    ] {
        assert!(
            !state.query_logs(&query(field)).await?.is_empty(),
            "missing {field}"
        );
    }
    for output in ["private-async-output", "private-sync-output"] {
        assert!(state.query_logs(&query(output)).await?.is_empty());
    }
    Ok(())
}
