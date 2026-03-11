use codex_git::merge_base_with_head;
use codex_protocol::protocol::ReviewRequest;
use codex_protocol::protocol::ReviewTarget;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedReviewRequest {
    pub target: ReviewTarget,
    pub pathspecs: Vec<String>,
    pub prompt: String,
    pub user_facing_hint: String,
}

const UNCOMMITTED_PROMPT: &str = "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings.";

const BASE_BRANCH_PROMPT_BACKUP: &str = "Review the code changes against the base branch '{branch}'. Start by finding the merge diff between the current branch and {branch}'s upstream e.g. (`git merge-base HEAD \"$(git rev-parse --abbrev-ref \"{branch}@{upstream}\")\"`), then run `git diff` against that SHA to see what changes we would merge into the {branch} branch. Provide prioritized, actionable findings.";
const BASE_BRANCH_PROMPT: &str = "Review the code changes against the base branch '{baseBranch}'. The merge base commit for this comparison is {mergeBaseSha}. Run `git diff {mergeBaseSha}` to inspect the changes relative to {baseBranch}. Provide prioritized, actionable findings.";

const COMMIT_PROMPT_WITH_TITLE: &str = "Review the code changes introduced by commit {sha} (\"{title}\"). Provide prioritized, actionable findings.";
const COMMIT_PROMPT: &str =
    "Review the code changes introduced by commit {sha}. Provide prioritized, actionable findings.";

#[derive(Clone, Debug, PartialEq)]
struct PromptPathScope {
    display_paths: Vec<String>,
    rendered_args: String,
}

struct ReviewCommandSet {
    primary_command: String,
    secondary_command: Option<String>,
    untracked_command: Option<String>,
}

pub fn resolve_review_request(
    request: ReviewRequest,
    cwd: &Path,
) -> anyhow::Result<ResolvedReviewRequest> {
    let ReviewRequest {
        target,
        pathspecs,
        user_facing_hint,
    } = request;
    let normalized_pathspecs = normalize_hint_pathspecs(&pathspecs)?;
    let prompt = review_prompt_for_target(&target, &normalized_pathspecs, cwd)?;
    let user_facing_hint = user_facing_hint
        .unwrap_or_else(|| user_facing_hint_for_target(&target, &normalized_pathspecs));

    Ok(ResolvedReviewRequest {
        target,
        pathspecs: normalized_pathspecs,
        prompt,
        user_facing_hint,
    })
}

pub fn review_prompt(request: &ReviewRequest, cwd: &Path) -> anyhow::Result<String> {
    let normalized_pathspecs = normalize_hint_pathspecs(&request.pathspecs)?;
    review_prompt_for_target(&request.target, &normalized_pathspecs, cwd)
}

fn review_prompt_for_target(
    target: &ReviewTarget,
    pathspecs: &[String],
    cwd: &Path,
) -> anyhow::Result<String> {
    let scope = render_path_scope(pathspecs, cwd)?;

    match target {
        ReviewTarget::UncommittedChanges => Ok(append_path_scope_guidance(
            UNCOMMITTED_PROMPT.to_string(),
            scope.as_ref(),
            Some(ReviewCommandSet {
                primary_command: "git diff --".to_string(),
                secondary_command: Some("git diff --staged --".to_string()),
                untracked_command: Some("git ls-files --others --exclude-standard --".to_string()),
            }),
        )),
        ReviewTarget::BaseBranch { branch } => {
            let base_prompt = if let Some(commit) = merge_base_with_head(cwd, branch)? {
                let prompt = BASE_BRANCH_PROMPT
                    .replace("{baseBranch}", branch)
                    .replace("{mergeBaseSha}", &commit);
                let commands = scope.as_ref().map(|_| ReviewCommandSet {
                    primary_command: format!("git diff {commit} --"),
                    secondary_command: None,
                    untracked_command: None,
                });
                append_path_scope_guidance(prompt, scope.as_ref(), commands)
            } else {
                append_path_scope_guidance(
                    BASE_BRANCH_PROMPT_BACKUP.replace("{branch}", branch),
                    scope.as_ref(),
                    None,
                )
            };
            Ok(base_prompt)
        }
        ReviewTarget::Commit { sha, title } => {
            let prompt = if let Some(title) = title {
                COMMIT_PROMPT_WITH_TITLE
                    .replace("{sha}", sha)
                    .replace("{title}", title)
            } else {
                COMMIT_PROMPT.replace("{sha}", sha)
            };
            Ok(append_path_scope_guidance(
                prompt,
                scope.as_ref(),
                Some(ReviewCommandSet {
                    primary_command: format!("git show {sha} --"),
                    secondary_command: None,
                    untracked_command: None,
                }),
            ))
        }
        ReviewTarget::Custom { instructions } => {
            let prompt = instructions.trim();
            if prompt.is_empty() {
                anyhow::bail!("Review prompt cannot be empty");
            }
            Ok(append_custom_path_scope_guidance(
                prompt.to_string(),
                scope.as_ref(),
            ))
        }
    }
}

pub fn user_facing_hint(target: &ReviewTarget) -> String {
    user_facing_hint_for_target(target, &[])
}

pub fn user_facing_hint_for_request(request: &ReviewRequest) -> anyhow::Result<String> {
    let pathspecs = normalize_hint_pathspecs(&request.pathspecs)?;
    Ok(user_facing_hint_for_target(&request.target, &pathspecs))
}

fn user_facing_hint_for_target(target: &ReviewTarget, pathspecs: &[String]) -> String {
    let base = match target {
        ReviewTarget::UncommittedChanges => "current changes".to_string(),
        ReviewTarget::BaseBranch { branch } => format!("changes against '{branch}'"),
        ReviewTarget::Commit { sha, title } => {
            let short_sha: String = sha.chars().take(7).collect();
            if let Some(title) = title {
                format!("commit {short_sha}: {title}")
            } else {
                format!("commit {short_sha}")
            }
        }
        ReviewTarget::Custom { instructions } => instructions.trim().to_string(),
    };

    match summarize_pathspecs(pathspecs) {
        Some(summary) => format!("{base} in {summary}"),
        None => base,
    }
}

fn normalize_hint_pathspecs(pathspecs: &[String]) -> anyhow::Result<Vec<String>> {
    let mut normalized = Vec::with_capacity(pathspecs.len());
    for raw in pathspecs {
        let pathspec = raw.trim();
        if pathspec.is_empty() {
            anyhow::bail!("Review pathspecs must not be empty");
        }
        let normalized_pathspec = normalize_hint_pathspec(pathspec);
        if normalized
            .iter()
            .all(|existing| existing != &normalized_pathspec)
        {
            normalized.push(normalized_pathspec);
        }
    }
    Ok(normalized)
}

fn normalize_hint_pathspec(pathspec: &str) -> String {
    let mut normalized = pathspec;
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped;
    }
    if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized.to_string()
    }
}

fn render_path_scope(pathspecs: &[String], cwd: &Path) -> anyhow::Result<Option<PromptPathScope>> {
    if pathspecs.is_empty() {
        return Ok(None);
    }

    let repo_root = review_repo_root(cwd)?;
    let cwd_prefix = cwd_prefix_within_repo(cwd, &repo_root)?;
    let mut display_paths = Vec::with_capacity(pathspecs.len());

    for pathspec in pathspecs {
        let relative = resolve_review_pathspec(pathspec, &repo_root, &cwd_prefix)?;
        let display = display_repo_relative_path(&relative);
        if display_paths.iter().all(|existing| existing != &display) {
            display_paths.push(display);
        }
    }

    let rendered_args = render_pathspec_args(&display_paths)?;
    Ok(Some(PromptPathScope {
        display_paths,
        rendered_args,
    }))
}

fn review_repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let base_dir = if cwd.is_dir() {
        cwd
    } else {
        cwd.parent().unwrap_or(cwd)
    };

    let output = Command::new("git")
        .current_dir(base_dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| anyhow::anyhow!("Failed to resolve review repository root: {error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "Failed to resolve review repository root: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| anyhow::anyhow!("Failed to decode repository root output: {error}"))?;
    Ok(PathBuf::from(stdout.trim()))
}

fn cwd_prefix_within_repo(cwd: &Path, repo_root: &Path) -> anyhow::Result<PathBuf> {
    let base_dir = if cwd.is_dir() {
        cwd
    } else {
        cwd.parent().unwrap_or(cwd)
    };

    if let Ok(relative) = base_dir.strip_prefix(repo_root) {
        return Ok(relative.to_path_buf());
    }

    let cwd_canon = std::fs::canonicalize(base_dir)
        .map_err(|error| anyhow::anyhow!("Failed to canonicalize review cwd: {error}"))?;
    let repo_root_canon = std::fs::canonicalize(repo_root)
        .map_err(|error| anyhow::anyhow!("Failed to canonicalize repository root: {error}"))?;

    cwd_canon
        .strip_prefix(&repo_root_canon)
        .map(Path::to_path_buf)
        .map_err(|_| anyhow::anyhow!("Review cwd is outside the repository root"))
}

fn resolve_review_pathspec(
    pathspec: &str,
    repo_root: &Path,
    cwd_prefix: &Path,
) -> anyhow::Result<PathBuf> {
    let raw_path = Path::new(pathspec);
    if raw_path.is_absolute() {
        let relative = raw_path.strip_prefix(repo_root).map_err(|_| {
            anyhow::anyhow!(
                "Review pathspec '{}' must be inside the current repository",
                raw_path.display()
            )
        })?;
        normalize_repo_relative_path(relative)
    } else {
        normalize_repo_relative_path(&cwd_prefix.join(raw_path))
    }
}

fn normalize_repo_relative_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    anyhow::bail!(
                        "Review pathspec '{}' escapes the repository root",
                        path.display()
                    );
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "Review pathspec '{}' must be relative to the repository root",
                    path.display()
                );
            }
        }
    }

    Ok(normalized)
}

fn display_repo_relative_path(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        ".".to_string()
    } else {
        path.components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn render_pathspec_args(display_paths: &[String]) -> anyhow::Result<String> {
    display_paths
        .iter()
        .map(|path| {
            let literal = if path == "." {
                ":(top,glob)**".to_string()
            } else {
                format!(":(top,literal){path}")
            };
            shlex::try_join([literal.as_str()])
                .map_err(|_| anyhow::anyhow!("Review pathspecs must not contain NUL bytes"))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map(|parts| parts.join(" "))
}

fn append_path_scope_guidance(
    base_prompt: String,
    scope: Option<&PromptPathScope>,
    commands: Option<ReviewCommandSet>,
) -> String {
    let Some(scope) = scope else {
        return base_prompt;
    };

    let rendered_paths = scope
        .display_paths
        .iter()
        .map(|path| format!("- {path}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut guidance = format!(
        "\n\nLimit the review to these paths:\n{rendered_paths}\n\nTreat them as literal file filters. When inspecting changes, append {} after the revision arguments.",
        markdown_inline_code(&scope.rendered_args),
    );

    if let Some(commands) = commands {
        guidance.push_str(&format!(
            " Start with {}.",
            markdown_inline_code(&format!(
                "{} {}",
                commands.primary_command, scope.rendered_args
            )),
        ));
        if let Some(secondary_command) = commands.secondary_command {
            guidance.push_str(&format!(
                " Also inspect {}.",
                markdown_inline_code(&format!("{} {}", secondary_command, scope.rendered_args)),
            ));
        }
        if let Some(untracked_command) = commands.untracked_command {
            guidance.push_str(&format!(
                " Check in-scope untracked files with {}.",
                markdown_inline_code(&format!("{} {}", untracked_command, scope.rendered_args)),
            ));
        }
    } else {
        guidance.push_str(
            " After identifying the comparison commit, keep any diff inspection restricted to those paths.",
        );
    }

    guidance
        .push_str(" Ignore unrelated files unless they are required to explain an in-scope issue.");
    format!("{base_prompt}{guidance}")
}

fn append_custom_path_scope_guidance(
    base_prompt: String,
    scope: Option<&PromptPathScope>,
) -> String {
    let Some(scope) = scope else {
        return base_prompt;
    };

    let rendered_paths = scope
        .display_paths
        .iter()
        .map(|path| format!("- {path}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "{base_prompt}\n\nLimit the review to these paths:\n{rendered_paths}\n\nTreat them as literal file filters, restrict any git or file inspection to {} and ignore unrelated files unless they are required to explain an in-scope issue.",
        markdown_inline_code(&scope.rendered_args),
    )
}

fn markdown_inline_code(text: &str) -> String {
    let max_backticks = text
        .chars()
        .fold((0usize, 0usize), |(max_run, current_run), ch| {
            if ch == '`' {
                let next_run = current_run + 1;
                (max_run.max(next_run), next_run)
            } else {
                (max_run, 0)
            }
        })
        .0;
    let fence = "`".repeat(max_backticks + 1);
    format!("{fence}{text}{fence}")
}

fn summarize_pathspecs(pathspecs: &[String]) -> Option<String> {
    match pathspecs {
        [] => None,
        [only] => Some(only.clone()),
        [first, second] => Some(format!("{first} and {second}")),
        [first, second, third] => Some(format!("{first}, {second}, and {third}")),
        [first, ..] => Some(format!("{first} and {} other paths", pathspecs.len() - 1)),
    }
}

impl From<ResolvedReviewRequest> for ReviewRequest {
    fn from(resolved: ResolvedReviewRequest) -> Self {
        ReviewRequest {
            target: resolved.target,
            pathspecs: resolved.pathspecs,
            user_facing_hint: Some(resolved.user_facing_hint),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn init_git_repo(path: &Path) {
        let output = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .output()
            .expect("spawn git init");
        assert!(
            output.status.success(),
            "git init failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn resolve_review_request_scopes_paths_from_nested_cwd() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path();
        init_git_repo(repo);
        std::fs::create_dir_all(repo.join("src/nested")).expect("create repo dirs");

        let request = ReviewRequest {
            target: ReviewTarget::UncommittedChanges,
            pathspecs: vec!["./file.rs".to_string(), "../sibling.rs".to_string()],
            user_facing_hint: None,
        };

        let resolved = resolve_review_request(request, &repo.join("src/nested"))
            .expect("resolve review request");

        assert_eq!(
            resolved.pathspecs,
            vec!["file.rs".to_string(), "../sibling.rs".to_string()]
        );
        assert_eq!(
            resolved.user_facing_hint,
            "current changes in file.rs and ../sibling.rs"
        );
        assert!(resolved.prompt.contains(":(top,literal)src/nested/file.rs"));
        assert!(resolved.prompt.contains(":(top,literal)src/sibling.rs"));
    }

    #[test]
    fn review_prompt_renders_repo_root_scope_for_dot() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path();
        init_git_repo(repo);

        let request = ReviewRequest {
            target: ReviewTarget::UncommittedChanges,
            pathspecs: vec!["./".to_string()],
            user_facing_hint: None,
        };

        let prompt = review_prompt(&request, repo).expect("build review prompt");

        assert!(prompt.contains(":(top,glob)**"));
        assert!(prompt.contains("Limit the review to these paths:\n- ."));
    }

    #[test]
    fn review_prompt_rejects_blank_pathspec() {
        let request = ReviewRequest {
            target: ReviewTarget::UncommittedChanges,
            pathspecs: vec!["   ".to_string()],
            user_facing_hint: None,
        };

        let err = review_prompt(&request, Path::new(".")).expect_err("blank pathspec should fail");

        assert_eq!(err.to_string(), "Review pathspecs must not be empty");
    }

    #[test]
    fn review_prompt_rejects_path_outside_repo() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path();
        init_git_repo(repo);
        std::fs::create_dir_all(repo.join("src/nested")).expect("create repo dirs");

        let request = ReviewRequest {
            target: ReviewTarget::UncommittedChanges,
            pathspecs: vec!["../../../elsewhere".to_string()],
            user_facing_hint: None,
        };

        let err = review_prompt(&request, &repo.join("src/nested"))
            .expect_err("path outside repo should fail");

        assert!(err.to_string().contains("escapes the repository root"));
    }
}
