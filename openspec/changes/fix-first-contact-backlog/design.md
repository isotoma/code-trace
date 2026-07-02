# Design: fix-first-contact-backlog

## Context

Chosen in discussion (2026-07-02) over alternatives: anchoring the cursor in `--on-start` (doesn't cover installs mid-session or setups without the SessionStart hook wired) and timestamp filtering (heuristic, needs a reliable install-time marker). "No cursor entry" is the precise, already-persisted signal for "code-trace has never processed this session's content."

## Goals / Non-Goals

**Goals:**
- Nothing that completed before code-trace first saw a session is ever emitted.
- Zero behaviour change for the normal case (new session, one turn per hook).
- All three sources identical.

**Non-Goals:**
- The disable-gap (cursor present, turns completed while `TRACE_TO_LANGFUSE=false`, re-enabled): fixing it would require state I/O while tracing is off, contradicting the zero-overhead-when-disabled design. Documented instead.
- Retro-active cleanup of already-leaked backlogs (that is `purge`).

## Decisions

1. **Cap to the last turn, not to nothing.** The hook fired because a turn just completed under an installed, enabled code-trace — that turn is the first legitimately traceable one. Emitting nothing on first contact would drop turn 1 of every genuinely new session, since "no cursor" is the normal state for a session's first hook.
2. **Detect first contact as cursor-key absence** (`cursors.get(&key).is_none()` before the default is materialized). Registry presence is not used: `--on-start` records sessions without cursors, and that must not defeat the cap.
3. **Skipped turns advance `turn_count`**, so the emitted turn's number equals its real position and the gap is visible — same convention as the pause fix.
4. **Ordering with suppression**: the suppressed consume-path runs first (emits nothing at all); the cap applies only on the emitting path. A session both suppressed and unseen emits nothing, as before.
5. **Per-arm implementation** mirroring the existing per-source duplication: compute `skipped`, bump `turn_count`, iterate `built_turns.iter().skip(skipped)`. A `log::info` records how many pre-contact turns were skipped, so a support question ("why is my history missing?") is answerable from the log.

## Risks / Trade-offs

- [A user *wanting* history traced on install is silently refused] → intended: opt-in backlog upload would be a separate feature; the log line makes the skip discoverable.
- [Cursor loss (7-day prune, deleted state file) makes an active session look like first contact → its next hook emits only the latest turn, dropping the gap turns] → strictly better than today, where the same situation re-emits the entire history as duplicates.
- [Stop hooks failing for several turns then recovering emits the catch-up (cursor exists)] → unchanged and desirable; only *pre-contact* content is capped.
