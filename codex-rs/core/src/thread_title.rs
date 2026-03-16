use std::path::Path;

use crate::ModelClientSession;
use crate::client_common::Prompt;
use crate::config::Config;
use crate::models_manager::manager::RefreshStrategy;
use crate::state::SessionServices;
use codex_otel::SessionTelemetry;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tracing::debug;
use tracing::warn;

const TITLE_MIN_WORDS: usize = 3;
const TITLE_MAX_WORDS: usize = 7;
const CLAUDE_TITLE_PROMPT: &str = r#"Generate a very short, sentence-case title (3-7 words) that captures the main topic or goal of this coding session.
Guidelines:
- Users often only see the first 12 visible characters. Put the gist there first.
- The first 12 visible characters should usually be enough to recognize the session.
- Target 24 visible characters or fewer, including spaces.
- Prefer 3-4 words when possible.
- Put the main noun and verb first.
- If a clear title would be too long, compress aggressively and drop filler words.
- Do not bury the distinguishing topic near the end.
- Use sentence case: capitalize only the first word and proper nouns.
Return JSON with a single "title" field.
Good examples:
{"title": "Fix login bug"}
{"title": "Cmux keybind fix"}
{"title": "Swift title audit"}
{"title": "CI failure debug"}
Bad (too vague): {"title": "Code changes"}
Bad (gist too late): {"title": "Investigate cmux keybinds"}
Bad (too long): {"title": "Investigate workspace keybinds across tools"}
Bad (wrong case): {"title": "Fix Login Bug"}"#;
const CLAUDE_RETITLE_PROMPT: &str = r#"Given the current title for a coding session and the latest meaningful user request, decide whether the title should change.
Return JSON with exactly 2 fields:
{"should_update": false, "title": "Current title"}
or
{"should_update": true, "title": "New sentence-case title"}

Rules:
- Update only for clear topic changes.
- Do not update for wording tweaks, follow-up clarifications, or temporary subtasks.
- Do not update when the current title still accurately describes the session.
- If should_update is false, return the unchanged current title.
- If should_update is true, return a concise sentence-case title in 3-7 words.
- Users often only see the first 12 visible characters. Put the gist there first.
- The first 12 visible characters should usually be enough to recognize the session.
- Target 24 visible characters or fewer, including spaces.
- Prefer 3-4 words when possible.
- Put the main noun and verb first.
- If needed, compress aggressively and drop filler words.
- Do not bury the distinguishing topic near the end.
"#;

#[derive(Debug, Deserialize)]
struct GeneratedTitleResponse {
    title: String,
}

#[derive(Debug, Deserialize)]
struct RetitleDecisionResponse {
    should_update: bool,
    title: String,
}

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
    let model = resolve_title_model(config, models.as_slice())?;
    let model_info = services
        .models_manager
        .get_model_info(model.as_str(), config)
        .await;
    let reasoning_effort = resolve_title_reasoning_effort(config, &model_info);

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
        reasoning_effort,
        &services.session_telemetry,
    )
    .await?;

    title = parse_generated_title(&title)?;

    if !title_fits_constraints(&title) {
        warn!("thread title generator produced an invalid title: {title:?}");
        return None;
    }

    Some(title)
}

pub(crate) async fn maybe_retitle_thread(
    services: &SessionServices,
    config: &Config,
    cwd: &Path,
    current_title: &str,
    prompt_text: &str,
) -> Option<String> {
    let current_title = sanitize_generated_title(current_title)?;
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
    let model = resolve_title_model(config, models.as_slice())?;
    let model_info = services
        .models_manager
        .get_model_info(model.as_str(), config)
        .await;
    let reasoning_effort = resolve_title_reasoning_effort(config, &model_info);

    let prompt = build_retitle_prompt(format!(
        "Working directory: {}\nCurrent session title: {}\nLatest user request: {}",
        cwd.display(),
        current_title,
        prompt_text
    ));
    let mut client_session = services.model_client.new_session();
    let result = collect_model_text(
        &mut client_session,
        &prompt,
        &model_info,
        reasoning_effort,
        &services.session_telemetry,
    )
    .await?;

    let decision = parse_retitle_decision(&result, &current_title)?;
    if !decision.should_update {
        return None;
    }

    let title = sanitize_generated_title(&decision.title)?;
    if !title_fits_constraints(&title) {
        warn!("thread retitle generator produced an invalid title: {title:?}");
        return None;
    }
    if same_title_identity(&title, &current_title) {
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
    build_title_prompt(
        context,
        title_generation_instructions(),
        title_output_schema(),
    )
}

fn build_retitle_prompt(context: String) -> Prompt {
    build_title_prompt(context, retitle_instructions(), retitle_output_schema())
}

fn build_title_prompt(
    context: String,
    instructions: String,
    output_schema: serde_json::Value,
) -> Prompt {
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
        base_instructions: BaseInstructions { text: instructions },
        personality: None,
        output_schema: Some(output_schema),
    }
}

async fn collect_model_text(
    client_session: &mut ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    reasoning_effort: Option<ReasoningEffort>,
    session_telemetry: &SessionTelemetry,
) -> Option<String> {
    let mut stream = match client_session
        .stream(
            prompt,
            model_info,
            session_telemetry,
            reasoning_effort,
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
    CLAUDE_TITLE_PROMPT.to_string()
}

fn retitle_instructions() -> String {
    CLAUDE_RETITLE_PROMPT.to_string()
}

fn title_output_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title"],
        "properties": {
            "title": {
                "type": "string"
            }
        }
    })
}

fn retitle_output_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["should_update", "title"],
        "properties": {
            "should_update": {
                "type": "boolean"
            },
            "title": {
                "type": "string"
            }
        }
    })
}

fn resolve_title_model(config: &Config, models: &[ModelPreset]) -> Option<String> {
    if let Some(model) = config
        .thread_title_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Some(model.to_string());
    }
    if let Some(model) = config
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Some(model.to_string());
    }
    models
        .iter()
        .find(|model| model.is_default)
        .or_else(|| models.first())
        .map(|model| model.model.clone())
}

fn resolve_title_reasoning_effort(
    config: &Config,
    model_info: &ModelInfo,
) -> Option<ReasoningEffort> {
    config.thread_title_reasoning_effort.or_else(|| {
        model_info
            .supported_reasoning_levels
            .iter()
            .any(|preset| preset.effort == ReasoningEffort::Low)
            .then_some(ReasoningEffort::Low)
            .or(model_info.default_reasoning_level)
    })
}

fn title_fits_constraints(title: &str) -> bool {
    let word_count = title.split_whitespace().count();
    let len = title.chars().count();
    len >= 3 && (TITLE_MIN_WORDS..=TITLE_MAX_WORDS).contains(&word_count)
}

fn parse_generated_title(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let parsed = parse_title_response(raw)
        .or_else(|| {
            let start = raw.find('{')?;
            let end = raw.rfind('}')?;
            parse_title_response(&raw[start..=end])
        })
        .unwrap_or_else(|| raw.to_string());
    sanitize_generated_title(&parsed)
}

fn parse_title_response(raw: &str) -> Option<String> {
    serde_json::from_str::<GeneratedTitleResponse>(raw)
        .ok()
        .map(|response| response.title)
}

fn parse_retitle_decision(raw: &str, current_title: &str) -> Option<RetitleDecisionResponse> {
    let raw = raw.trim();
    parse_retitle_response(raw)
        .or_else(|| {
            let start = raw.find('{')?;
            let end = raw.rfind('}')?;
            parse_retitle_response(&raw[start..=end])
        })
        .or_else(|| {
            let title = parse_generated_title(raw)?;
            Some(RetitleDecisionResponse {
                should_update: !same_title_identity(&title, current_title),
                title,
            })
        })
}

fn parse_retitle_response(raw: &str) -> Option<RetitleDecisionResponse> {
    serde_json::from_str::<RetitleDecisionResponse>(raw).ok()
}

fn sanitize_generated_title(title: &str) -> Option<String> {
    let mut title = title.trim();
    for (start, end) in [("\"", "\""), ("'", "'"), ("`", "`")] {
        if title.starts_with(start) && title.ends_with(end) && title.len() > 2 {
            title = &title[start.len()..title.len() - end.len()];
        }
    }
    let title = title.trim().trim_end_matches('.').trim();
    let title = collapse_whitespace(title);
    (!title.is_empty()).then_some(title)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn same_title_identity(left: &str, right: &str) -> bool {
    collapse_whitespace(left).eq_ignore_ascii_case(collapse_whitespace(right).as_str())
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
    use crate::config::test_config;
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

    fn model_info(
        default_reasoning_level: Option<ReasoningEffort>,
        supported_reasoning_levels: Vec<ReasoningEffortPreset>,
    ) -> ModelInfo {
        ModelInfo {
            slug: "gpt-5.4".to_string(),
            display_name: "gpt-5.4".to_string(),
            description: None,
            default_reasoning_level,
            supported_reasoning_levels,
            shell_type: codex_protocol::openai_models::ConfigShellToolType::Default,
            visibility: codex_protocol::openai_models::ModelVisibility::List,
            supported_in_api: true,
            priority: 0,
            availability_nux: None,
            upgrade: None,
            base_instructions: String::new(),
            model_messages: None,
            supports_reasoning_summaries: false,
            default_reasoning_summary: codex_protocol::config_types::ReasoningSummary::None,
            support_verbosity: false,
            default_verbosity: None,
            apply_patch_tool_type: None,
            web_search_tool_type: codex_protocol::openai_models::WebSearchToolType::Text,
            truncation_policy: codex_protocol::openai_models::TruncationPolicyConfig::bytes(20_000),
            supports_parallel_tool_calls: true,
            supports_image_detail_original: false,
            context_window: None,
            auto_compact_token_limit: None,
            effective_context_window_percent: 95,
            experimental_supported_tools: Vec::new(),
            input_modalities: codex_protocol::openai_models::default_input_modalities(),
            prefer_websockets: false,
            used_fallback_model_metadata: false,
            supports_search_tool: false,
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
    fn resolve_title_model_prefers_thread_title_model_override() {
        let mut config = test_config();
        config.model = Some("gpt-5.1".to_string());
        config.thread_title_model = Some("gpt-5.4".to_string());
        let models = vec![preset("gpt-5.1-codex-mini", true)];
        assert_eq!(
            resolve_title_model(&config, &models).as_deref(),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn resolve_title_model_falls_back_to_session_model() {
        let mut config = test_config();
        config.model = Some("gpt-5.3-codex".to_string());
        let models = vec![preset("gpt-5.4", true), preset("gpt-5.3-codex", false)];
        assert_eq!(
            resolve_title_model(&config, &models).as_deref(),
            Some("gpt-5.3-codex")
        );
    }

    #[test]
    fn resolve_title_model_falls_back_to_default() {
        let config = test_config();
        let models = vec![preset("gpt-5.4", true), preset("gpt-5.3-codex", false)];
        assert_eq!(
            resolve_title_model(&config, &models).as_deref(),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn resolve_title_reasoning_effort_prefers_configured_override() {
        let mut config = test_config();
        config.thread_title_reasoning_effort = Some(ReasoningEffort::Minimal);
        let model_info = model_info(
            Some(ReasoningEffort::High),
            vec![
                ReasoningEffortPreset {
                    effort: ReasoningEffort::Low,
                    description: "low".to_string(),
                },
                ReasoningEffortPreset {
                    effort: ReasoningEffort::High,
                    description: "high".to_string(),
                },
            ],
        );

        assert_eq!(
            resolve_title_reasoning_effort(&config, &model_info),
            Some(ReasoningEffort::Minimal)
        );
    }

    #[test]
    fn resolve_title_reasoning_effort_prefers_low_when_supported() {
        let config = test_config();
        let model_info = model_info(
            Some(ReasoningEffort::High),
            vec![
                ReasoningEffortPreset {
                    effort: ReasoningEffort::Low,
                    description: "low".to_string(),
                },
                ReasoningEffortPreset {
                    effort: ReasoningEffort::High,
                    description: "high".to_string(),
                },
            ],
        );

        assert_eq!(
            resolve_title_reasoning_effort(&config, &model_info),
            Some(ReasoningEffort::Low)
        );
    }

    #[test]
    fn resolve_title_reasoning_effort_falls_back_to_model_default() {
        let config = test_config();
        let model_info = model_info(
            Some(ReasoningEffort::High),
            vec![ReasoningEffortPreset {
                effort: ReasoningEffort::High,
                description: "high".to_string(),
            }],
        );

        assert_eq!(
            resolve_title_reasoning_effort(&config, &model_info),
            Some(ReasoningEffort::High)
        );
    }

    #[test]
    fn parse_generated_title_extracts_json_payload() {
        assert_eq!(
            parse_generated_title("{\"title\":\"Fix login button on mobile\"}").as_deref(),
            Some("Fix login button on mobile")
        );
    }

    #[test]
    fn sanitize_generated_title_strips_quotes_and_period() {
        assert_eq!(
            sanitize_generated_title("\"Fix login button on mobile.\"").as_deref(),
            Some("Fix login button on mobile")
        );
    }

    #[test]
    fn sanitize_generated_title_preserves_sentence_case() {
        assert_eq!(
            sanitize_generated_title("Fix login button on mobile").as_deref(),
            Some("Fix login button on mobile")
        );
    }

    #[test]
    fn title_generation_instructions_match_claude_reverse_engineered_prompt() {
        assert_eq!(
            title_generation_instructions(),
            r#"Generate a very short, sentence-case title (3-7 words) that captures the main topic or goal of this coding session.
Guidelines:
- Users often only see the first 12 visible characters. Put the gist there first.
- The first 12 visible characters should usually be enough to recognize the session.
- Target 24 visible characters or fewer, including spaces.
- Prefer 3-4 words when possible.
- Put the main noun and verb first.
- If a clear title would be too long, compress aggressively and drop filler words.
- Do not bury the distinguishing topic near the end.
- Use sentence case: capitalize only the first word and proper nouns.
Return JSON with a single "title" field.
Good examples:
{"title": "Fix login bug"}
{"title": "Cmux keybind fix"}
{"title": "Swift title audit"}
{"title": "CI failure debug"}
Bad (too vague): {"title": "Code changes"}
Bad (gist too late): {"title": "Investigate cmux keybinds"}
Bad (too long): {"title": "Investigate workspace keybinds across tools"}
Bad (wrong case): {"title": "Fix Login Bug"}"#
        );
    }

    #[test]
    fn retitle_instructions_front_load_the_gist() {
        let instructions = retitle_instructions();
        assert!(instructions.contains("first 12 visible characters"));
        assert!(instructions.contains("24 visible characters or fewer, including spaces"));
        assert!(instructions.contains("Put the main noun and verb first."));
        assert!(instructions.contains("Do not bury the distinguishing topic near the end."));
    }

    #[test]
    fn parse_retitle_decision_extracts_json_payload() {
        let decision = parse_retitle_decision(
            "{\"should_update\":true,\"title\":\"Fix login button on mobile\"}",
            "Current title",
        )
        .expect("expected decision");
        assert!(decision.should_update);
        assert_eq!(decision.title, "Fix login button on mobile");
    }

    #[test]
    fn parse_retitle_decision_falls_back_to_same_title_without_update() {
        let decision =
            parse_retitle_decision("Fix login button on mobile", "Fix login button on mobile")
                .expect("expected decision");
        assert!(!decision.should_update);
        assert_eq!(decision.title, "Fix login button on mobile");
    }

    #[test]
    fn same_title_identity_ignores_case_and_spacing() {
        assert!(same_title_identity(
            "Fix login button on mobile",
            "  fix   login button on mobile  "
        ));
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
