use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentMessageContentDeltaEvent;
use codex_protocol::protocol::AgentMessageDeltaEvent;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExitedReviewModeEvent;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ReviewOutputEvent;
use codex_protocol::protocol::SubAgentSource;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::codex::Session;
use crate::codex::TurnContext;
use crate::codex_delegate::run_codex_thread_one_shot;
use crate::config::Constrained;
use crate::features::Feature;
use crate::review_format::format_review_findings_block;
use crate::review_format::render_review_output_text;
use crate::state::TaskKind;
use codex_protocol::user_input::UserInput;

use super::SessionTask;
use super::SessionTaskContext;

#[derive(Clone, Copy)]
pub(crate) struct ReviewTask;

impl ReviewTask {
    pub(crate) fn new() -> Self {
        Self
    }
}

fn review_idle_timeout() -> Duration {
    if let Some(timeout_ms) = std::env::var("CODEX_REVIEW_IDLE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_millis(timeout_ms);
    }

    Duration::from_secs(90)
}

#[async_trait]
impl SessionTask for ReviewTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Review
    }

    fn span_name(&self) -> &'static str {
        "session_task.review"
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let review_cancellation_token = cancellation_token.child_token();
        let _ = session
            .session
            .services
            .session_telemetry
            .counter("codex.task.review", 1, &[]);

        // Start sub-codex conversation and get the receiver for events.
        let output = match start_review_conversation(
            session.clone(),
            ctx.clone(),
            input,
            review_cancellation_token.clone(),
        )
        .await
        {
            Some(receiver) => {
                process_review_events(
                    session.clone(),
                    ctx.clone(),
                    receiver,
                    review_cancellation_token,
                )
                .await
            }
            None => None,
        };
        if !cancellation_token.is_cancelled() {
            exit_review_mode(session.clone_session(), output.clone(), ctx.clone()).await;
        }
        None
    }

    async fn abort(&self, session: Arc<SessionTaskContext>, ctx: Arc<TurnContext>) {
        exit_review_mode(session.clone_session(), None, ctx).await;
    }
}

async fn start_review_conversation(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    input: Vec<UserInput>,
    cancellation_token: CancellationToken,
) -> Option<async_channel::Receiver<Event>> {
    let config = ctx.config.clone();
    let mut sub_agent_config = config.as_ref().clone();
    // Carry over review-only feature restrictions so the delegate cannot
    // re-enable blocked tools (web search, collab tools, view image).
    if let Err(err) = sub_agent_config
        .web_search_mode
        .set(WebSearchMode::Disabled)
    {
        panic!("by construction Constrained<WebSearchMode> must always support Disabled: {err}");
    }
    let _ = sub_agent_config.features.disable(Feature::Collab);

    // Set explicit review rubric for the sub-agent
    sub_agent_config.base_instructions = Some(crate::REVIEW_PROMPT.to_string());
    sub_agent_config.permissions.approval_policy = Constrained::allow_only(AskForApproval::Never);

    let model = config
        .review_model
        .clone()
        .unwrap_or_else(|| ctx.model_info.slug.clone());
    sub_agent_config.model = Some(model);
    (run_codex_thread_one_shot(
        sub_agent_config,
        session.auth_manager(),
        session.models_manager(),
        input,
        session.clone_session(),
        ctx.clone(),
        cancellation_token,
        SubAgentSource::Review,
        None,
        None,
    )
    .await)
        .ok()
        .map(|io| io.rx_event)
}

async fn process_review_events(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    receiver: async_channel::Receiver<Event>,
    review_cancellation_token: CancellationToken,
) -> Option<ReviewOutputEvent> {
    let mut prev_agent_message: Option<Event> = None;
    loop {
        let event = match timeout(review_idle_timeout(), receiver.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(_)) => return None,
            Err(_) => {
                if review_cancellation_token.is_cancelled() {
                    return None;
                }

                review_cancellation_token.cancel();
                session
                    .clone_session()
                    .send_event(
                        ctx.as_ref(),
                        EventMsg::Error(ErrorEvent {
                            message: format!(
                                "review delegate made no progress for {}; cancelling review",
                                review_idle_timeout().as_secs_f32()
                            ),
                            codex_error_info: None,
                        }),
                    )
                    .await;
                return None;
            }
        };

        match event.clone().msg {
            EventMsg::AgentMessage(_) => {
                if let Some(prev) = prev_agent_message.take() {
                    session
                        .clone_session()
                        .send_event(ctx.as_ref(), prev.msg)
                        .await;
                }
                prev_agent_message = Some(event);
            }
            // Suppress ItemCompleted only for assistant messages: forwarding it
            // would trigger legacy AgentMessage via as_legacy_events(), which this
            // review flow intentionally hides in favor of structured output.
            EventMsg::ItemCompleted(ItemCompletedEvent {
                item: TurnItem::AgentMessage(_),
                ..
            })
            | EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { .. })
            | EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent { .. }) => {}
            EventMsg::TurnComplete(task_complete) => {
                // Parse review output from the last agent message (if present).
                let out = task_complete
                    .last_agent_message
                    .as_deref()
                    .map(parse_review_output_event);
                return out;
            }
            EventMsg::TurnAborted(_) => {
                // Cancellation or abort: consumer will finalize with None.
                return None;
            }
            other => {
                session
                    .clone_session()
                    .send_event(ctx.as_ref(), other)
                    .await;
            }
        }
    }
}

/// Parse a ReviewOutputEvent from a text blob returned by the reviewer model.
/// If the text is valid JSON matching ReviewOutputEvent, deserialize it.
/// Otherwise, attempt to extract the first JSON object substring and parse it.
/// If parsing still fails, return a structured fallback carrying the plain text
/// in `overall_explanation`.
fn parse_review_output_event(text: &str) -> ReviewOutputEvent {
    if let Ok(ev) = serde_json::from_str::<ReviewOutputEvent>(text) {
        return ev;
    }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}'))
        && start < end
        && let Some(slice) = text.get(start..=end)
        && let Ok(ev) = serde_json::from_str::<ReviewOutputEvent>(slice)
    {
        return ev;
    }
    ReviewOutputEvent {
        overall_explanation: text.to_string(),
        ..Default::default()
    }
}

/// Emits an ExitedReviewMode Event with optional ReviewOutput,
/// and records a developer message with the review output.
pub(crate) async fn exit_review_mode(
    session: Arc<Session>,
    review_output: Option<ReviewOutputEvent>,
    ctx: Arc<TurnContext>,
) {
    const REVIEW_USER_MESSAGE_ID: &str = "review_rollout_user";
    const REVIEW_ASSISTANT_MESSAGE_ID: &str = "review_rollout_assistant";
    let (user_message, assistant_message) = if let Some(out) = review_output.clone() {
        let mut findings_str = String::new();
        let text = out.overall_explanation.trim();
        if !text.is_empty() {
            findings_str.push_str(text);
        }
        if !out.findings.is_empty() {
            let block = format_review_findings_block(&out.findings, None);
            findings_str.push_str(&format!("\n{block}"));
        }
        let rendered =
            crate::client_common::REVIEW_EXIT_SUCCESS_TMPL.replace("{results}", &findings_str);
        let assistant_message = render_review_output_text(&out);
        (rendered, assistant_message)
    } else {
        let rendered = crate::client_common::REVIEW_EXIT_INTERRUPTED_TMPL.to_string();
        let assistant_message =
            "Review was interrupted. Please re-run /review and wait for it to complete."
                .to_string();
        (rendered, assistant_message)
    };

    session
        .record_conversation_items(
            &ctx,
            &[ResponseItem::Message {
                id: Some(REVIEW_USER_MESSAGE_ID.to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText { text: user_message }],
                end_turn: None,
                phase: None,
            }],
        )
        .await;

    session
        .send_event(
            ctx.as_ref(),
            EventMsg::ExitedReviewMode(ExitedReviewModeEvent { review_output }),
        )
        .await;
    session
        .record_response_item_and_emit_turn_item(
            ctx.as_ref(),
            ResponseItem::Message {
                id: Some(REVIEW_ASSISTANT_MESSAGE_ID.to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: assistant_message,
                }],
                end_turn: None,
                phase: None,
            },
        )
        .await;

    // Review turns can run before any regular user turn, so explicitly
    // materialize rollout persistence. Do this after emitting review output so
    // file creation + git metadata collection cannot delay client-facing items.
    session.ensure_rollout_materialized().await;
}
