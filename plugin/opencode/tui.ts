/**
 * code-trace OpenCode plugin — TUI part (opencode v2 plugin API).
 *
 * Shows the code-trace tracing reminder (ENABLED / PAUSED / inactive) as a
 * toast when a root session is created. The v1 plugin did this from the
 * server part via `client.tui.showToast`, which no longer exists in v2; the
 * v2 TUI part owns toasts (`context.ui.toast.show`).
 *
 * This runs `code-trace --on-start` itself and parses the JSON
 * `{ codeTrace: { level, message } }` reply. The server part (index.ts) runs
 * the same command to record the session; the registry write is idempotent,
 * so running it twice is harmless. Only this part needs the output.
 *
 * Subagent sessions fire `session.created` too but carry a `parentID` —
 * skip those so a Task delegation doesn't pop a banner each time.
 */

import { spawn } from "node:child_process";
import { Plugin } from "@opencode/plugin/tui";

// How long to wait for `--on-start` to print its reminder. The binary takes
// the blocking state flock, so in the worst case (another code-trace process
// wedged on the lock) it must time out rather than stall the TUI.
const ON_START_TIMEOUT_MS = 3000;

type CodeTraceNote = { codeTrace?: { level?: string; message?: string } };

/// Run `code-trace --on-start` with an opencode-shaped payload and resolve to
/// its stdout (trimmed), or `null` if the binary is missing, times out,
/// errors, or prints nothing (tracing not configured).
function runOnStart(sessionId: string, cwd: string): Promise<string | null> {
  const binPath = process.env.CODE_TRACE_BIN ?? "code-trace";
  const payload = JSON.stringify({
    source: "opencode",
    sessionId,
    cwd,
  });

  return new Promise((resolve) => {
    const child = spawn(binPath, ["--on-start"], {
      stdio: ["pipe", "pipe", "pipe"],
      shell: true,
    });

    let stdout = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      resolve(null);
    }, ON_START_TIMEOUT_MS);

    child.on("error", () => {
      clearTimeout(timer);
      resolve(null);
    });

    child.stdout?.on("data", (chunk: Buffer) => {
      if (stdout.length < 16 * 1024) stdout += chunk.toString("utf-8");
    });

    child.on("close", (code) => {
      clearTimeout(timer);
      const trimmed = stdout.trim();
      resolve(code === 0 && trimmed ? trimmed : null);
    });

    child.stdin?.end(payload, "utf-8");
  });
}

/// Fetch the reminder and render it as a toast. No-ops when tracing is not
/// configured (no note) or the output is unparseable.
async function showBanner(context: any, sessionId: string, cwd: string): Promise<void> {
  const out = await runOnStart(sessionId, cwd);
  if (!out) return;

  let note: CodeTraceNote;
  try {
    note = JSON.parse(out);
  } catch {
    console.error(`code-trace: unparseable --on-start output: ${out}`);
    return;
  }

  const message = note.codeTrace?.message;
  if (!message) return;
  const level = note.codeTrace?.level;

  try {
    context.ui.toast.show({
      title: "code-trace",
      message,
      variant: level === "warning" ? "warning" : "info",
    });
  } catch (err) {
    console.error(`code-trace: failed to show tracing banner: ${err}`);
  }
}

export default Plugin.define({
  id: "code-trace.tui",
  setup(context: any) {
    const stop = context.data.on("session.created", (event: any) => {
      const info = event.data ?? {};
      if (!info.sessionID || info.parentID) return;
      const cwd =
        info.location?.directory ??
        context.location?.directory ??
        context.data.location.default().directory;
      void showBanner(context, info.sessionID, cwd);
    });
    return stop;
  },
});
