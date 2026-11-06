/**
 * code-trace OpenCode plugin — server part (opencode v2 plugin API).
 *
 * Spawns the code-trace binary from the opencode server:
 *
 *  - `session.created` (root sessions only): runs `code-trace --on-start`,
 *    which records the session in code-trace's registry. That record is what
 *    `code-trace pause` and the sessions list operate on, so it must happen
 *    even headless (`opencode run`), where the TUI part is not loaded. The
 *    registry write is idempotent, so the TUI part (tui.ts) running
 *    `--on-start` again for its banner is harmless.
 *
 *  - `session.execution.succeeded` / `failed` / `interrupted`: pipes the
 *    messages added since the last export to the binary's stdin for
 *    Langfuse ingestion. (v1 used `session.idle`, which opencode v2 does
 *    not emit.)
 *
 * This is the v2 port of the v1 single-file plugin: `Plugin.define` with an
 * `ctx.event.subscribe` stream instead of a returned `event` hook, messages
 * from `ctx.session.context` instead of `ctx.client.session.messages`, and
 * the version from `ctx.app` instead of shelling out to `opencode --version`.
 * The on-disk cursor (`~/.local/share/code-trace/opencode_cursor.json`) is
 * unchanged, so upgrading the plugin mid-session keeps the cursor position.
 *
 * The messages are sent in opencode's native (v2 flat) shape; the binary's
 * normalizer (`src/opencode.rs`) understands it.
 */

import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { Plugin } from "@opencode/plugin";

// How long to wait for `--on-start` to finish. The binary takes the blocking
// state flock, so in the worst case (another code-trace process wedged on the
// lock) it must time out rather than stall the plugin's event loop.
const ON_START_TIMEOUT_MS = 3000;

interface CursorEntry {
  lastIndex: number;
  lastId: string;
}

interface CursorStore {
  [sessionId: string]: CursorEntry;
}

function getCursorPath(): string {
  const home = process.env.HOME ?? process.env.USERPROFILE ?? "~";
  return join(home, ".local", "share", "code-trace", "opencode_cursor.json");
}

function ensureDir(path: string): void {
  const dir = dirname(path);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
}

function loadCursor(): CursorStore {
  const path = getCursorPath();
  try {
    if (existsSync(path)) {
      return JSON.parse(readFileSync(path, "utf-8"));
    }
  } catch {
    // ignore
  }
  return {};
}

function saveCursor(store: CursorStore): void {
  const path = getCursorPath();
  ensureDir(path);
  writeFileSync(path, JSON.stringify(store, null, 2));
}

/// Run `code-trace --on-start` with an opencode-shaped payload and resolve
/// once the binary exits (its stdout is not needed here — the TUI part shows
/// the reminder). Resolves even when the binary is missing, times out, or
/// errors: recording a session is best-effort.
function runOnStart(sessionId: string, cwd: string): Promise<void> {
  const binPath = process.env.CODE_TRACE_BIN ?? "code-trace";
  const payload = JSON.stringify({
    source: "opencode",
    sessionId,
    cwd,
  });

  return new Promise((resolve) => {
    const child = spawn(binPath, ["--on-start"], {
      stdio: ["pipe", "ignore", "ignore"],
      shell: true,
    });

    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      resolve();
    }, ON_START_TIMEOUT_MS);

    child.on("error", () => {
      clearTimeout(timer);
      resolve();
    });

    child.on("close", () => {
      clearTimeout(timer);
      resolve();
    });

    child.stdin?.end(payload, "utf-8");
  });
}

/// Fetch the session's messages, send the ones past the cursor to code-trace,
/// and advance the cursor. Errors are reported on the server process's stderr
/// and swallowed: a failed export must not disturb the session.
async function exportSession(ctx: any, sessionId: string, cwd: string): Promise<void> {
  const cursor = loadCursor();
  const prev = cursor[sessionId];
  const startIndex = prev?.lastIndex ?? 0;

  let allMessages: any[];
  try {
    allMessages = await ctx.session.context({ sessionID: sessionId });
  } catch (err) {
    console.error(`code-trace: failed to fetch session messages: ${err}`);
    return;
  }

  if (allMessages.length <= startIndex) return;

  const newMessages = allMessages.slice(startIndex);
  if (newMessages.length === 0) return;

  const lastMsg = newMessages[newMessages.length - 1];
  const lastId = lastMsg?.id ?? String(startIndex + newMessages.length - 1);

  const payload = {
    source: "opencode",
    sessionId,
    cwd,
    messages: newMessages,
    agentVersion: ctx.app.version,
  };

  const binPath = process.env.CODE_TRACE_BIN ?? "code-trace";

  try {
    const child = spawn(binPath, [], {
      stdio: ["pipe", "ignore", "ignore"],
      detached: true,
      shell: true,
    });

    child.stdin?.end(JSON.stringify(payload), "utf-8");
    child.unref();
  } catch (err) {
    console.error(`code-trace: failed to spawn code-trace: ${err}`);
    return;
  }

  cursor[sessionId] = { lastIndex: allMessages.length, lastId };
  saveCursor(cursor);
}

export default Plugin.define({
  id: "code-trace",
  setup(ctx: any) {
    const controller = new AbortController();

    void (async () => {
      try {
        for await (const event of ctx.event.subscribe({
          types: [
            "session.created",
            // A run finishing (successfully, failed, or interrupted) is the
            // v2 signal that new messages may need exporting. v1 used
            // session.idle, which opencode v2 does not emit; it is kept in
            // the list in case a release restores it — the cursor makes a
            // double export a no-op.
            "session.execution.succeeded",
            "session.execution.failed",
            "session.execution.interrupted",
            "session.idle",
          ],
          signal: controller.signal,
        })) {
          // Record root sessions at creation (subagent sessions carry a
          // parentID and are skipped — same filter as the v1 banner). Resumed
          // sessions fire no session.created; they fall through to the
          // execution events. Not location-gated: recording is idempotent,
          // and the session's own location instance may not be subscribed
          // yet when the very first session in a fresh directory is created.
          if (event.type === "session.created") {
            const info = event.data ?? {};
            if (!info.sessionID || info.parentID) continue;
            const cwd = info.location?.directory ?? ctx.location.directory;
            await runOnStart(info.sessionID, cwd);
            continue;
          }

          // Execution finished: export everything added since the last
          // export. The server runs one plugin instance per location, and
          // every instance receives every session's events, so exports are
          // gated on the session belonging to this instance's location —
          // otherwise N instances race on the cursor file and the same turn
          // is traced N times.
          const sessionId = event.data?.sessionID;
          if (!sessionId) {
            console.error(`code-trace: ${event.type} event missing sessionID`);
            continue;
          }
          let cwd: string | undefined;
          try {
            const info = await ctx.session.get({ sessionID: sessionId });
            cwd = info?.location?.directory;
          } catch {
            // session lookup is best-effort
          }
          if (cwd !== ctx.location.directory) continue;
          await exportSession(ctx, sessionId, cwd!);
        }
      } catch (err) {
        // AbortError is the normal shutdown path (cleanup aborts the stream).
        if ((err as Error).name !== "AbortError") throw err;
      }
    })();

    return () => controller.abort();
  },
});
