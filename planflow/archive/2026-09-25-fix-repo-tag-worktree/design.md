# Fix repo tagging for git worktrees and non-standard clone directories — Design

## Summary

The `repo:` tag emitted by `gather_env_tags` is currently derived from the basename of `git rev-parse --show-toplevel`. For git worktrees and non-standard clone directories this produces branch-name or ticket-name tags instead of the actual repository name. This change derives the `repo:` tag from the origin remote URL (the last path segment, stripping `.git`), reusing the same `git remote get-url origin` fetch already used for the `org:` tag. It falls back to the existing show-toplevel basename when no origin remote exists or the URL is unparseable. A new private `repo_from_remote_url` function mirrors the existing `org_from_remote_url`, keeping the change minimal and the existing org-tag tests untouched.

## Definition of Done

- repo tag for a clone of e.g. `git@github.com:org/hbf.git` is `hbf` regardless of the directory name it lives in
- repo tag still falls back to the toplevel directory basename when no origin remote exists (matches the existing no-org-tag-without-remote test design)
- unit tests in `src/tags.rs` covering: worktree-style directory with an origin remote, no-remote fallback, ssh/https/git-protocol URL forms (mirror the existing `org_*` test cases)
- `cargo test` and `cargo clippy` pass
- note for reviewers: existing Langfuse traces keep the historical bad tags; no data migration in scope, but consider mentioning it in the release notes

## Architecture

### Current state

`src/tags.rs` `gather_env_tags()` produces the `repo:` tag at lines 83-87:

```rust
// Git repo name
if let Some(toplevel) = git_cmd(&["rev-parse", "--show-toplevel"], cwd) {
    if let Some(name) = std::path::Path::new(&toplevel).file_name() {
        tags.push(format!("repo:{}", name.to_string_lossy()));
    }
}
```

For a linked git worktree, `--show-toplevel` returns the worktree directory, which users typically name after the branch or ticket (e.g. `wt579b`, `hbf-578-idfix`). The same problem affects any clone directory with an unusual name. The tag is written once at trace start, so downstream consumers (code-trace-analyze) cannot fix it.

The `org:` tag (lines 90-94) already fetches the origin remote URL and passes it to `org_from_remote_url` (lines 42-71), which normalises SCP-style / ssh:// / https:// URLs and returns the second-to-last path segment.

### Change

1. **New function `repo_from_remote_url`** — a sibling to `org_from_remote_url` that normalises the remote URL the same way but returns the **last** path segment (the repo name) instead of the second-to-last. Returns `None` for unparseable URLs (local paths, etc.), same as `org_from_remote_url`.

2. **Restructure the repo + org tag blocks in `gather_env_tags`** — fetch `git remote get-url origin` once. If a URL is obtained:
   - Derive `repo:` from `repo_from_remote_url(&url)`. If that returns `None` (unparseable URL), fall back to the show-toplevel basename.
   - Derive `org:` from `org_from_remote_url(&url)` as before.
   If no remote URL is obtained (no origin remote):
   - Fall back to the show-toplevel basename for `repo:`.
   - No `org:` tag (existing behaviour).

   This replaces the current two independent blocks (lines 82-87 for repo, 90-94 for org) with a single coordinated block that fetches the remote URL once and derives both tags from it, with the show-toplevel fetch as the repo fallback.

### Fallback ordering

```
repo tag:  origin remote URL → repo_from_remote_url → repo name
           ↓ (no remote or unparseable)
           show-toplevel basename

org tag:   origin remote URL → org_from_remote_url → org name
           ↓ (no remote or unparseable)
           (no org tag)
```

The `show-toplevel` git command is only needed for the fallback path now. When a remote URL is present and parseable, the `show-toplevel` call is unnecessary. The implementation fetches the remote URL first; if it yields a valid repo name, `show-toplevel` is not called. If the remote URL is absent or unparseable, `show-toplevel` is called for the fallback. This avoids a redundant git subprocess in the common (remote-present) case.

## Existing Patterns followed

- **Sibling extraction functions** — `repo_from_remote_url` mirrors the structure and URL-normalisation logic of `org_from_remote_url` exactly, returning a different path segment. This follows the existing pattern of small, single-purpose, private functions with dedicated unit tests.
- **Test mirroring** — new `repo_*` test cases mirror the existing `org_*` test cases (`org_from_ssh_url`, `org_from_https_url`, `org_from_ssh_protocol_url`, `org_from_https_url_without_git_suffix`, `org_from_gitlab_subgroup_url`, `org_from_local_path_returns_none`) one-to-one, asserting on the repo name instead of the org.
- **Graceful fallback** — the no-remote fallback to show-toplevel basename follows the same design as the existing `no_org_tag_without_remote` test: when git data is unavailable, the tag is simply not emitted or falls back to a local alternative.
- **`git_cmd` helper** — all git subprocess calls use the existing `git_cmd(&[...], cwd)` helper at lines 5-22.

## The loop

**What the agent runs:** `cargo test` and `cargo clippy` in the repo root — standard Rust toolchain, genuine coverage. New unit tests in `src/tags.rs` exercise `repo_from_remote_url` directly (pure-function tests, no git subprocess) and `gather_env_tags` end-to-end via temp-dir git repos with remotes (mirroring `org_tag_emitted_when_remote_present`). No falsework. No integration tests need changes — no existing test asserts on the `repo:` tag value from `gather_env_tags`.

**What stays human-verified:** the release-notes note about historical Langfuse traces retaining bad tags is a documentation concern, not a testable code property.

## Implementation Phases

1. **Add `repo_from_remote_url`** — new private function in `src/tags.rs`, sibling to `org_from_remote_url`. Normalises the URL identically, returns the last path segment instead of the second-to-last. Returns `None` for unparseable URLs.

2. **Add `repo_from_remote_url` unit tests** — mirror the 6 existing `org_*` test cases (`org_from_ssh_url` → `repo_from_ssh_url`, etc.) plus `org_from_gitlab_subgroup_url` and `org_from_local_path_returns_none`. Assert on the repo name (last segment).

3. **Restructure `gather_env_tags` repo + org blocks** — replace the current two separate blocks (lines 82-94) with a single block: fetch the origin remote URL once; if present and parseable, derive `repo:` from `repo_from_remote_url` and `org:` from `org_from_remote_url`; if absent or unparseable for repo, fall back to `show-toplevel` basename for `repo:` only.

4. **Add `gather_env_tags` integration-style tests** — two new tests mirroring `org_tag_emitted_when_remote_present` and `no_org_tag_without_remote`: one that creates a temp git repo with an origin remote and asserts both `repo:widgets` and `org:acme` are present (proving the repo tag comes from the URL, not the temp-dir name); one that creates a repo with no remote and asserts `repo:` falls back to the temp-dir basename.

5. **Run `cargo test` and `cargo clippy`** — verify all tests pass and no clippy warnings. Fix any issues.

## Additional Considerations

- **Historical data** — existing Langfuse traces retain the directory-name-derived repo tags. No data migration is in scope. The release notes should mention this so reviewers understand the tag values change going forward but historical traces are unaffected.
- **code-trace-analyze (cta)** — not touched. Its aggregation is correct; the fix is upstream at the tag-derivation source.
- **Python reference implementation** (`docs/langfuse_hook.py`) — uses the old show-toplevel logic and has no `org:` tag. It is already out of sync with the Rust implementation and is historical reference material; no change needed.
- **README documentation** — `README.md` and `plugin/README.md` list `repo:<name>` in tag tables without describing derivation semantics. No doc change strictly required; the example `repo:my-project` remains valid in form.
- **URL-normalisation duplication** — `repo_from_remote_url` and `org_from_remote_url` both normalise the URL independently. This is acceptable for two small functions; if a third URL form ever appears, a refactor to a shared `parse_remote_url` returning both segments (Approach B from brainstorming) is straightforward.

## Acceptance Criteria

### DoD 1: repo tag derived from remote URL regardless of directory name

- **fix-repo-tag-worktree.AC1.1 (success):** A git repo cloned into a directory named `wt579b` with origin `git@github.com:org/hbf.git` produces the tag `repo:hbf`, not `repo:wt579b`.
- **fix-repo-tag-worktree.AC1.2 (success):** A git repo cloned into a directory named `hbf-578-idfix` with origin `https://github.com/org/hbf.git` produces the tag `repo:hbf`, not `repo:hbf-578-idfix`.
- **fix-repo-tag-worktree.AC1.3 (success):** A git repo with origin `ssh://git@github.com/org/hbf.git` produces `repo:hbf` regardless of its directory name.
- **fix-repo-tag-worktree.AC1.4 (failure):** A git repo with an origin remote that is a local path (e.g. `/home/doug/projects/hbf`) does not produce a remote-derived repo tag; it falls back to the show-toplevel basename (see AC2).

### DoD 2: fallback to show-toplevel basename when no origin remote

- **fix-repo-tag-worktree.AC2.1 (success):** A git repo with no origin remote produces `repo:<toplevel-basename>` where toplevel-basename is the directory name from `git rev-parse --show-toplevel`.
- **fix-repo-tag-worktree.AC2.2 (success):** A git repo with an origin remote that is a local path (unparseable by `repo_from_remote_url`) falls back to `repo:<toplevel-basename>`.
- **fix-repo-tag-worktree.AC2.3 (failure):** A git repo with no origin remote does not produce an `org:` tag (existing behaviour preserved — `no_org_tag_without_remote` test still passes).

### DoD 3: unit tests covering URL forms and fallback

- **fix-repo-tag-worktree.AC3.1 (success):** `repo_from_ssh_url` test asserts `repo_from_remote_url("git@github.com:acme/widgets.git") == Some("widgets")`.
- **fix-repo-tag-worktree.AC3.2 (success):** `repo_from_https_url` test asserts `repo_from_remote_url("https://github.com/acme/widgets.git") == Some("widgets")`.
- **fix-repo-tag-worktree.AC3.3 (success):** `repo_from_ssh_protocol_url` test asserts `repo_from_remote_url("ssh://git@github.com/acme/widgets.git") == Some("widgets")`.
- **fix-repo-tag-worktree.AC3.4 (success):** `repo_from_https_url_without_git_suffix` test asserts `repo_from_remote_url("https://github.com/acme/widgets") == Some("widgets")`.
- **fix-repo-tag-worktree.AC3.5 (success):** `repo_from_gitlab_subgroup_url` test asserts `repo_from_remote_url("git@gitlab.com:acme/platform/widgets.git") == Some("widgets")`.
- **fix-repo-tag-worktree.AC3.6 (success):** `repo_from_local_path_returns_none` test asserts `repo_from_remote_url("/home/doug/projects/widgets") == None`.
- **fix-repo-tag-worktree.AC3.7 (success):** `repo_tag_from_remote_when_present` test creates a temp git repo with origin `git@github.com:acme/widgets.git` and asserts `gather_env_tags` returns both `repo:widgets` and `org:acme` (repo tag is URL-derived, not temp-dir-name-derived).
- **fix-repo-tag-worktree.AC3.8 (success):** `repo_tag_falls_back_to_toplevel_without_remote` test creates a temp git repo with no remote and asserts `gather_env_tags` returns `repo:<temp-dir-basename>`.

### DoD 4: cargo test and cargo clippy pass

- **fix-repo-tag-worktree.AC4.1 (success):** `cargo test` passes with zero failures (all existing tests plus new tests).
- **fix-repo-tag-worktree.AC4.2 (success):** `cargo clippy` passes with zero warnings.
- **fix-repo-tag-worktree.AC4.3 (failure):** If `cargo test` or `cargo clippy` fails, the change is not complete.

### DoD 5: release-notes note about historical traces

- **fix-repo-tag-worktree.AC5.1 (success):** The design document (this file) notes that existing Langfuse traces retain historical bad repo tags and no data migration is in scope. (This is satisfied by the Additional Considerations section above; the release-notes mention itself is a human action at release time, not a code-testable property.)

## Glossary

- **repo tag** — the `repo:<name>` Langfuse trace tag emitted by `gather_env_tags`, intended to identify the git repository the traced session was running in.
- **org tag** — the `org:<name>` Langfuse trace tag emitted by `gather_env_tags`, derived from the origin remote URL's second-to-last path segment (the GitHub/GitLab owner or organisation).
- **show-toplevel** — `git rev-parse --show-toplevel`, which returns the absolute path of the working tree root. For a linked worktree this is the worktree directory, not the main repository.
- **linked worktree** — a git worktree created with `git worktree add`, which checks out a branch in a separate directory that shares the main repository's `.git` store. Users typically name the worktree directory after the branch or ticket.
- **remote URL** — the URL of the `origin` git remote, fetched via `git remote get-url origin`. Common forms: SCP-style (`git@github.com:org/repo.git`), ssh:// (`ssh://git@github.com/org/repo.git`), https:// (`https://github.com/org/repo.git`).
- **`org_from_remote_url`** — existing private function in `src/tags.rs` that extracts the organisation (second-to-last path segment) from a remote URL.
- **`repo_from_remote_url`** — new private function in `src/tags.rs` that extracts the repository name (last path segment) from a remote URL, mirroring `org_from_remote_url`.
- **`gather_env_tags`** — public function in `src/tags.rs` that collects environment-derived tags (repo, org, branch, user, host, os, agent version) for a Langfuse trace.
- **code-trace-analyze (cta)** — the downstream tool that aggregates Langfuse traces by repo/branch. Not in scope for this change; its aggregation logic is correct.
