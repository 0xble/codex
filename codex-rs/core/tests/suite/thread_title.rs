#![allow(clippy::unwrap_used)]

use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once_match;
use core_test_support::responses::sse;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use core_test_support::wait_for_event_match;
use wiremock::MockServer;
use wiremock::matchers::body_string_contains;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_turn_title_uses_thread_title_model_and_low_reasoning() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;

    let title_mock = mount_sse_once_match(
        &server,
        body_string_contains("Generate a concise, sentence-case title (3-7 words)"),
        sse(vec![
            ev_response_created("resp-title"),
            ev_assistant_message(
                "msg-title",
                "{\"title\":\"Fix terminal title naming flow\"}",
            ),
            ev_completed("resp-title"),
        ]),
    )
    .await;

    let turn_mock = mount_sse_once_match(
        &server,
        body_string_contains("\"model\":\"gpt-5.1\""),
        sse(vec![
            ev_response_created("resp-turn"),
            ev_assistant_message("msg-turn", "done"),
            ev_completed("resp-turn"),
        ]),
    )
    .await;

    let mut builder = test_codex().with_model("gpt-5.1").with_config(|config| {
        config.thread_title_model = Some("gpt-5.1".to_string());
        config.thread_title_reasoning_effort = None;
    });
    let test = builder.build(&server).await?;

    test.codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "fix the terminal title naming flow".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: test.cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: test.session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let title_event = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::ThreadNameUpdated(event) => Some(event.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        title_event.thread_name.as_deref(),
        Some("Fix terminal title naming flow")
    );

    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let title_requests = title_mock.requests();
    assert!(
        !title_requests.is_empty(),
        "expected at least one title-generation request"
    );
    let title_request = title_requests
        .iter()
        .find(|request| {
            request.body_json()["model"].as_str() == Some("gpt-5.1")
                && request
                    .instructions_text()
                    .contains("Generate a concise, sentence-case title (3-7 words)")
        })
        .unwrap();
    assert_eq!(
        title_request.body_json()["reasoning"]["effort"].as_str(),
        Some("low")
    );
    assert!(
        title_request
            .instructions_text()
            .contains("Return JSON with a single \"title\" field."),
        "expected Claude-style title instructions"
    );
    assert!(
        title_request
            .message_input_texts("user")
            .iter()
            .any(|text| text.contains("fix the terminal title naming flow")),
        "expected original request to be passed into the rename prompt"
    );

    let turn_request = turn_mock.single_request();
    assert_eq!(turn_request.body_json()["model"].as_str(), Some("gpt-5.1"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_turn_title_falls_back_to_session_model_when_unset() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;

    let title_mock = mount_sse_once_match(
        &server,
        body_string_contains("Generate a concise, sentence-case title (3-7 words)"),
        sse(vec![
            ev_response_created("resp-title"),
            ev_assistant_message("msg-title", "{\"title\":\"Keep session model for titles\"}"),
            ev_completed("resp-title"),
        ]),
    )
    .await;

    let turn_mock = mount_sse_once_match(
        &server,
        body_string_contains("\"model\":\"gpt-5.3-codex\""),
        sse(vec![
            ev_response_created("resp-turn"),
            ev_assistant_message("msg-turn", "done"),
            ev_completed("resp-turn"),
        ]),
    )
    .await;

    let mut builder = test_codex()
        .with_model("gpt-5.3-codex")
        .with_config(|config| {
            config.thread_title_model = None;
            config.thread_title_reasoning_effort = None;
        });
    let test = builder.build(&server).await?;

    test.codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "keep the session model for title generation".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: test.cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: test.session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let title_event = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::ThreadNameUpdated(event) => Some(event.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        title_event.thread_name.as_deref(),
        Some("Keep session model for titles")
    );

    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let title_request = title_mock
        .requests()
        .into_iter()
        .find(|request| {
            request
                .message_input_texts("user")
                .iter()
                .any(|text| text.contains("Working directory:"))
        })
        .expect("expected a title-generation request");
    assert_eq!(
        title_request.body_json()["model"].as_str(),
        Some("gpt-5.3-codex")
    );

    let turn_request = turn_mock
        .requests()
        .into_iter()
        .find(|request| {
            request
                .message_input_texts("user")
                .iter()
                .all(|text| !text.contains("Working directory:"))
        })
        .expect("expected a non-title turn request");
    assert_eq!(
        turn_request.body_json()["model"].as_str(),
        Some("gpt-5.3-codex")
    );

    Ok(())
}
