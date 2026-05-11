# Non-interactive mode

For information about non-interactive mode, see [this documentation](https://developers.openai.com/codex/noninteractive).

## Code review

`codex exec review` runs Codex's reviewer without starting the TUI. Pick one
native target and add path or instruction constraints when needed:

```sh
codex exec review --commit HEAD --title "$(git log -1 --pretty=%s)"
codex exec review --base main --files codex-rs/exec/src/lib.rs codex-rs/core/src/review_prompts.rs
codex exec review --base-commit <checkpoint-sha> --pathspec-from-file /tmp/review-paths.txt
codex exec review --uncommitted --instructions-file /tmp/review-lens.txt
```

Targets:

- `--uncommitted` reviews staged, unstaged, and untracked changes.
- `--base <branch>` reviews changes against a base branch.
- `--base-commit <sha>` reviews changes since a specific commit.
- `--commit <sha>` reviews one commit. Use `--title` to provide its subject.
- A positional prompt can provide custom reviewer instructions when no native
  target fits.

Scope and output controls:

- `--files` limits review to one or more pathspecs. `--paths` is accepted as
  an alias.
- `--pathspec-from-file <file>` reads newline-delimited pathspecs. Blank lines
  and lines starting with `#` are ignored.
- `--instructions <text>` and `--instructions-file <file>` add supplemental
  reviewer instructions to a native target.
- `--output-review-json <file>` writes the structured review result.
- `--timeout <duration>` interrupts long reviews. Durations accept suffixes
  like `300s`, `5m`, or `1000ms`.
- `--fail-on-findings` exits with status 1 when the reviewer reports findings.

`codex exec review` rejects nested review runs by setting `CODEX_REVIEW_MODE`
inside the review sub-agent. If a review needs a second pass, fix the material
findings in the parent session and run `codex exec review` again over the same
bounded scope.
