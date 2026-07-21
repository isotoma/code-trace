# Git-repo-only tracing

## Goal

Only send traces when the session's working directory is inside a git
repository. This is the new **default**; a single env var disables it.

## Behavior

- **Default (`CODE_TRACE_REQUIRE_GIT_REPO` unset or `true`)**: emit only when
  the payload's cwd is inside a git work tree. Any subdirectory of a repo
  counts — `git rev-parse` walks up to the repo root.
- **`CODE_TRACE_REQUIRE_GIT_REPO=false`**: trace everywhere (pre-change
  behavior).

The variable is read from the environment or `~/.config/code-trace/config`
like every other setting (loaded into env in `main()`).

## Implementation

- `tags::cwd_in_git_repo(cwd: Option<&str>) -> bool` — runs
  `git rev-parse --is-inside-work-tree` in `cwd` via the existing `git_cmd`
  helper; true only on a literal `true` result.
- `langfuse::require_git_repo() -> bool` — reads `CODE_TRACE_REQUIRE_GIT_REPO`;
  defaults to `true`, false only when the value lowercases to `false`.
- Gate in `run()` (main.rs), right after `cwd` is computed and before
  `gather_env_tags` / any state recording:

  ```rust
  if langfuse::require_git_repo() && !tags::cwd_in_git_repo(cwd.as_deref()) {
      log::debug("cwd not in a git repo; skipping (CODE_TRACE_REQUIRE_GIT_REPO)");
      return 0;
  }
  ```

  Early-return style: no session recorded, no cursor advanced. If the user
  later moves into a git repo, first-contact-skip handles the transition.

## Consequences (accepted)

1. Existing installs stop tracing outside git repos on upgrade until the user
   sets `CODE_TRACE_REQUIRE_GIT_REPO=false`.
2. If `git` is not installed, `rev-parse` fails → treated as "not a repo" →
   nothing traces under the default.
3. Shared integration `TestEnv` runs in non-git temp dirs, so it must set
   `CODE_TRACE_REQUIRE_GIT_REPO=false` by default to keep delivery tests
   emitting; the gate gets its own dedicated tests.

## Tests

- Unit: `cwd_in_git_repo` (temp dir with `git init` → true; plain temp dir →
  false); `require_git_repo` env parsing (unset → true, `false` → false,
  `true` → true).
- Integration: a non-git cwd produces no delivery; the same payload with the
  gate disabled does.
