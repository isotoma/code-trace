# Fix repo tagging for git worktrees and non-standard clone directories — Implementation Plan

Source: `planflow/changes/fix-repo-tag-worktree/design.md`

## Codebase verification summary

All design assumptions confirmed against the current code (verified 2026-09-25):

- `src/tags.rs` is 287 lines; `org_from_remote_url` at lines 42–71, `git_cmd` at lines 5–22, `gather_env_tags` at lines 73–151.
- `repo:` block confirmed at lines 82–87 (show-toplevel basename, no remote consultation).
- `org:` block confirmed at lines 89–94 (remote URL → `org_from_remote_url`).
- Existing `org_*` test functions at lines 214–287 (6 unit tests + 2 integration tests).
- No existing `repo_from_remote_url` function or `repo_*` URL-parsing tests.
- No URL-parsing crates; manual string-splitting is the pattern.
- `git_cmd` returns `None` on failure/non-zero/empty — tests build real temp git repos.
- The only import needed (`std::process::Command`, `crate::source::Source`) is already present; no new imports required.

No discrepancies found. The plan is grounded in the code as it is now.

## Acceptance-criteria traceability

| AC | Task(s) |
|----|---------|
| AC1.1 (repo:hbf from SSH URL in worktree-named dir) | T2, T3, T4 |
| AC1.2 (repo:hbf from HTTPS URL in ticket-named dir) | T2, T3, T4 |
| AC1.3 (repo:hbf from ssh:// URL) | T2 |
| AC1.4 (local-path remote falls back to toplevel) | T2, T4 |
| AC2.1 (no remote → toplevel basename) | T3, T4 |
| AC2.2 (unparseable remote → toplevel basename) | T2, T4 |
| AC2.3 (no remote → no org tag, existing behaviour) | T3, T4 |
| AC3.1–AC3.6 (unit tests for URL forms) | T2 |
| AC3.7 (integration: repo+org from remote) | T4 |
| AC3.8 (integration: repo fallback without remote) | T4 |
| AC4.1 (cargo test passes) | T5 |
| AC4.2 (cargo clippy passes) | T5 |
| AC4.3 (failure means not complete) | T5 |
| AC5.1 (release-notes note — human action) | — (design doc already satisfies; no code task) |

## Tasks

### T1: Add `repo_from_remote_url` function

**File:** `src/tags.rs`

**What:** Add a new private function `repo_from_remote_url` as a sibling to `org_from_remote_url`, placed immediately after `org_from_remote_url` (after line 71, before `gather_env_tags` at line 73). Include a `///` doc comment mirroring the style of `org_from_remote_url`'s doc comment (lines 32–41), documenting the URL forms handled.

**Implementation detail:**

The function normalises the URL identically to `org_from_remote_url` but returns the **last** path segment instead of the second-to-last. The normalisation logic is the same:

1. Trim whitespace, strip trailing `.git`.
2. If URL contains `://`, take the path after the first `/` following the scheme part (discards `user@host`).
3. Otherwise split on `:` for SCP-style (`git@github.com:org/repo`); if no `:` and no `://`, return `None` (local path).
4. Split path on `/`, filter empty segments.
5. Require `segments.len() >= 2` (same guard — a bare repo name with no org is not a valid remote URL for this purpose).
6. Return `segments[segments.len() - 1]` (the repo name, last segment). Return `None` if empty.

```rust
/// Extract the repository name from a git remote URL.
///
/// Handles SCP-style (`git@github.com:org/repo.git`),
/// ssh:// (`ssh://git@github.com/org/repo.git`),
/// and https:// (`https://github.com/org/repo.git`) forms.
/// Returns `None` for local paths or unparseable URLs.
fn repo_from_remote_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(".git");

    let path = match url.split_once("://") {
        Some((_, rest)) => {
            let after_host = rest.split_once('/').map(|(_, h)| h).unwrap_or(rest);
            after_host
        }
        None => match url.split_once(':') {
            Some((_, rest)) => rest,
            None => return None,
        },
    };

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return None;
    }
    let repo = segments[segments.len() - 1];
    if repo.is_empty() {
        None
    } else {
        Some(repo.to_string())
    }
}
```

**Dependencies:** None (first task).

**Test:** T2 (unit tests for this function).

**Verification command:**
```bash
cargo build 2>&1
```
Expected: compiles with no errors (function is unused until T3 wires it in, so there may be a dead-code warning — that's acceptable and resolved by T3).

**Commit:** `feat: add repo_from_remote_url function`

---

### T2: Add `repo_from_remote_url` unit tests

**File:** `src/tags.rs` (in the `#[cfg(test)] mod tests` block, after the existing `org_from_local_path_returns_none` test at line 257 and before the `org_tag_emitted_when_remote_present` test at line 259)

**What:** Add 6 unit test functions mirroring the existing `org_*` URL-parsing tests one-to-one. Each asserts on the repo name (last path segment) instead of the org (second-to-last).

**Tests to add:**

```rust
#[test]
fn repo_from_ssh_url() {
    assert_eq!(
        repo_from_remote_url("git@github.com:acme/widgets.git"),
        Some("widgets".to_string())
    );
}

#[test]
fn repo_from_https_url() {
    assert_eq!(
        repo_from_remote_url("https://github.com/acme/widgets.git"),
        Some("widgets".to_string())
    );
}

#[test]
fn repo_from_ssh_protocol_url() {
    assert_eq!(
        repo_from_remote_url("ssh://git@github.com/acme/widgets.git"),
        Some("widgets".to_string())
    );
}

#[test]
fn repo_from_https_url_without_git_suffix() {
    assert_eq!(
        repo_from_remote_url("https://github.com/acme/widgets"),
        Some("widgets".to_string())
    );
}

#[test]
fn repo_from_gitlab_subgroup_url() {
    assert_eq!(
        repo_from_remote_url("git@gitlab.com:acme/platform/widgets.git"),
        Some("widgets".to_string())
    );
}

#[test]
fn repo_from_local_path_returns_none() {
    assert_eq!(
        repo_from_remote_url("/home/doug/projects/widgets"),
        None
    );
}
```

**Dependencies:** T1 (function must exist).

**Verification command:**
```bash
cargo test repo_from 2>&1
```
Expected: 6 tests pass, 0 failures:
```
running 6 tests
test tags::tests::repo_from_ssh_url ... ok
test tags::tests::repo_from_https_url ... ok
test tags::tests::repo_from_ssh_protocol_url ... ok
test tags::tests::repo_from_https_url_without_git_suffix ... ok
test tags::tests::repo_from_gitlab_subgroup_url ... ok
test tags::tests::repo_from_local_path_returns_none ... ok

test result: ok. 6 passed; 0 failed
```

**Commit:** `test: add repo_from_remote_url unit tests`

---

### T3: Restructure `gather_env_tags` repo + org blocks

**File:** `src/tags.rs` (lines 82–94 of the current code)

**What:** Replace the current two independent blocks (repo at lines 82–87, org at lines 89–94) with a single coordinated block that fetches the origin remote URL once and derives both tags. The show-toplevel git command becomes the fallback path only.

**Current code to replace (lines 82–94):**

```rust
    // Git repo name
    if let Some(toplevel) = git_cmd(&["rev-parse", "--show-toplevel"], cwd) {
        if let Some(name) = std::path::Path::new(&toplevel).file_name() {
            tags.push(format!("repo:{}", name.to_string_lossy()));
        }
    }

    // Git organisation (owner) from the origin remote URL.
    if let Some(url) = git_cmd(&["remote", "get-url", "origin"], cwd) {
        if let Some(org) = org_from_remote_url(&url) {
            tags.push(format!("org:{org}"));
        }
    }
```

**Replacement code:**

```rust
    // Git repo name and organisation from the origin remote URL.
    // Falls back to the show-toplevel basename for repo when no origin
    // remote exists or the URL is unparseable (e.g. local path).
    if let Some(url) = git_cmd(&["remote", "get-url", "origin"], cwd) {
        match repo_from_remote_url(&url) {
            Some(repo) => tags.push(format!("repo:{repo}")),
            None => {
                if let Some(toplevel) = git_cmd(&["rev-parse", "--show-toplevel"], cwd) {
                    if let Some(name) = std::path::Path::new(&toplevel).file_name() {
                        tags.push(format!("repo:{}", name.to_string_lossy()));
                    }
                }
            }
        }
        if let Some(org) = org_from_remote_url(&url) {
            tags.push(format!("org:{org}"));
        }
    } else {
        // No origin remote: fall back to show-toplevel basename for repo.
        if let Some(toplevel) = git_cmd(&["rev-parse", "--show-toplevel"], cwd) {
            if let Some(name) = std::path::Path::new(&toplevel).file_name() {
                tags.push(format!("repo:{}", name.to_string_lossy()));
            }
        }
    }
```

**Key behavioural properties of this restructure:**
- When a remote URL is present and parseable: `repo:` comes from `repo_from_remote_url`, `org:` comes from `org_from_remote_url`, and `show-toplevel` is never called (avoids a redundant git subprocess in the common case).
- When a remote URL is present but unparseable (local path): `repo:` falls back to show-toplevel basename, `org:` is not emitted (same as current `org_from_remote_url` returning `None`).
- When no origin remote exists: `repo:` falls back to show-toplevel basename, no `org:` tag (existing behaviour preserved).
- Tag ordering is preserved: `repo:` before `org:`, matching the current order.

**Dependencies:** T1 (uses `repo_from_remote_url`).

**Test:** T4 (integration tests exercise `gather_env_tags` with and without remotes). Existing `org_tag_emitted_when_remote_present` and `no_org_tag_without_remote` tests must still pass.

**Verification command:**
```bash
cargo test 2>&1
```
Expected: all existing tests pass (including `org_tag_emitted_when_remote_present` and `no_org_tag_without_remote`). No new tests yet — those come in T4. The build should have no dead-code warning now (function is used).

**Commit:** `feat: derive repo tag from origin remote URL with show-toplevel fallback`

---

### T4: Add `gather_env_tags` integration-style tests for repo tag

**File:** `src/tags.rs` (in the `#[cfg(test)] mod tests` block, after the existing `no_org_tag_without_remote` test)

**What:** Add two new integration-style tests that exercise `gather_env_tags` end-to-end via temp-dir git repos, mirroring the existing `org_tag_emitted_when_remote_present` (line 259) and `no_org_tag_without_remote` (line 279) patterns.

**Tests to add:**

```rust
#[test]
fn repo_tag_from_remote_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();
    // Initialise a git repo and add an origin remote.
    Command::new("git")
        .args(["init"])
        .current_dir(dir_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["remote", "add", "origin", "git@github.com:acme/widgets.git"])
        .current_dir(dir_path)
        .output()
        .unwrap();

    let tags = gather_env_tags(Source::Opencode, Some(dir_path), None);

    // Repo tag comes from the remote URL, not the temp-dir name.
    assert!(
        tags.iter().any(|t| t == "repo:widgets"),
        "expected repo:widgets in tags: {tags:?}"
    );
    // Org tag should also be present.
    assert!(
        tags.iter().any(|t| t == "org:acme"),
        "expected org:acme in tags: {tags:?}"
    );
    // Should NOT contain repo:<temp-dir-name>.
    let dir_basename = std::path::Path::new(dir_path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        !tags.iter().any(|t| t == format!("repo:{dir_basename}")),
        "should not contain repo:{dir_basename} in tags: {tags:?}"
    );
}

#[test]
fn repo_tag_falls_back_to_toplevel_without_remote() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();
    // Initialise a git repo with no remote.
    Command::new("git")
        .args(["init"])
        .current_dir(dir_path)
        .output()
        .unwrap();

    let tags = gather_env_tags(Source::Opencode, Some(dir_path), None);

    let dir_basename = std::path::Path::new(dir_path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        tags.iter().any(|t| t == format!("repo:{dir_basename}")),
        "expected repo:{dir_basename} in tags: {tags:?}"
    );
    // No org tag should be present without a remote.
    assert!(
        !tags.iter().any(|t| t.starts_with("org:")),
        "should not contain any org: tag in tags: {tags:?}"
    );
}
```

**Dependencies:** T3 (the restructured `gather_env_tags` that derives repo from remote URL).

**Verification command:**
```bash
cargo test repo_tag 2>&1
```
Expected: 2 tests pass, 0 failures:
```
running 2 tests
test tags::tests::repo_tag_from_remote_when_present ... ok
test tags::tests::repo_tag_falls_back_to_toplevel_without_remote ... ok

test result: ok. 2 passed; 0 failed
```

**Commit:** `test: add gather_env_tags integration tests for repo tag derivation`

---

### T5: Run full test suite and clippy

**What:** Run the complete verification suite per the design's loop section. Fix any failures or warnings that arise from the changes in T1–T4.

**Verification commands:**
```bash
cargo test 2>&1
```
Expected: all tests pass (existing + 6 new unit tests + 2 new integration tests), 0 failures.

```bash
cargo clippy 2>&1
```
Expected: zero warnings.

**If failures occur:** fix the code or tests until both commands pass clean. Common issues to watch for:
- Clippy may suggest using `unwrap_or_else` instead of `match` in the fallback — follow the suggestion if it arises.
- The `repo_from_remote_url` function's normalisation logic must produce exactly the same path-parsing behaviour as `org_from_remote_url` for all tested URL forms — if a test fails, check segment indexing (`segments.len() - 1` vs `segments.len() - 2`).
- Ensure the `tempfile` crate is already a dev-dependency (it is used by existing tests at lines 259 and 279, so it should already be available).

**Dependencies:** T1, T2, T3, T4 (all code and test changes must be in place).

**Commit:** (only if fixes were needed) `fix: resolve test/clippy issues from repo tag restructure`

---

## Task dependency graph

```
T1 (add repo_from_remote_url)
├── T2 (add unit tests)         depends on T1
└── T3 (restructure gather_env_tags)  depends on T1
    └── T4 (add integration tests)     depends on T3
        └── T5 (full test + clippy)    depends on T1, T2, T3, T4
```

T1 is the root. T2 and T3 can proceed in parallel after T1. T4 depends on T3. T5 depends on everything.
