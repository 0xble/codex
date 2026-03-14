use std::path::Path;

use crate::ModelClientSession;
use crate::client_common::Prompt;
use crate::config::Config;
use crate::models_manager::manager::RefreshStrategy;
use crate::state::SessionServices;
use crate::util::normalize_thread_name;
use codex_otel::SessionTelemetry;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use futures::StreamExt;
use tracing::debug;
use tracing::warn;

const TITLE_TARGET_MAX_CHARS: usize = 24;
const TITLE_TARGET_MIN_CHARS: usize = 13;
const TITLE_MAX_WORDS: usize = 4;
const PREFERRED_TITLE_MODELS: &[&str] = &[
    "gpt-5.1-codex-mini",
    "gpt-5-mini",
    "gpt-5.1-mini",
    "o4-mini",
];

pub(crate) async fn generate_thread_title(
    services: &SessionServices,
    config: &Config,
    cwd: &Path,
    prompt_text: &str,
) -> Option<String> {
    let prompt_text = compact_title_prompt(prompt_text);
    if prompt_text.is_empty()
        || is_low_signal_prompt(&prompt_text)
        || !is_informative_prompt(&prompt_text)
    {
        return None;
    }

    let models = services
        .models_manager
        .list_models(RefreshStrategy::OnlineIfUncached)
        .await;
    let model = select_title_model(models.as_slice())?;
    let model_info = services
        .models_manager
        .get_model_info(model.as_str(), config)
        .await;

    let prompt = build_generation_prompt(format!(
        "Working directory: {}\nUser request: {}",
        cwd.display(),
        prompt_text
    ));
    let mut client_session = services.model_client.new_session();
    let mut title = collect_model_text(
        &mut client_session,
        &prompt,
        &model_info,
        &services.session_telemetry,
    )
    .await?;

    title = sanitize_generated_title(&title)?;
    if !title_fits_constraints(&title) {
        let repair_prompt = build_generation_prompt(format!(
            "Rewrite this thread title so it stays in Title Case, fits within {TITLE_TARGET_MIN_CHARS}-{TITLE_TARGET_MAX_CHARS} characters, and uses at most {TITLE_MAX_WORDS} words. Output ONLY the rewritten title.\nCurrent title: {title}\nOriginal request: {prompt_text}"
        ));
        title = sanitize_generated_title(
            &collect_model_text(
                &mut client_session,
                &repair_prompt,
                &model_info,
                &services.session_telemetry,
            )
            .await?,
        )?;
    }

    if !title_fits_constraints(&title) {
        warn!("thread title generator produced an invalid title: {title:?}");
        return None;
    }

    Some(title)
}

pub(crate) fn prompt_text_from_user_input(
    input: &[codex_protocol::user_input::UserInput],
) -> Option<String> {
    let parts = input
        .iter()
        .filter_map(|item| match item {
            codex_protocol::user_input::UserInput::Text { text, .. } => {
                let compacted = compact_title_prompt(text);
                (!compacted.is_empty()).then_some(compacted)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let joined = parts.join(" ");
    (!joined.is_empty()).then_some(joined)
}

fn build_generation_prompt(context: String) -> Prompt {
    Prompt {
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText { text: context }],
            end_turn: None,
            phase: None,
        }],
        tools: Vec::new(),
        parallel_tool_calls: false,
        base_instructions: BaseInstructions {
            text: title_generation_instructions(),
        },
        personality: None,
        output_schema: None,
    }
}

async fn collect_model_text(
    client_session: &mut ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    session_telemetry: &SessionTelemetry,
) -> Option<String> {
    let mut stream = match client_session
        .stream(
            prompt,
            model_info,
            session_telemetry,
            Some(ReasoningEffort::None),
            ReasoningSummary::None,
            None,
            None,
        )
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            warn!("thread title generation request failed: {err}");
            return None;
        }
    };

    let mut result = String::new();
    while let Some(message) = stream.next().await.transpose().ok()? {
        match message {
            crate::ResponseEvent::OutputTextDelta(delta) => result.push_str(&delta),
            crate::ResponseEvent::OutputItemDone(item) => {
                if result.is_empty()
                    && let ResponseItem::Message { content, .. } = item
                    && let Some(text) = crate::compact::content_items_to_text(&content)
                {
                    result.push_str(&text);
                }
            }
            crate::ResponseEvent::Completed { .. } => break,
            _ => {}
        }
    }
    if result.trim().is_empty() {
        debug!("thread title generation returned empty output");
        return None;
    }
    Some(result)
}

fn title_generation_instructions() -> String {
    format!(
        "Name this Codex thread in {TITLE_TARGET_MIN_CHARS}-{TITLE_TARGET_MAX_CHARS} characters. Title Case. Output ONLY the thread title.\nCapture the action and subject. Use at most {TITLE_MAX_WORDS} words. Never abbreviate words except: API, CLI, DB, TG, CI, UI.\n\n\"turn on the living room lights\" -> \"Turn On Lights\"\n\"Check emails about Eight Sleep\" -> \"8 Sleep Emails\"\n\"Update agents.md for SuperWhisper\" -> \"Whisper Vocab\"\n\"the light automations aren't working\" -> \"Fix Light Sched\"\n\"how should i configure tailscale\" -> \"Config Tailscale\"\n\"Ensure changes applied and /ship\" -> \"Ship All Changes\"\n\"why does opensea show generic icon\" -> \"Fix OpenSea Icon\"\n\"fix: Time Machine Backup Status\" -> \"Fix TM Backup\"\n\"Give me Amazon links\" -> \"Amazon Links\"\n\"connect tesla to iphone hotspot\" -> \"Tesla Hotspot\"\n\"abandon work on the play feature\" -> \"Revert Play Work\""
    )
}

fn select_title_model(models: &[ModelPreset]) -> Option<String> {
    for preferred in PREFERRED_TITLE_MODELS {
        if models.iter().any(|model| model.model == *preferred) {
            return Some((*preferred).to_string());
        }
    }
    if let Some(model) = models
        .iter()
        .find(|model| model.model.contains("mini") || model.model.contains("nano"))
    {
        return Some(model.model.clone());
    }
    models
        .iter()
        .find(|model| model.is_default)
        .or_else(|| models.first())
        .map(|model| model.model.clone())
}

fn title_fits_constraints(title: &str) -> bool {
    let len = title.chars().count();
    len >= 3 && len <= TITLE_TARGET_MAX_CHARS && title.split_whitespace().count() <= TITLE_MAX_WORDS
}

fn sanitize_generated_title(title: &str) -> Option<String> {
    let mut title = title.trim();
    for (start, end) in [("\"", "\""), ("'", "'"), ("`", "`")] {
        if title.starts_with(start) && title.ends_with(end) && title.len() > 2 {
            title = &title[start.len()..title.len() - end.len()];
        }
    }
    let title = title.trim().trim_end_matches('.').trim();
    normalize_thread_name(&collapse_whitespace(title))
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_title_prompt(prompt: &str) -> String {
    let mut prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return String::new();
    }

    let mut lower = prompt.to_lowercase();
    for marker in ["</environment_context>", "</instructions>"] {
        if let Some(idx) = lower.rfind(marker) {
            let tail = prompt[idx + marker.len()..].trim();
            if !tail.is_empty() {
                prompt = tail.to_string();
                lower = prompt.to_lowercase();
            }
        }
    }

    if lower.starts_with("# agents.md instructions for") {
        return String::new();
    }

    collapse_whitespace(&prompt)
}

fn is_informative_prompt(prompt: &str) -> bool {
    let normalized = compact_title_prompt(prompt).to_lowercase();
    if normalized.is_empty() || is_low_signal_prompt(&normalized) {
        return false;
    }

    match normalized.as_str() {
        "continue" | "resume" | "go on" | "next" | "ok" | "okay" | "k" | "yes" | "y" | "no"
        | "n" | "thanks" | "thank you" | "thx" | "again" => false,
        _ => normalized.chars().count() >= 12 && normalized.split_whitespace().count() >= 2,
    }
}

fn is_low_signal_prompt(prompt: &str) -> bool {
    let normalized = compact_title_prompt(prompt).to_lowercase();
    !normalized.is_empty()
        && (normalized.starts_with("<skill>")
            || normalized.starts_with("<environment_context>")
            || normalized.starts_with("<permissions instructions>")
            || normalized.starts_with("<turn_aborted>")
            || normalized.starts_with("<collaboration_mode>")
            || normalized.starts_with("<user_action>")
            || normalized.starts_with("<task_notification>")
            || normalized.starts_with("<subagent_notification>")
            || normalized.contains("```")
            || normalized.contains("generic agent job")
            || normalized.contains("the user interrupted the previous turn on purpose")
            || normalized.contains("user initiated a review task")
            || normalized.contains("<local-command-")
            || normalized.contains("<command-")
            || normalized.contains("<output>")
            || normalized.contains("<context>")
            || normalized.contains("<name>")
            || normalized.contains("<path>")
            || normalized.contains("set sandbox permissions"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::openai_models::ReasoningEffortPreset;

    fn preset(model: &str, is_default: bool) -> ModelPreset {
        ModelPreset {
            id: model.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            description: String::new(),
            default_reasoning_effort: ReasoningEffort::Low,
            supported_reasoning_efforts: vec![ReasoningEffortPreset {
                effort: ReasoningEffort::Low,
                description: "low".to_string(),
            }],
            supports_personality: false,
            is_default,
            upgrade: None,
            show_in_picker: true,
            availability_nux: None,
            supported_in_api: true,
            input_modalities: codex_protocol::openai_models::default_input_modalities(),
        }
    }

    #[test]
    fn compact_title_prompt_strips_agents_wrapper_prefix() {
        let raw = "# AGENTS.md instructions for /Users/brianle\n<INSTRUCTIONS>\nUse the rules.\n</INSTRUCTIONS>\n<environment_context>\n  <cwd>/Users/brianle</cwd>\n</environment_context>\ncreate a new ai subcommand for recalling sessions";
        assert_eq!(
            compact_title_prompt(raw),
            "create a new ai subcommand for recalling sessions"
        );
    }

    #[test]
    fn low_signal_prompt_detection_matches_wrapper_noise() {
        assert!(is_low_signal_prompt(
            "<user_action> <context>User initiated a review task"
        ));
        assert!(is_low_signal_prompt(
            "# Ship ```text ``` ## Goal Fix safe commit push flow"
        ));
        assert!(!is_low_signal_prompt(
            "openviking migration from yesterday in codex"
        ));
    }

    #[test]
    fn select_title_model_prefers_mini_candidates() {
        let models = vec![preset("gpt-5.4", true), preset("gpt-5.1-codex-mini", false)];
        assert_eq!(
            select_title_model(&models).as_deref(),
            Some("gpt-5.1-codex-mini")
        );
    }

    #[test]
    fn select_title_model_falls_back_to_default() {
        let models = vec![preset("gpt-5.4", true), preset("gpt-5.3-codex", false)];
        assert_eq!(select_title_model(&models).as_deref(), Some("gpt-5.4"));
    }

    #[test]
    fn sanitize_generated_title_strips_quotes_and_period() {
        assert_eq!(
            sanitize_generated_title("\"Fix Title Flow.\"").as_deref(),
            Some("Fix Title Flow")
        );
    }

    #[test]
    fn prompt_text_from_user_input_joins_text_items() {
        let input = vec![
            codex_protocol::user_input::UserInput::Text {
                text: "first".to_string(),
                text_elements: Vec::new(),
            },
            codex_protocol::user_input::UserInput::Image {
                image_url: "https://example.com".to_string(),
            },
            codex_protocol::user_input::UserInput::Text {
                text: "second".to_string(),
                text_elements: Vec::new(),
            },
        ];
        assert_eq!(
            prompt_text_from_user_input(&input).as_deref(),
            Some("first second")
        );
    }
}
