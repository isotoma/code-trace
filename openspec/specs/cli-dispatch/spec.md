# cli-dispatch

## Purpose
The binary's argument dispatch: subcommands and flags route to handlers; everything else falls through to the stdin/emit path.

## Requirements

### Requirement: Known subcommands dispatch, everything else falls through to emit
The binary SHALL dispatch on its first argument: known subcommands and flags (`--on-start`, `status`, `sessions`, `pause`, `resume`, `purge`, `--version`, `--help`) SHALL route to their handlers; any other invocation (including no arguments) SHALL fall through to the existing stdin/emit behaviour unchanged, so the installed Stop hook (`"command": "code-trace"`) keeps working.

#### Scenario: Bare invocation still emits
- **WHEN** `code-trace` runs with no arguments and a Stop-hook payload on stdin
- **THEN** the existing emit path runs exactly as before this change

#### Scenario: Subcommand routes to handler
- **WHEN** `code-trace status` runs
- **THEN** the status handler runs and stdin is not treated as a hook payload

### Requirement: status summarizes configuration and sessions
`code-trace status` SHALL print whether tracing is configured, the Langfuse host, the count of active (non-suppressed) registry sessions, and the count of suppressed sessions.

#### Scenario: Configured with a paused session
- **WHEN** `status` runs with tracing configured and one of three registered sessions suppressed
- **THEN** the output shows the host, 2 active sessions, and 1 suppressed session

### Requirement: sessions lists the registry
`code-trace sessions` SHALL list registry entries most-recent first, showing for each: truncated session id, source, suppressed flag, last-seen time, and transcript path.

#### Scenario: Listing entries
- **WHEN** `sessions` runs with a populated registry
- **THEN** each entry appears with id, source, suppressed state, last-seen, and transcript path, ordered by `last_seen_epoch` descending

### Requirement: --version prints the crate version
`code-trace --version` SHALL print the binary's version and exit 0.

#### Scenario: Version output
- **WHEN** `code-trace --version` runs
- **THEN** stdout contains the Cargo package version

### Requirement: --help prints usage
`code-trace --help` SHALL print a usage summary covering all subcommands and exit 0.

#### Scenario: Help output
- **WHEN** `code-trace --help` runs
- **THEN** stdout lists `--on-start`, `status`, `sessions`, `pause`, `resume`, `purge`, `--version`, and `--help`
