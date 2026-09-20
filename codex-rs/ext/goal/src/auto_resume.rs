use codex_protocol::protocol::ThreadGoalStatus;

/// Identifies the source of input that may make a stopped goal runnable again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutoResumeSource {
    User,
    Background,
}

pub(crate) fn should_auto_resume(
    status: ThreadGoalStatus,
    objective: &str,
    text: &str,
    source: AutoResumeSource,
) -> bool {
    if !matches!(status, ThreadGoalStatus::Paused | ThreadGoalStatus::Blocked) {
        return false;
    }

    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() || is_status_only(&text) || contains_stop_request(&text) {
        return false;
    }

    match source {
        AutoResumeSource::User => {
            contains_generic_continuation_signal(&text)
                || (contains_action_signal(&text) && overlaps_objective(objective, &text))
        }
        AutoResumeSource::Background => {
            contains_completion_signal(&text)
                && !contains_failure_signal(&text)
                && overlaps_objective(objective, &text)
        }
    }
}

fn is_status_only(text: &str) -> bool {
    [
        "status",
        "status?",
        "what's the status",
        "what is the status",
        "any update",
        "any updates",
        "where are we",
        "how is it going",
    ]
    .iter()
    .any(|prefix| text == *prefix || text.starts_with(&format!("{prefix} ")))
}

fn contains_stop_request(text: &str) -> bool {
    [
        "pause",
        "stop",
        "hold off",
        "don't continue",
        "do not continue",
    ]
    .iter()
    .any(|signal| text.contains(signal))
}

fn contains_generic_continuation_signal(text: &str) -> bool {
    [
        "continue",
        "proceed",
        "resume",
        "retry",
        "try again",
        "keep going",
        "pick up",
        "go ahead",
    ]
    .iter()
    .any(|signal| text.contains(signal))
}

fn contains_action_signal(text: &str) -> bool {
    [
        "implement",
        "apply",
        "fix",
        "land",
        "deploy",
        "publish",
        "merge",
    ]
    .iter()
    .any(|signal| text.contains(signal))
}

fn overlaps_objective(objective: &str, text: &str) -> bool {
    let objective_words = objective
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() >= 5)
        .map(str::to_ascii_lowercase)
        .filter(|word| !matches!(word.as_str(), "about" | "after" | "before" | "should"))
        .collect::<Vec<_>>();
    let text = text.to_ascii_lowercase();
    objective_words.iter().any(|word| {
        text.split(|character: char| !character.is_alphanumeric())
            .any(|token| token == word)
    })
}

fn contains_completion_signal(text: &str) -> bool {
    [
        "completed",
        "complete",
        "succeeded",
        "success",
        "ready",
        "resolved",
        "fixed",
        "available",
        "approved",
        "unblocked",
        "finished",
        "done",
    ]
    .iter()
    .any(|signal| text.contains(signal))
}

fn contains_failure_signal(text: &str) -> bool {
    [
        "failed",
        "failure",
        "error",
        "blocked",
        "unavailable",
        "timed out",
        "timeout",
    ]
    .iter()
    .any(|signal| text.contains(signal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_continuation_resumes_stopped_goals() {
        assert!(should_auto_resume(
            ThreadGoalStatus::Paused,
            "migrate the backup repository",
            "Proceed with the migration",
            AutoResumeSource::User
        ));
        assert!(should_auto_resume(
            ThreadGoalStatus::Blocked,
            "migrate the backup repository",
            "The credentials are fixed, continue",
            AutoResumeSource::User
        ));
    }

    #[test]
    fn status_and_stop_messages_do_not_resume() {
        for text in ["What is the status?", "pause this", "stop for now"] {
            assert!(!should_auto_resume(
                ThreadGoalStatus::Paused,
                "migrate the backup repository",
                text,
                AutoResumeSource::User
            ));
        }
    }

    #[test]
    fn background_completion_resumes_but_failure_does_not() {
        assert!(should_auto_resume(
            ThreadGoalStatus::Blocked,
            "migrate the backup repository",
            "Worker completed the backup repository migration and the credential is available",
            AutoResumeSource::Background
        ));
        assert!(!should_auto_resume(
            ThreadGoalStatus::Blocked,
            "migrate the backup repository",
            "Worker failed with a timeout",
            AutoResumeSource::Background
        ));
        assert!(!should_auto_resume(
            ThreadGoalStatus::Paused,
            "migrate the backup repository",
            "Fix the unrelated website",
            AutoResumeSource::User
        ));
    }

    #[test]
    fn limits_and_completion_are_never_auto_resumed() {
        for status in [
            ThreadGoalStatus::Active,
            ThreadGoalStatus::UsageLimited,
            ThreadGoalStatus::BudgetLimited,
            ThreadGoalStatus::Complete,
        ] {
            assert!(!should_auto_resume(
                status,
                "migrate the backup repository",
                "continue",
                AutoResumeSource::User
            ));
        }
    }
}
