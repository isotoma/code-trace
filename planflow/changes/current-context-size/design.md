# current-context-size Design

## Summary

Add a per-trace `current_context_size` metric to the Langfuse generation observation's `usageDetails`, reproducing the number shown in the OpenCode TUI "Context" panel (e.g. `69,406`). For OpenCode it is `input + output + reasoning + cache.read + cache.write` of the trace's last assistant message with `output > 0`; for Claude Code the same code path reduces to Anthropic's four usage fields. Implementation threads `reasoning_tokens` through the existing normalised `message.usage` seam (opencode.rs), adds a source-agnostic `get_context_size` accessor (transcript.rs), and appends one conditional key to the emitted `usageDetails` (emit.rs). Existing summed token reporting is untouched; the metric is omitted (never zero) when token data is absent; Pi traces are unaffected.

## Definition of Done

- [ ] Each Langfuse trace produced by code-trace from an OpenCode session carries a generation `usageDetails.current_context_size` equal to the value the opencode TUI "Context" panel would show at that point in the session.
- [ ] The same applies to Claude Code traces, using the Anthropic-usage equivalent formula.
- [ ] Existing `usageDetails` keys (`input`, `output`, `cache_creation_input_tokens`, `cache_read_input_tokens`) and their summed values are byte-for-byte unchanged.
- [ ] Traces with no usable token data produce no `current_context_size` key (no zeroes).
- [ ] Pi-agent traces are unaffected.
- [ ] Covered by unit tests (normaliser + emit) and integration tests for both OpenCode and Claude Code fixtures.

## Architecture

Both OpenCode and Claude Code sources converge on the normalised Claude-style `message.usage` block before `emit.rs` builds the Langfuse batch. The metric is computed **once, source-agnostically**, from that block.

Approved approach: **A — carry `reasoning` as a `reasoning_tokens` key in the normalised `message.usage` Value**, so a single formula serves both sources.

### Component changes

1. **`src/opencode.rs`** — `extract_opencode_usage` (currently src/opencode.rs:206) additionally emits `"reasoning_tokens": get("reasoning")` in the returned usage Value. `get()` already defaults to 0 when the key is absent, so v1-format messages (`info.metadata.assistant.tokens`) need no handling. No other change.

2. **`src/transcript.rs`** — new function alongside `get_usage` (src/transcript.rs:119):

   ```rust
   pub fn get_context_size(msg: &Value) -> Option<u64> {
       let u = msg.get("message")?.get("usage")?;
       let get = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
       Some(get("input_tokens") + get("output_tokens") + get("reasoning_tokens")
            + get("cache_read_input_tokens") + get("cache_creation_input_tokens"))
   }
   ```

   Returns `None` when no usage block exists — same "absent ≠ zero" contract as `get_usage`. `Usage` struct and `get_usage` are **untouched**.

3. **`src/emit.rs`** — in `build_ingestion_batch`, computed next to `total_usage` (currently src/emit.rs:106):

   - scan `turn.assistant_msgs` in reverse for the first message with a usage block and `output_tokens > 0`, then `get_context_size` it.
   - if found, add `"current_context_size"` to the existing `usageDetails` json! object (src/emit.rs:190) — only when `usageDetails` is already being emitted (i.e. `total_usage` is `Some`).

### Data flow

```
OpenCode SDK message info.tokens {input, output, reasoning, cache{read,write}}
  → extract_opencode_usage (adds reasoning_tokens)
  → normalised message.usage
                                        ┌→ get_usage (summed, existing 4 keys) — unchanged
Claude Code JSONL message.usage ────────┤
                                        └→ get_context_size (last msg with output>0)
  → build_ingestion_batch → generation usageDetails { input, output,
      cache_creation_input_tokens, cache_read_input_tokens, current_context_size }
```

No changes to `src/turns.rs`, `src/main.rs`, `src/pi_agent.rs`, or either TypeScript plugin.

### Semantics & edge cases

- **TUI parity (OpenCode):** per turn (one turn = one trace), `current_context_size` = `input + output + reasoning + cache.read + cache.write` from the turn's **last assistant message with `output > 0`** — exactly the OpenCode TUI sidebar formula (`packages/opencode/src/cli/cmd/tui/routes/session/sidebar.tsx`, the `context()` memo). In a multi-step tool-call turn this is the final step — the largest context of the turn and the number on screen at idle.
- **Claude Code:** same code path; the formula reduces to `input_tokens + output_tokens + cache_read_input_tokens + cache_creation_input_tokens` (`reasoning_tokens` absent → 0). Anthropic's API exposes no reasoning field; this is straight accounting of the final call's window.
- **`output_tokens > 0` guard:** skips degenerate/cancelled assistant steps, mirroring the TUI's `findLast(x => x.role === "assistant" && x.tokens.output > 0)`. Applied uniformly; every real API reply has output > 0, so for Claude Code it only filters pathological entries. If the last assistant message *lacks usage entirely*, the reverse scan falls back to an earlier qualifying message — matching the TUI's backward scan.
- **Absent ≠ zero:** when no qualifying assistant message exists, `current_context_size` is omitted from `usageDetails` — never emitted as `0`. When the turn has no usage at all, no `usageDetails` object is created (existing behaviour). Deliberate asymmetry: a turn whose only assistant steps have `output_tokens == 0` still gets summed `usageDetails` but no `current_context_size`; this mirrors the TUI and must carry a code comment.
- **Pi:** no usage block → `get_context_size` returns `None` → nothing emitted. `pi_agent.rs` untouched.
- **Privacy:** the metric is a token count only; no new content leaves the machine.

## Existing Patterns followed

- **Normalise-then-emit:** sources translate to the Claude-format `message.usage` Value; emit stays source-agnostic (same seam `extract_opencode_usage` already uses).
- **`get_usage` contract:** `Option<u64>`-style absence semantics; `?`-short-circuit on missing blocks; `unwrap_or(0)` per key.
- **Omit-don't-zero:** the existing comment at src/emit.rs:184–186 ("Omitted entirely (not zero-filled)… so Langfuse never prices a generation at $0") is the precedent for omitting `current_context_size` rather than emitting 0.
- Keeping the `Usage` struct fixed while adding a parallel accessor avoids touching its `Add` impl and the summed-pricing path.

## Implementation Phases

1. **Normaliser seam** — `opencode.rs` emits `reasoning_tokens`; unit test updates (`extracts_v2_tokens_into_usage` with non-zero `reasoning`, v1 path asserting `reasoning_tokens == 0`).
2. **Metric computation** — `transcript::get_context_size` + unit tests; `emit.rs` reverse-scan + conditional `current_context_size` in `usageDetails`; emit unit tests (last-not-summed regression, omitted-when-absent, omitted-when-zero-output, exact key set).
3. **Integration coverage** — extend `end_to_end_opencode_transcript` (multi-assistant turn, non-zero reasoning, assert TUI-formula value ≠ summed input), extend `end_to_end_simple_transcript` (Claude Code assertion), Pi fixture asserting key absence. `cargo test` green.

## Additional Considerations

- **v1 OpenCode messages:** `info.metadata.assistant.tokens` predates the `reasoning` field; `get()`'s `unwrap_or(0)` covers it.
- **usageDetails key set:** after the change, `usageDetails` contains exactly `input`, `output`, `cache_creation_input_tokens`, `cache_read_input_tokens`, and (when data exists) `current_context_size`. `reasoning_tokens` must **not** leak into emitted `usageDetails` — it exists only in the internal normalised Value.
- **Out of scope:** `tests/concurrency_test.rs` (Track 2 race suite), `harness/` (Track 1 container), any change to summed usage semantics, any front-end/Langfuse dashboard work, exposing `reasoning` as its own usageDetails key (possible later, not required by the DoD).

## Acceptance Criteria

- **current-context-size.AC1.1** *(success)* — OpenCode turn with assistant messages carrying v2 `tokens` including `reasoning`: the generation's `usageDetails.current_context_size` equals `input + output + reasoning + cache.read + cache.write` of the last assistant message with `output > 0`.
- **current-context-size.AC1.2** *(failure — regression guard)* — multi-assistant tool-loop turn: `current_context_size` is taken from the last qualifying step and **differs from** the summed per-step usage; it must not equal `total_usage`'s summed input.
- **current-context-size.AC1.3** *(failure — compat)* — OpenCode v1-format message without `reasoning`: `reasoning` contributes 0 and the formula still matches the TUI equivalent.
- **current-context-size.AC2.1** *(success)* — Claude Code trace: `current_context_size` equals `input_tokens + output_tokens + cache_read_input_tokens + cache_creation_input_tokens` of the last assistant message with `output_tokens > 0`.
- **current-context-size.AC2.2** *(failure — fallback)* — last assistant message missing a usage block entirely: metric falls back to an earlier assistant message with `output_tokens > 0`.
- **current-context-size.AC3.1** *(regression)* — the existing four `usageDetails` keys' values are byte-for-byte identical to before the change; all pre-existing `emit.rs` and integration tests pass unmodified.
- **current-context-size.AC3.2** *(failure — key hygiene)* — emitted `usageDetails` contains no `reasoning` / `reasoning_tokens` key.
- **current-context-size.AC4.1** *(failure — absent ≠ zero)* — turn with no usage data at all: no `usageDetails` object is created; no `current_context_size` key anywhere.
- **current-context-size.AC4.2** *(failure — guard)* — assistant usage present but `output_tokens == 0` on every step: `usageDetails` may exist but must not contain `current_context_size` (no zeroes).
- **current-context-size.AC5.1** *(regression)* — Pi traces carry no `current_context_size` key and existing Pi fixtures/tests pass unchanged.
- **current-context-size.AC6.1** *(verification)* — `cargo test` is green, including the new unit tests (`opencode.rs`, `transcript.rs`, `emit.rs`) and both extended integration tests.

## Glossary

- **`current_context_size`** — the new metric: token count of the context window at the end of a trace's final assistant step, per the formulas in Semantics.
- **turn** — one user message plus all following assistant messages (a tool-call loop counts as one turn); each turn produces one Langfuse trace with one generation observation. See `src/turns.rs`.
- **`message.usage`** — the normalised Claude-format usage block both sources converge on: `{input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens}` (+ `reasoning_tokens` after this change).
- **normalisation seam** — the point where a source-specific message format (OpenCode SDK, Pi entries) is translated into the Claude-format Values consumed by `emit.rs`; implemented in `src/opencode.rs` / `src/pi_agent.rs`.
- **`usageDetails`** — Langfuse's numeric usage map on a generation observation (`input`, `output`, …). Accepts arbitrary extra keys such as `current_context_size`.
- **TUI context panel** — OpenCode's sidebar display ("Context: N tokens, X% used"), computed in `sidebar.tsx` as `input + output + reasoning + cache.read + cache.write` of the last assistant message with `output > 0`.
- **OpenCode v1 / v2 formats** — two SDK message shapes for token data: v1 nests under `info.metadata.assistant.tokens`; v2 uses `info.tokens` (with `reasoning`). Both are handled by `extract_opencode_usage`.
- **absent ≠ zero** — the codebase convention of omitting a metric entirely when its source data is missing, rather than emitting 0 (which Langfuse would treat as a real measurement).
