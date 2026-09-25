# Delta for tags

## ADDED Requirements

### Requirement: Repo tag derived from origin remote URL
The system SHALL derive the `repo:<name>` tag from the last path segment of the origin remote URL (with trailing `.git` stripped), not from the working-tree directory basename.

#### Scenario: Clone in a worktree-named directory with SSH remote
- GIVEN a git repository whose origin remote is `git@github.com:org/hbf.git`
- AND the working tree directory is named `wt579b` (a linked worktree)
- WHEN `gather_env_tags` collects tags
- THEN the tag `repo:hbf` SHALL be emitted, not `repo:wt579b`

#### Scenario: Clone in a ticket-named directory with HTTPS remote
- GIVEN a git repository whose origin remote is `https://github.com/org/hbf.git`
- AND the working tree directory is named `hbf-578-idfix`
- WHEN `gather_env_tags` collects tags
- THEN the tag `repo:hbf` SHALL be emitted, not `repo:hbf-578-idfix`

#### Scenario: ssh:// protocol URL form
- GIVEN a git repository whose origin remote is `ssh://git@github.com/org/hbf.git`
- WHEN `gather_env_tags` collects tags
- THEN the tag `repo:hbf` SHALL be emitted regardless of the working-tree directory name

#### Scenario: GitLab subgroup URL
- GIVEN a git repository whose origin remote is `git@gitlab.com:acme/platform/widgets.git`
- WHEN `gather_env_tags` collects tags
- THEN the tag `repo:widgets` SHALL be emitted (last path segment, ignoring subgroups)

#### Scenario: HTTPS URL without .git suffix
- GIVEN a git repository whose origin remote is `https://github.com/acme/widgets` (no trailing `.git`)
- WHEN `gather_env_tags` collects tags
- THEN the tag `repo:widgets` SHALL be emitted

### Requirement: Repo tag fallback to show-toplevel basename
The system SHALL fall back to the `git rev-parse --show-toplevel` basename for the `repo:` tag when no origin remote exists, or when the origin remote URL is unparseable (e.g. a local filesystem path).

#### Scenario: No origin remote
- GIVEN a git repository with no origin remote configured
- WHEN `gather_env_tags` collects tags
- THEN the tag `repo:<toplevel-basename>` SHALL be emitted, where toplevel-basename is the directory name from `git rev-parse --show-toplevel`
- AND no `org:` tag SHALL be emitted

#### Scenario: Origin remote is a local path
- GIVEN a git repository whose origin remote is `/home/doug/projects/widgets` (a local filesystem path, no scheme and no SCP-style colon)
- WHEN `gather_env_tags` collects tags
- THEN `repo_from_remote_url` SHALL return `None` for the local path
- AND the tag `repo:<toplevel-basename>` SHALL be emitted as a fallback

### Requirement: Show-toplevel not called when remote URL is parseable
The system SHALL NOT invoke `git rev-parse --show-toplevel` when the origin remote URL is present and successfully parsed by `repo_from_remote_url`, to avoid a redundant git subprocess in the common case.

#### Scenario: Remote present and parseable
- GIVEN a git repository with a valid origin remote URL (SCP-style, ssh://, or https://)
- WHEN `gather_env_tags` collects tags
- THEN `repo:` SHALL be derived from `repo_from_remote_url`
- AND `git rev-parse --show-toplevel` SHALL NOT be invoked

### Requirement: repo_from_remote_url URL normalisation
The system SHALL provide a `repo_from_remote_url` function that normalises git remote URLs identically to `org_from_remote_url` but returns the last path segment (repository name) instead of the second-to-last (organisation). It SHALL return `None` for local paths and unparseable URLs.

#### Scenario: SCP-style URL
- GIVEN `repo_from_remote_url("git@github.com:acme/widgets.git")`
- WHEN the function is called
- THEN it SHALL return `Some("widgets")`

#### Scenario: HTTPS URL
- GIVEN `repo_from_remote_url("https://github.com/acme/widgets.git")`
- WHEN the function is called
- THEN it SHALL return `Some("widgets")`

#### Scenario: ssh:// protocol URL
- GIVEN `repo_from_remote_url("ssh://git@github.com/acme/widgets.git")`
- WHEN the function is called
- THEN it SHALL return `Some("widgets")`

#### Scenario: HTTPS URL without .git suffix
- GIVEN `repo_from_remote_url("https://github.com/acme/widgets")`
- WHEN the function is called
- THEN it SHALL return `Some("widgets")`

#### Scenario: GitLab subgroup URL
- GIVEN `repo_from_remote_url("git@gitlab.com:acme/platform/widgets.git")`
- WHEN the function is called
- THEN it SHALL return `Some("widgets")`

#### Scenario: Local path
- GIVEN `repo_from_remote_url("/home/doug/projects/widgets")`
- WHEN the function is called
- THEN it SHALL return `None`

## MODIFIED Requirements

### Requirement: Org tag derivation unchanged
The system SHALL derive the `org:<name>` tag from `org_from_remote_url` applied to the origin remote URL, identically to the pre-change behaviour. The restructure of the repo and org tag blocks into a single coordinated block SHALL NOT alter the org tag's value, ordering (after `repo:`), or fallback behaviour (no org tag when no remote or unparseable URL).

Previously: The org tag was derived independently in a separate block that fetched `git remote get-url origin` and called `org_from_remote_url`. The repo tag was derived independently in a separate block that fetched `git rev-parse --show-toplevel` and took the basename.

## REMOVED Requirements

### Requirement: Repo tag derived from show-toplevel basename
The system no longer derives the `repo:<name>` tag from the basename of `git rev-parse --show-toplevel` as the primary method. This is now the fallback only, used when no origin remote URL is available or the URL is unparseable. The primary derivation is from the origin remote URL's last path segment.

(Reason: the show-toplevel basename produces incorrect repo tags for git worktrees and non-standard clone directory names, as described in the design document.)
