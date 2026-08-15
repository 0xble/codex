# Codex fork maintenance contract

This file governs the maintained `0xble/codex` fork and the local Codex installation derived from it. It is adapted from the Hermes fork contract, but Codex-specific paths, validators, release behavior, and patch records are authoritative here.

## Repository and runtime topology

- **Canonical checkout:** `/Users/brianle/Repos/codex`
- **Maintained branch:** `main`
- **Fork remote:** `origin` → `git@github.com:0xble/codex.git`
- **Read-only upstream:** `upstream` → `git@github.com:openai/codex.git`
- **Rust workspace:** `/Users/brianle/Repos/codex/codex-rs`
- **Release artifact:** `codex-rs/target/release/codex`
- **Stable fork install:** `~/.local/opt/codex/local/libexec/codex`
- **PATH wrapper:** `~/.local/bin/codex`
- **Official fallback:** `~/.codex/packages/standalone/current/bin/codex`

Never push to `upstream`. The wrapper and official fallback are separate runtime candidates; a successful source build is not proof that the fork is installed or active.

## Current fork state

At the time this contract was created, `main` was clean, at `bb8a282e39a7943a95585c3bc3c61648601123ec`, and was 16 commits ahead of the locally fetched `upstream/main` at `63002bdb26c939925f3fa59b9575cc0a3564cb45`. Re-fetch both remotes before relying on these values.

The maintained fork currently contains intentional changes in review automation, CLI and exec guardrails, TUI account/status presentation, session-ID override behavior, plugin parsing compatibility, generated artifacts, tests, and fork version metadata. Do not infer that a fork-only commit is obsolete merely because upstream has a similarly named feature; compare behavior and executable tests.

## Patch register

Each record below tracks one intentional fork-only commit. A commit, issue, and pull request are different evidence types. The upstream PR fields were checked against the live `openai/codex` repository on **2026-08-15**. Exact commit-to-PR API lookup found no direct upstream PR for these fork commits. Keyword matches are recorded only as related context, never as direct association.

### CODEX-001 — Account status-line item styling

- **Commit:** `ce80d031714e297630df08b2dd7b5c52ddce55a4` — `fix(tui): style account status line item`
- **Status:** Active
- **Summary:** Preserve the fork’s account status-line presentation and its intended visual treatment.
- **Surfaces:** TUI account/status-line rendering and focused snapshot or status tests.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run the focused `codex-tui` status-line tests and snapshots, then the fork diff-selected test target.
- **Rollback:** Revert only this commit and its focused snapshot changes.
- **Retirement:** Retire when released upstream provides equivalent account status-line styling and the fork snapshot is no longer intentionally different.

### CODEX-002 — Empty status-line segments

- **Commit:** `5cb21b7ac4df7a21f8279d5db285b0e54f21e82d` — `fix(tui): skip empty status line segments`
- **Status:** Active
- **Summary:** Avoid rendering empty status-line segments in the fork’s TUI.
- **Surfaces:** TUI status-line assembly and related tests.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run focused TUI status and layout tests.
- **Rollback:** Revert this commit while preserving later status-line changes.
- **Retirement:** Retire after upstream rejects equivalent empty-segment output without regressing intentional spacing.

### CODEX-003 — Fork status-line construction

- **Commit:** `54664c79106f2765f1d7530b6c81cfbc44565aea` — `fix(tui): keep fork status line building`
- **Status:** Active
- **Summary:** Preserve the fork-specific status-line construction path after upstream changes.
- **Surfaces:** TUI status-line builder and focused rendering tests.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run focused TUI status-line tests and snapshots.
- **Rollback:** Revert only the construction fix and its tests.
- **Retirement:** Retire when upstream supplies the same stable construction behavior.

### CODEX-004 — Skills-list thread-start test

- **Commit:** `7821428c20c59405a076e10f3795602249520a84` — `fix(codex): update skills list thread start test`
- **Status:** Active
- **Summary:** Keep the fork’s skills-list thread-start regression aligned with its behavior.
- **Surfaces:** Thread-start behavior and app-server/core test fixtures.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run the focused thread-start and skills-list tests.
- **Rollback:** Revert the test and any behavior it uniquely protects as one unit.
- **Retirement:** Retire when upstream’s test and behavior provide equivalent coverage.

### CODEX-005 — Link scopes for account status

- **Commit:** `53a3fa1864f60ea9905fd9fa95ef7f9e0b12686b` — `fix(tui): use link scopes for account statusline`
- **Status:** Active
- **Summary:** Use link scopes for the fork’s account status-line styling.
- **Surfaces:** TUI status styling and snapshots.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run focused status styling and snapshot tests.
- **Rollback:** Revert the scope change and associated snapshot only.
- **Retirement:** Retire when upstream adopts equivalent scopes or the fork’s deliberate visual distinction is removed.

### CODEX-006 — Review CLI and exec guardrails

- **Commit:** `b2f20bcb0e3eb2310deb62ed9e647b6f34d7f0b5` — `feat(cli,exec): harden codex review CLI guardrails`
- **Status:** Active
- **Summary:** Reject unsupported root and exec `--image` usage, restore explicit merge-base instructions for scoped reviews, and prevent duplicate `ExitedReviewMode` JSONL items while preserving last-message recovery.
- **Surfaces:** CLI parsing, exec event processing, review prompts, JSONL output, and related integration tests.
- **Upstream tracking:** Exact commit lookup found no direct upstream PR. Related keyword results included closed #20837 and merged #31473; neither is treated as source-equivalent.
- **Upstream PR:** None after checked 2026-08-15. Related: #20837, #31473; not direct associations.
- **Regression:** Run focused CLI, exec, review, JSONL, and app-server review tests; then the fork-selected `just test` targets.
- **Rollback:** Revert the guardrail implementation, prompt changes, output changes, and their focused tests together. Preserve the established fork version suffix policy.
- **Retirement:** Retire only after released upstream satisfies all four behaviors: input rejection, explicit merge-base guidance, no duplicate JSONL review item, and last-message recovery.

### CODEX-007 — Blue account-status scopes

- **Commit:** `dd8611be5702a7d4f6bb5106fc67b4de8a739c0d` — `style(tui): use blue scopes for account statusline accent`
- **Status:** Active
- **Summary:** Use cool/blue semantic scopes for account text in the TUI.
- **Surfaces:** TUI status styling and snapshot coverage.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run focused TUI styling and snapshot tests.
- **Rollback:** Revert this styling-only commit and its snapshot.
- **Retirement:** Retire when upstream provides the intended equivalent styling or the fork no longer needs the distinction.

### CODEX-008 — Review automation

- **Commit:** `7dac717233bb4cf2abd6e379c753085f7f611386` — `feat(review): improve codex review automation`
- **Status:** Active
- **Summary:** Preserve the fork’s review automation improvements and their end-to-end behavior.
- **Surfaces:** Review CLI, exec flow, prompts, app-server protocol, core review tasks, TUI review surfaces, and integration tests.
- **Upstream tracking:** Exact commit lookup found no direct upstream PR. Related keyword results included closed #20837 and merged #31473; neither is treated as source-equivalent.
- **Upstream PR:** None after checked 2026-08-15. Related: #20837, #31473; not direct associations.
- **Regression:** Run focused review tests across CLI, core, exec, app-server, and TUI; use `just test -p <changed-project>`.
- **Rollback:** Revert the smallest coherent review-automation unit and its tests; do not roll back unrelated TUI or version commits.
- **Retirement:** Retire only after released upstream covers the complete fork behavior and all focused regressions remain valid.

### CODEX-009 — Review automation documentation

- **Commit:** `f7b0f3dbf5acfa2a1a7d6fe398244a9055480546` — `docs(review): document exec review automation flags`
- **Status:** Active
- **Summary:** Document the fork-specific exec review automation flags.
- **Surfaces:** `docs/exec.md` and any associated CLI help expectations.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Verify documented flags against `codex exec --help` and run relevant CLI tests.
- **Rollback:** Remove only the stale or fork-specific documentation after confirming code behavior.
- **Retirement:** Retire when upstream documentation covers the same flags and semantics.

### CODEX-010 — Review QA regressions

- **Commit:** `699199713b644cc9b2322634c14954a43d6a19ac` — `fix(review): stabilize qa regressions`
- **Status:** Active
- **Summary:** Preserve fixes for review QA regressions discovered in the fork.
- **Surfaces:** Review integration tests and the implementation paths they exercise.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Re-run the exact review QA tests first, then affected project tests.
- **Rollback:** Revert only the regression fix and its focused tests after reproducing the upstream behavior.
- **Retirement:** Retire after upstream fixes the same regressions and released behavior is verified.

### CODEX-011 — Session-ID override

- **Commit:** `a46ce012a4438a1a502858bb8c02668be3f3fc0b` — `fix(tui): restore session id override`
- **Status:** Active
- **Summary:** Restore the fork’s session-ID override behavior in the TUI.
- **Surfaces:** TUI CLI/session startup and the dedicated session-ID override suite.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association; broad search results were not source-equivalent.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run the dedicated `cli_session_id_override` suite and affected TUI tests.
- **Rollback:** Revert the override implementation and its dedicated test together.
- **Retirement:** Retire after a released upstream implementation preserves the same override contract.

### CODEX-012 — Review and plugin parsing

- **Commit:** `778073811bfcd0b81127bc2e07ecbf4e937ada90` — `fix(cli): align review and plugin parsing`
- **Status:** Active
- **Summary:** Keep review and plugin argument parsing aligned in the fork.
- **Surfaces:** CLI parsing, review commands, plugin configuration, and parser tests.
- **Upstream tracking:** Exact commit lookup found no direct upstream PR. Related merged plugin-parsing work includes #36796, but it is not an exact source mapping.
- **Upstream PR:** None after checked 2026-08-15. Related: #36796; not a direct association.
- **Regression:** Run focused CLI review/plugin parsing tests and affected `codex-cli` tests.
- **Rollback:** Revert the parser alignment and its tests without removing unrelated upstream plugin work.
- **Retirement:** Retire after released upstream provides equivalent parsing and compatibility coverage.

### CODEX-013 — Fork test repairs

- **Commit:** `0b84b2ed2bbd8c7b30208a300894821132fc0c51` — `fix: repair fork tests after upstream sync`
- **Status:** Active
- **Summary:** Repair tests required by the fork’s intentional behavior after upstream synchronization.
- **Surfaces:** Test fixtures and fork-specific review, TUI, app-server, and CLI coverage.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run all tests touched by the fork-only diff; do not treat test-only changes as disposable without checking the protected behavior.
- **Rollback:** Revert only fork-specific test repairs after confirming the corresponding behavior and fixtures upstream.
- **Retirement:** Retire when upstream tests cover the same fork contract without local repairs.

### CODEX-014 — Fork version metadata

- **Commit:** `0a80ddfd8f5e026423b70d71e655bc0d520e799f` — `chore: update fork version to 0.148.0-0xble.2.2.0`
- **Status:** Active
- **Summary:** Keep the fork version’s upstream base and established `-0xble` suffix honest after synchronization.
- **Surfaces:** Cargo/package version metadata and generated release identity.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Verify version output, package metadata, lockfile consistency, and release artifact identity.
- **Rollback:** Restore the prior version only with the matching source and generated-artifact state.
- **Retirement:** Replace with the next reconciled upstream base version while preserving the fork suffix convention.

### CODEX-015 — Generated artifacts

- **Commit:** `833d21b1c5e2c453cc72390af7efb9141c494998` — `chore: refresh generated artifacts after upstream sync`
- **Status:** Active
- **Summary:** Keep generated schemas, locks, snapshots, and other repository-owned artifacts synchronized with the fork source.
- **Surfaces:** `Cargo.lock`, app-server schemas, generated protocol artifacts, snapshots, and related source outputs.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run repository generators where required, `just fmt-check`, lock checks, and affected project tests; inspect generated diff for unintended churn.
- **Rollback:** Regenerate from the preceding verified source or revert this generated-artifact commit only.
- **Retirement:** Retire when the artifacts are regenerated as part of an upstream-satisfied source state.

### CODEX-016 — Formatted agent guidance spacing

- **Commit:** `bb8a282e39a7943a95585c3bc3c61648601123ec` — `chore: preserve spacing in formatted agent guidance`
- **Status:** Active
- **Summary:** Preserve intentional spacing in the fork’s formatted agent guidance.
- **Surfaces:** `AGENTS.md` and formatter-sensitive documentation output.
- **Upstream tracking:** Exact commit lookup found no upstream PR or issue association.
- **Upstream PR:** None after checked 2026-08-15.
- **Regression:** Run the repository formatter/check and inspect the exact documentation diff.
- **Rollback:** Revert this documentation-only commit if upstream or the formatter makes it unnecessary.
- **Retirement:** Retire when the upstream guidance and formatter preserve the same output.

## Upstream association and feedback contract

Every patch record must have separate `Upstream tracking` and `Upstream PR` fields. The PR field must identify a direct, source, associated, or merely related pull request, or explicitly say `None after checked YYYY-MM-DD`. Never promote an issue, commit, keyword-search result, or same-topic change into a PR association.

On every maintenance run, and before publishing, installing, or retiring a patch:

1. Fetch `origin` and `upstream` separately and record both exact SHAs.
2. Resolve every linked issue, PR, commit, and released descendant against the live upstream repository. Check state, reviews, requested changes, unresolved threads, comments, CI, linked commits, merge/revert state, and release evidence.
3. Judge feedback against the patch contract, current source, reproduction, and executable tests. Reviewer authority or approval state alone is not proof.
4. Classify substantive feedback as valid, invalid, stale, already addressed, or decision-required, with evidence for consequential classifications.
5. Apply valid feedback to the maintained fork and focused tests first. Then port the same verified correction to any associated upstream PR branch and read back both remote SHAs.
6. Block publication when valid feedback remains unresolved, association data is stale or ambiguous, or the fork and associated PR no longer implement the same behavior.
7. Retire fork code only after released upstream satisfies the complete behavioral contract.

## Maintenance procedure

1. Read this file, `AGENTS.md`, and the `fork-maintenance` skill. Treat repository-local instructions as applicable engineering constraints, not as evidence about current fork state.
2. Confirm the canonical checkout is on `main`. Record `HEAD`, fetched `origin/main`, fetched `upstream/main`, remotes, branch, and full worktree status.
3. If dirty, create one exact `git stash push --include-untracked`, record its ref and hash, and restore it only after synchronization and verification. Never include unrelated dirt in a maintenance commit.
4. Fetch `origin` and `upstream` separately with pruning. Map fork-only commits by behavior, not only SHA counts.
5. Rebase the maintained fork stack onto current `upstream/main`. Abort on ambiguous conflicts. Preserve only intentional fork behavior; do not retain duplicate implementations when upstream now satisfies a patch.
6. Reconcile version metadata, `Cargo.lock`, Bazel locks, schemas, snapshots, and other generated artifacts through repository-owned generators.
7. Run `just fmt`, then targeted `just test -p <project>` checks for every changed surface. For shared/core/protocol changes, follow the repository’s complete-test approval rule. Run `just fix -p <project>` for large Rust changes before finalizing and do not assume formatting alone proves correctness.
8. Independently review the exact final fork-only diff against upstream. Check every patch record and every upstream association and feedback state.
9. Build the exact release candidate with `just build-for-release` or the repository’s current canonical release command. Verify version output and artifact hash.
10. Push only to `origin/main` with `--force-with-lease=refs/heads/main:<recorded-origin-sha>`. Fetch afterward and prove remote `main` equals the verified local commit.
11. Install atomically into `~/.local/opt/codex/local/libexec/codex` only after source and release gates pass. Preserve the official fallback and verify wrapper selection, hashes, `codex --version`, and a real `codex exec --skip-git-repo-check --ephemeral --json 'Reply with exactly OK.'` smoke test.
12. If installation or smoke verification fails, restore the previous fork binary and wrapper state, then verify rollback. Source sync, push, installation, and active-runtime proof are separate completion states.
13. Restore the exact pre-existing stash and prove unrelated work is present. If restoration conflicts, retain the stash and report the exact blocker.

## Rollback

- **Sync failure:** abort the rebase and leave `main` at its preflight commit.
- **Push failure:** do not create a branch or bypass the lease; report the remote blocker.
- **Build failure:** do not install; preserve the previous installed artifact.
- **Install failure:** restore the prior fork artifact and wrapper, then run the normal version and smoke checks.
- **Runtime failure:** prefer the official fallback only as an explicit, verified rollback decision; never silently replace the maintained fork.

## Final invariants

A successful run proves:

- no rebase or merge is in progress;
- current upstream behavior is present in the maintained branch;
- every intentional fork divergence has a current patch record;
- every patch has explicit upstream-PR status and feedback review;
- local `main` and `origin/main` match the verified commit;
- unrelated dirt is restored or preserved in a named stash;
- the release artifact hash matches the installed fork binary;
- the active PATH wrapper selects the intended fork artifact;
- the real Codex smoke test passes;
- a valid rollback artifact remains available.

If the fork is already current and the installation is already verified, make no changes and report a concise verified no-op.
