use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::GitToolingError;
use crate::operations::ensure_git_repository;
use crate::operations::repo_subdir;
use crate::operations::resolve_head;
use crate::operations::resolve_repository_root;
use crate::operations::run_git_for_status;
use crate::operations::run_git_for_stdout;

#[derive(Debug)]
pub struct StagedReviewSnapshot {
    _tempdir: TempDir,
    repo_root: PathBuf,
    worktree_path: PathBuf,
    cwd: PathBuf,
}

impl StagedReviewSnapshot {
    pub fn new(repo_path: &Path) -> Result<Self, GitToolingError> {
        ensure_git_repository(repo_path)?;

        let repo_root = resolve_repository_root(repo_path)?;
        let repo_prefix = repo_subdir(repo_root.as_path(), repo_path);
        let source_index = git_path(repo_root.as_path(), repo_root.as_path(), "index")?;
        let source_head = resolve_head(repo_root.as_path())?;

        let tempdir = tempfile::Builder::new()
            .prefix("codex-staged-review-")
            .tempdir()?;
        let worktree_path = tempdir.path().join("snapshot");

        let Some(source_head) = source_head else {
            return Err(GitToolingError::NoHeadCommit {
                path: repo_root.clone(),
            });
        };

        run_git_for_status(
            repo_root.as_path(),
            vec![
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("--detach"),
                OsString::from("--no-checkout"),
                worktree_path.as_os_str().to_os_string(),
                OsString::from(source_head),
            ],
            None,
        )?;

        if let Err(error) = populate_staged_worktree(&worktree_path, &source_index) {
            remove_worktree(repo_root.as_path(), &worktree_path);
            return Err(error);
        }

        let cwd = match repo_prefix {
            Some(prefix) => {
                let cwd = worktree_path.join(prefix);
                if !cwd.exists() {
                    std::fs::create_dir_all(&cwd)?;
                }
                cwd
            }
            None => worktree_path.clone(),
        };

        Ok(Self {
            _tempdir: tempdir,
            repo_root,
            worktree_path,
            cwd,
        })
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

impl Drop for StagedReviewSnapshot {
    fn drop(&mut self) {
        remove_worktree(&self.repo_root, &self.worktree_path);
    }
}

fn populate_staged_worktree(
    worktree_path: &Path,
    source_index: &Path,
) -> Result<(), GitToolingError> {
    let snapshot_index = git_path(worktree_path, worktree_path, "index")?;
    if let Some(parent) = snapshot_index.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source_index, &snapshot_index)?;

    run_git_for_status(
        worktree_path,
        vec![
            OsString::from("checkout-index"),
            OsString::from("--all"),
            OsString::from("--force"),
        ],
        None,
    )?;
    Ok(())
}

fn git_path(run_dir: &Path, base_dir: &Path, git_path: &str) -> Result<PathBuf, GitToolingError> {
    let raw = run_git_for_stdout(
        run_dir,
        vec![
            OsString::from("rev-parse"),
            OsString::from("--git-path"),
            OsString::from(git_path),
        ],
        None,
    )?;
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(base_dir.join(path))
    }
}

fn remove_worktree(repo_root: &Path, worktree_path: &Path) {
    let _ = std::process::Command::new("git")
        .current_dir(repo_root)
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_string_lossy().as_ref(),
        ])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(cwd: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8 stdout")
    }

    #[test]
    fn staged_review_snapshot_materializes_index_contents() -> Result<(), GitToolingError> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path();

        run_git(repo, &["init", "-b", "main"]);
        run_git(repo, &["config", "user.email", "test@example.com"]);
        run_git(repo, &["config", "user.name", "Test User"]);
        std::fs::write(repo.join("tracked.txt"), "base\n")?;
        std::fs::write(repo.join("delete-me.txt"), "bye\n")?;
        run_git(repo, &["add", "tracked.txt", "delete-me.txt"]);
        run_git(repo, &["commit", "-m", "initial"]);

        std::fs::write(repo.join("tracked.txt"), "staged\n")?;
        run_git(repo, &["add", "tracked.txt"]);
        std::fs::write(repo.join("tracked.txt"), "unstaged\n")?;
        std::fs::write(repo.join("added.txt"), "only staged\n")?;
        run_git(repo, &["add", "added.txt"]);
        run_git(repo, &["rm", "delete-me.txt"]);
        std::fs::write(repo.join("ignored-untracked.txt"), "untracked\n")?;

        let snapshot = StagedReviewSnapshot::new(repo)?;

        assert_eq!(
            std::fs::read_to_string(snapshot.cwd().join("tracked.txt"))?,
            "staged\n"
        );
        assert_eq!(
            std::fs::read_to_string(snapshot.cwd().join("added.txt"))?,
            "only staged\n"
        );
        assert!(!snapshot.cwd().join("delete-me.txt").exists());
        assert!(!snapshot.cwd().join("ignored-untracked.txt").exists());

        let cached_diff = git_stdout(snapshot.cwd(), &["diff", "--cached", "--", "tracked.txt"]);
        assert!(cached_diff.contains("-base"));
        assert!(cached_diff.contains("+staged"));

        let unstaged_diff = git_stdout(snapshot.cwd(), &["diff", "--", "tracked.txt"]);
        assert!(unstaged_diff.trim().is_empty());

        Ok(())
    }

    #[test]
    fn staged_review_snapshot_preserves_subdirectory_cwd() -> Result<(), GitToolingError> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path();

        run_git(repo, &["init", "-b", "main"]);
        run_git(repo, &["config", "user.email", "test@example.com"]);
        run_git(repo, &["config", "user.name", "Test User"]);
        std::fs::create_dir_all(repo.join("nested"))?;
        std::fs::write(repo.join("nested/file.txt"), "base\n")?;
        run_git(repo, &["add", "nested/file.txt"]);
        run_git(repo, &["commit", "-m", "initial"]);

        std::fs::write(repo.join("nested/file.txt"), "staged\n")?;
        run_git(repo, &["add", "nested/file.txt"]);
        std::fs::write(repo.join("nested/file.txt"), "unstaged\n")?;

        let snapshot = StagedReviewSnapshot::new(&repo.join("nested"))?;

        assert!(snapshot.cwd().ends_with("nested"));
        assert_eq!(
            std::fs::read_to_string(snapshot.cwd().join("file.txt"))?,
            "staged\n"
        );

        Ok(())
    }
}
