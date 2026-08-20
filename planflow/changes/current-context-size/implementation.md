# current-context-size — Implementation Plan

Source design: `planflow/changes/current-context-size/design.md`
Slug: `current-context-size`

All file paths and line numbers below were verified against the current code (HEAD) by a read-only scout pass on 2026-08-20. Discrepancies found: none material (see Verification Notes). Two plan-level gaps the scout surfaced are handled here as explicit new tests, not extensions of non-existent ones.

Verification command (all tasks): `cargo test`
Expected: `test result: ok.` for every test binary, zero failures.

---

## Phase 1 — Normaliser seam: carry `reasoning_tokens`

### Task 1.1 — Emit `reasoning_tokens` from `extract_opencode_usage`

**File:** `src/opencode.rs` (function `extract_opencode_usage`, lines 206–231)

**Change:** In the `json!` block at lines 225–230, add one key after `cache_read_input_tokens`:

```rust
    Some(json!({
        "input_tokens": get("input"),
        "output_tokens": get("output"),
        "cache_creation_input_tokens": cache_write,
        "cache_read_input_tokens": cache_read,
        "reasoning_tokens": get("reasoning"),
    }))
```

No other change. `get` (line 214) already returns `0` for absent keys via `unwrap_or(0)`, so v1 messages (`info.metadata.assistant.tokens`, which predate `reasoning`) automatically yield `reasoning_tokens: 0`. The v1/v2 `or_else` fallback at lines 208–213 is untouched.

**Depends on:** nothing.
**Commit:** `feat(opencode): carry reasoning_tokens through normaliser`
**ACs covered:** contributes to AC1.1, AC1.3.
**Test:** Task 1.2.

---

### Task 1.2 — Unit tests for `reasoning_tokens` extraction

**File:** `src/opencode.rs` (`#[cfg(test)] mod tests`, starts line 233)

**Change (a) — extend `extracts_v2_tokens_into_usage` (line 302):** add `"reasoning": 7` to the `tokens` block in the fixture (line 307) and add an assertion that the normalised `usage.reasoning_tokens == 7`. The existing four assertions (lines 312–315) stay unchanged.

**Change (b) — new test `extracts_v1_tokens_without_reasoning` (after line 326):** a v1-format assistant message whose `info.metadata.assistant.tokens = { input: 5, output: 9, cache: { read: 1, write: 2 } }` (no `reasoning` key). Assert: `usage.reasoning_tokens == 0` and `usage.input_tokens == 5` (proves v1 path still works and reasoning defaults to 0 — satisfies AC1.3). This is a new test, not an extension — no existing v1 usage test exists (scout flag #1).

**Verify:**
```
cargo test opencode
```
Expected: `extracts_v2_tokens_into_usage` and `extracts_v1_tokens_without_reasoning` both pass; `omits_usage_when_no_tokens_block` (line 319) still passes.

**Depends on:** 1.1.
**Commit:** `test(opencode): reasoning_tokens extraction (v2 + v1)`
**ACs covered:** AC1.1, AC1.3.

---

## Phase 2 — Source-agnostic metric: `get_context_size` + emit

### Task 2.1 — Add `get_context_size` to `transcript.rs`

**File:** `src/transcript.rs` (insert after `get_usage`, which ends at line 133; before `extract_text` at line 136)

**Change:** new public function:

```rust
/// Context-window size for one assistant message, as the OpenCode TUI
/// "Context" panel reports it: input + output + reasoning + cache read +
/// cache write. Returns None when the message has no usage block, matching
/// `get_usage`'s "absent ≠ zero" contract.
pub fn get_context_size(msg: &Value) -> Option<u64> {
    let u = msg.get("message")?.get("usage")?;
    let get = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(
        get("input_tokens")
            + get("output_tokens")
            + get("reasoning_tokens")
            + get("cache_read_input_tokens")
            + get("cache_creation_input_tokens"),
    )
}
```

`Usage` struct (line 94), its `Add` impl (lines 102–114), and `get_usage` (line 119) are **not modified**.

**Depends on:** nothing (independent of Phase 1; `reasoning_tokens` defaults to 0 when absent).
**Commit:** `feat(transcript): get_context_size accessor`
**ACs covered:** contributes to AC1.1, AC2.1.
**Test:** Task 2.2.

---

### Task 2.2 — Unit tests for `get_context_size`

**File:** `src/transcript.rs` (`mod tests`, starts line 197)

**Change — new tests (after `get_usage_absent_block_returns_none` at line 273):**

- `get_context_size_sums_all_five_fields`: a message whose `usage` has `input_tokens: 10, output_tokens: 20, reasoning_tokens: 5, cache_read_input_tokens: 3, cache_creation_input_tokens: 2` → returns `Some(40)`.
- `get_context_size_missing_fields_default_to_zero`: usage with only `input_tokens: 7` → returns `Some(7)` (other four default 0; proves Claude Code's `reasoning_tokens`-absent case reduces to the four-field Anthropic formula).
- `get_context_size_absent_block_returns_none`: no `usage` key → returns `None` (mirrors the existing `get_usage_absent_block_returns_none` contract — AC4.1 at the accessor level).

**Verify:**
```
cargo test transcript
```
Expected: all three new tests pass; existing `get_usage` tests (lines 248, 261, 273) still pass.

**Depends on:** 2.1.
**Commit:** `test(transcript): get_context_size (sum, defaults, absent)`
**ACs covered:** AC1.1, AC2.1, AC4.1 (accessor).

---

### Task 2.3 — Emit `current_context_size` in `build_ingestion_batch`

**File:** `src/emit.rs` (`build_ingestion_batch`, lines 78–269)

**Change (a) — compute the metric (insert after the `total_usage` fold, lines 106–114, before `trace_id` at line 116):**

```rust
    // Mirror the OpenCode TUI "Context" panel: the last assistant step of
    // the turn with output > 0. Omitted (not zero) when no qualifying step
    // exists, so Langfuse never records a synthetic 0 context size.
    let context_size = turn
        .assistant_msgs
        .iter()
        .rev()
        .find(|m| {
            matches!(transcript::get_usage(m), Some(u) if u.output_tokens > 0)
        })
        .and_then(transcript::get_context_size);
```

`turn.assistant_msgs` is `Vec<Value>` in chronological order (`src/turns.rs:8`, verified — reverse yields last-first; duplicate streamed fragments are already merged by `build_turns`).

**Change (b) — emit the key (modify the `usageDetails` block, lines 187–197):**

Replace the `json!` macro with a mutable `serde_json::Map` so the extra key is added only when present:

```rust
    if let Some(usage) = total_usage {
        let mut details = serde_json::Map::new();
        details.insert("input".to_string(), json!(usage.input_tokens));
        details.insert("output".to_string(), json!(usage.output_tokens));
        details.insert(
            "cache_creation_input_tokens".to_string(),
            json!(usage.cache_creation_input_tokens),
        );
        details.insert(
            "cache_read_input_tokens".to_string(),
            json!(usage.cache_read_input_tokens),
        );
        if let Some(cs) = context_size {
            details.insert("current_context_size".to_string(), json!(cs));
        }
        gen_body.insert("usageDetails".to_string(), Value::Object(details));
    }
```

The four existing keys keep identical values (AC3.1). `current_context_size` is added only inside the `if let Some(usage) = total_usage` block — so a turn with no usage at all still gets no `usageDetails` object (AC4.1), and a turn whose only steps have `output_tokens == 0` gets `usageDetails` without `current_context_size` (AC4.2). `reasoning_tokens` never appears in `usageDetails` (AC3.2).

**Depends on:** 2.1 (for `get_context_size`).
**Commit:** `feat(emit): current_context_size in generation usageDetails`
**ACs covered:** AC1.1, AC1.2, AC2.1, AC2.2, AC3.1, AC3.2, AC4.1, AC4.2.
**Tests:** 2.4.

---

### Task 2.4 — Unit tests for emit

**File:** `src/emit.rs` (`#[cfg(test)] mod tests`)

**Change (a) — extend `generation_event_carries_usage_details` (line 422):** after the existing per-key assertions (lines 444–448), add an exact-key-set assertion to catch leaks (scout flag #2, satisfies AC3.2):

```rust
        let keys: Vec<&str> = usage.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["input", "output", "cache_creation_input_tokens", "cache_read_input_tokens"]);
        // current_context_size: single assistant with output>0 → present, equals the 4-field sum.
        // (This fixture has no reasoning_tokens, so the metric = 10+20+3+5 = 38.)
        assert_eq!(usage["current_context_size"], 38);
```

Note: the existing fixture (lines 433–438) has no `reasoning_tokens`, so the expected value is `input + output + cache_read + cache_creation = 10+20+3+5 = 38` — this also proves the Claude-Code-style 4-field reduction (AC2.1).

**Change (b) — new test `current_context_size_uses_last_assistant_not_sum` (after line 484):** a turn with two assistant messages, both with `output_tokens > 0`:
- msg A: `input_tokens: 10, output_tokens: 5, cache_creation: 0, cache_read: 0, reasoning_tokens: 0`
- msg B (later): `input_tokens: 40, output_tokens: 8, cache_creation: 2, cache_read: 1, reasoning_tokens: 3`

Assert:
- `usageDetails.input == 50` (summed — AC3.1 unchanged behaviour)
- `usageDetails.current_context_size == 54` (40+8+1+2+3 from msg B, the **last** — NOT 50+13+... summed — this is the AC1.2 regression guard)
- key set includes `current_context_size` and the four standard keys, nothing else.

**Change (c) — new test `current_context_size_omitted_when_no_output` (after (b)):** a turn with one assistant message whose usage has `output_tokens: 0` (and input 5). Assert `usageDetails` exists (has `input: 5`) but **has no `current_context_size` key** (AC4.2). Optionally assert key set is exactly the four standard keys.

**Change (d) — new test `current_context_size_falls_back_to_earlier_step` (after (c)):** two assistant messages: msg A `output_tokens: 4, input 10`, msg B (later) **no usage block at all**. Assert `current_context_size` is present and equals msg A's 4-field value (AC2.2 fallback — reverse scan skips msg B, finds msg A).

**Verify:**
```
cargo test emit
```
Expected: `generation_event_carries_usage_details` and `generation_event_sums_usage_across_assistant_messages` still pass (AC3.1 regression); four new assertions/tests pass.

**Depends on:** 2.3.
**Commit:** `test(emit): current_context_size (last-not-sum, omitted, fallback, key set)`
**ACs covered:** AC1.2, AC2.1, AC2.2, AC3.1, AC3.2, AC4.2.

---

## Phase 3 — Integration coverage for both sources + Pi regression

### Task 3.1 — Extend OpenCode integration test

**File:** `tests/integration_test.rs` (`end_to_end_opencode_transcript`, lines 70–113)

**Change (a) — add `reasoning` to msg_2's tokens (line 77):**
```rust
"tokens": { "input": 12, "output": 34, "reasoning": 6, "cache": { "read": 0, "write": 0 } }
```

**Change (b) — add a second qualifying assistant step (msg_4) after msg_3, with larger input (simulating context growth after the tool result), output > 0, and reasoning. Insert after line 88 (before the closing `]` of `msgs` at line 89):**
```rust
        json!({
            "info": { "id": "msg_4", "role": "assistant", "providerID": "anthropic", "modelID": "claude-sonnet-4-20250514", "tokens": { "input": 200, "output": 15, "reasoning": 9, "cache": { "read": 4, "write": 1 } } },
            "parts": [{ "type": "text", "text": "Done!" }]
        }),
```

After normalisation, `turn.assistant_msgs` = [msg_2, msg_3(no usage), msg_4] (msg_3 is the tool_result carrier, no tokens). `build_turns` still yields 1 turn (the `turns.len() == 1` assertion at line 93 still holds — msg_4 is another assistant step in the same turn).

**Change (c) — update/add assertions (after line 112):**
```rust
    // Existing summed usage (unchanged): input 12+200=212, output 34+15=49.
    assert_eq!(events[1]["body"]["usageDetails"]["input"], 212);
    assert_eq!(events[1]["body"]["usageDetails"]["output"], 49);
    // current_context_size = last step (msg_4) TUI formula:
    //   200 + 15 + 9 + 4 + 1 = 229. Differs from summed input (212) → AC1.2.
    assert_eq!(events[1]["body"]["usageDetails"]["current_context_size"], 229);
    // AC3.2: no reasoning_tokens leaked into usageDetails.
    assert!(events[1]["body"]["usageDetails"].as_object().unwrap().get("reasoning_tokens").is_none());
```

Update the existing two assertions at lines 111–112 from `12`/`34` to `212`/`49` (the fixture now sums two assistant steps).

**Verify:**
```
cargo test --test integration_test end_to_end_opencode_transcript
```
Expected: pass.

**Depends on:** 2.3 (emit must emit the key), 1.1 (opencode must carry reasoning).
**Commit:** `test(integration): opencode current_context_size (multi-assistant, reasoning)`
**ACs covered:** AC1.1, AC1.2, AC3.2 (integration level).

---

### Task 3.2 — Extend Claude Code integration test

**File:** `tests/integration_test.rs` (`end_to_end_simple_transcript`, lines 6–37)

**Change:** the existing assistant fixture (line 9) has usage `{input_tokens: 12, output_tokens: 34, cache_creation_input_tokens: 0, cache_read_input_tokens: 0}`. Add an assertion after line 36:

```rust
    // current_context_size = 12 + 34 + 0 + 0 = 46 (no reasoning_tokens in
    // Claude Code transcripts → 4-field Anthropic formula). AC2.1.
    assert_eq!(events[1]["body"]["usageDetails"]["current_context_size"], 46);
```

No fixture change needed — the existing single-assistant turn already has `output_tokens: 34 > 0` and is the last assistant message.

**Verify:**
```
cargo test --test integration_test end_to_end_simple_transcript
```
Expected: pass.

**Depends on:** 2.3.
**Commit:** `test(integration): claude code current_context_size`
**ACs covered:** AC2.1 (integration level).

---

### Task 3.3 — Pi regression assertion

**File:** `tests/integration_test.rs` (`end_to_end_pi_agent_transcript`, lines 116–157)

**Change:** add an assertion that the Pi trace's generation event has **no** `current_context_size` key (and no `usageDetails` at all, since Pi carries no usage). After the existing metadata assertions (around line 157):

```rust
    // AC5.1: Pi traces carry no usage block → no usageDetails, no current_context_size.
    assert!(events[1]["body"].get("usageDetails").is_none());
```

(Pi's `normalize_pi_agent_messages` emits assistant messages without a `usage` key — verified at `src/pi_agent.rs:110–118` — so `total_usage` is `None`, no `usageDetails` object is created, and `current_context_size` cannot appear.)

**Verify:**
```
cargo test --test integration_test end_to_end_pi_agent_transcript
```
Expected: pass.

**Depends on:** 2.3.
**Commit:** `test(integration): pi trace has no current_context_size (regression)`
**ACs covered:** AC5.1.

---

### Task 3.4 — Full test suite green

**Command:**
```
cargo test
```

**Expected:** all test binaries report `test result: ok.`; zero failures. This is the AC6.1 verification gate. No code change — this task is the verification step that closes the plan.

**Depends on:** 3.1, 3.2, 3.3 (and transitively all of Phases 1–2).
**Commit:** none (verification only). If any test fails, fix in the responsible task before re-running.

**ACs covered:** AC6.1.

---

## AC → Task traceability

| AC | Tasks |
|----|------|
| AC1.1 | 1.1, 1.2, 2.1, 2.3, 3.1 |
| AC1.2 | 2.3, 2.4(b), 3.1 |
| AC1.3 | 1.2(b) |
| AC2.1 | 2.1, 2.2, 2.3, 2.4(a), 3.2 |
| AC2.2 | 2.3, 2.4(d) |
| AC3.1 | 2.3, 2.4(a), 2.4(b), 3.1 |
| AC3.2 | 2.3, 2.4(a), 3.1 |
| AC4.1 | 2.2, 2.3 |
| AC4.2 | 2.3, 2.4(c) |
| AC5.1 | 3.3 |
| AC6.1 | 3.4 |

Every AC maps to at least one task. Every functionality task (1.1, 2.1, 2.3) has a paired test task (1.2, 2.2, 2.4). No task depends on "this will exist somehow" — dependencies are explicit and ordered.

## Verification Notes (from scout pass)

- All design line references confirmed accurate: opencode.rs:206, transcript.rs:94/119, emit.rs:78/106/187–197, turns.rs:6–10.
- `extract_opencode_usage` returns `Option<Value>` (wrapped in `Some(json!(...)))` — design's "emits a key" wording matches.
- `turns.rs` collects `assistant_msgs` in first-seen (chronological) order; streamed duplicates are merged to their final content (lines 75–78) — reverse `.rev()` yields the true last step.
- `tests/support/fake_langfuse.rs` stores events as raw `serde_json::Value` (line 41, `Mutex<Vec<Value>>`) with no schema validation (lines 278–283) — extra `usageDetails` keys pass through and are queryable via pointer/index. No fake changes needed.
- `Cargo.toml` version 0.5.1 — unchanged by this plan.
- Scout flags handled: #1 (v1 test is new, Task 1.2b), #2 (exact-key assertion is new, Task 2.4a), #3 (opencode fixture extended with reasoning + second step, Task 3.1). Flags #4/#5 are cosmetic, no action.
