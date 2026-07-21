# Secret masking before send

## Goal

Redact shape-recognizable secrets (API keys, tokens, private keys) from trace
content before it is sent to Langfuse. On by default; one env var disables it.

Scope is **secrets/credentials only** — not PII, not project-specific strings.
Detection is a curated set of high-precision regex patterns (near-zero false
positives). Best-effort: it reduces leakage, it does not eliminate it.

## Behavior

- **Default (`CODE_TRACE_MASK_SECRETS` unset or `true`)**: mask secrets in all
  emitted trace content.
- **`CODE_TRACE_MASK_SECRETS=false`**: no masking.

Read from env or `~/.config/code-trace/config` like every other setting.

## Module: `src/mask.rs`

Ordered rules compiled once via `OnceLock<Vec<Rule>>`. A `Rule` pairs a
compiled `Regex` with a replacement. Rules run in sequence, specific before
generic, so a replacement placeholder is never re-matched by a later rule.

Replacement is a typed placeholder naming the *kind*, never the value:
`[REDACTED:aws-key]`, `[REDACTED:github-token]`, etc.

### Ruleset (high-precision tier)

| Kind | Shape (conservative) | Placeholder |
|---|---|---|
| Private keys | `-----BEGIN … PRIVATE KEY----- … END …` | `[REDACTED:private-key]` |
| AWS key id | `(AKIA\|ASIA)[0-9A-Z]{16}` | `[REDACTED:aws-key]` |
| GitHub | `gh[pousr]_…`, `github_pat_…` | `[REDACTED:github-token]` |
| Anthropic | `sk-ant-…` | `[REDACTED:anthropic-key]` |
| OpenAI | `sk-[A-Za-z0-9]{32,}` | `[REDACTED:openai-key]` |
| Slack | `xox[baprs]-…` | `[REDACTED:slack-token]` |
| Google | `AIza[0-9A-Za-z_-]{35}` | `[REDACTED:google-key]` |
| Stripe | `(sk\|rk\|pk)_live_…` | `[REDACTED:stripe-key]` |
| JWT | `eyJ….eyJ….…` | `[REDACTED:jwt]` |
| Bearer header | `Bearer <token>` | `Bearer [REDACTED:token]` |
| URL creds | `://user:pass@` | `://user:[REDACTED:password]@` |

Ordering note: the Anthropic rule (`sk-ant-`) runs before the OpenAI rule
(`sk-…`) so an Anthropic key is labelled correctly and not double-matched.

Structured so a contextual tier (generic `password=…` assignments, bare AWS
secret keys) and user-supplied literals can be added later without rework.

### API

- `enabled() -> bool` — reads `CODE_TRACE_MASK_SECRETS`, default `true`.
- `mask_str(&str) -> (String, usize)` — redacted text + count of redactions.
- `mask_value(&mut Value) -> usize` — recurse over a JSON value, mask every
  string leaf in place, return the total count.

## Integration: `src/emit.rs`

Masking happens **before** `truncate()`, at the existing chokepoints, only when
`mask::enabled()`:

- user prompt text → `mask_str`
- last assistant text → `mask_str`
- tool **result** string → `mask_str`
- tool **input** → `mask_value` (recurse). Today `emit.rs` passes non-string
  tool inputs through raw; structured inputs like `{"command": "..."}` or
  `Write`'s `{"content": "..."}` are a real leak surface, so they must be
  walked, not just the string path.

The redaction count is folded into the relevant existing `*_meta` metadata as
`"redacted": N`, so masking is observable without exposing what was masked.

## Dependency

Adds the `regex` crate (standard, well-audited). Patterns compiled once.

## Limitation (documented in README)

Best-effort. Catches shaped secrets; misses novel or unshaped ones. `pause` and
the git-repo gate remain the airtight privacy controls.

## Tests (TDD)

- `mask.rs`: per-pattern positive + negative cases; ordering (Anthropic vs
  OpenAI); `mask_value` recursion over nested tool-input JSON; `enabled()` env
  parsing (unset → true, `false` → false).
- `emit.rs`: a planted key in the prompt, in a structured tool input, and in a
  tool result is redacted in the built events, and the count lands in metadata;
  a gate-off (`CODE_TRACE_MASK_SECRETS=false`) test leaves content untouched.
