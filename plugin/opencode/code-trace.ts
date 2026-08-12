import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";

// How long to wait for `--on-start` to print its reminder. The binary takes the
// blocking state flock, so in the worst case (another code-trace process wedged
// on the lock) it must time out rather than stall the plugin's event loop.
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

function getOpencodeVersion(): string | undefined {
  try {
    const { execSync } = require("node:child_process");
    const result = execSync("opencode --version", { encoding: "utf-8", timeout: 5000 });
    return result.trim() || undefined;
  } catch {
    return undefined;
  }
}

/// Run `code-trace --on-start` with an opencode-shaped payload and resolve to
/// its stdout (trimmed), or `null` if the binary is missing, times out, errors,
/// or prints nothing (tracing not configured). The output is a JSON
/// `{ codeTrace: { level, message } }` object; the caller parses it.
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

type CodeTraceNote = { codeTrace?: { level?: string; message?: string } };

/// Render the `--on-start` reminder as a TUI toast. No-ops when tracing is not
/// configured (no note) or when no TUI is attached (headless `opencode run`).
async function showBanner(ctx: any, sessionId: string, cwd: string): Promise<void> {
  const out = await runOnStart(sessionId, cwd);
  if (!out) return;

  let note: CodeTraceNote;
  try {
    note = JSON.parse(out);
  } catch {
    await ctx.client.app.log({
      body: {
        service: "code-trace",
        level: "warn",
        message: `unparseable --on-start output: ${out}`,
      },
    });
    return;
  }

  const message = note.codeTrace?.message;
  if (!message) return;
  const level = note.codeTrace?.level;

  try {
    await ctx.client.tui.showToast({
      body: {
        title: "code-trace",
        message,
        variant: level === "warning" ? "warning" : "info",
      },
    });
  } catch (err) {
    await ctx.client.app.log({
      body: {
        service: "code-trace",
        level: "error",
        message: `Failed to show tracing banner: ${err}`,
      },
    });
  }
}

async function CodeTracePlugin(ctx: any) {
  return {
    event: async (input: any) => {
      const event = input.event ?? input;

      // Surface the tracing reminder (ENABLED / PAUSED / inactive) to the USER
      // as a TUI toast when a root session is created. Subagent sessions fire
      // `session.created` too but carry a `parentID` — skip those so a Task
      // delegation doesn't pop a banner each time. Resumed sessions fire no
      // `session.created`; they fall through to the first idle below.
      if (event.type === "session.created") {
        const info = event.properties?.info;
        const sessionId = info?.id ?? event.properties?.sessionID;
        if (!sessionId || info?.parentID) return;
        await showBanner(ctx, sessionId, ctx.directory);
        return;
      }

      if (event.type !== "session.idle") return;

      const sessionId = event.sessionID ?? event.properties?.sessionID;
      if (!sessionId) {
        await ctx.client.app.log({
          body: {
            service: "code-trace",
            level: "warn",
            message: `session.idle event missing sessionID, keys: ${Object.keys(event).join(",")}`,
          },
        });
        return;
      }

      const cursor = loadCursor();
      const prev = cursor[sessionId];
      const startIndex = prev?.lastIndex ?? 0;

      let messagesResponse;
      try {
        messagesResponse = await ctx.client.session.messages({ path: { id: sessionId } });
      } catch (err) {
        await ctx.client.app.log({
          body: {
            service: "code-trace",
            level: "error",
            message: `Failed to fetch session messages: ${err}`,
          },
        });
        return;
      }

      const allMessages = messagesResponse.data;
      if (allMessages.length <= startIndex) return;

      const newMessages = allMessages.slice(startIndex);
      if (newMessages.length === 0) return;

      const lastMsg = newMessages[newMessages.length - 1];
      const lastId = lastMsg?.info?.id ?? String(startIndex + newMessages.length - 1);
      const agentVersion = getOpencodeVersion();

      const payload = {
        source: "opencode",
        sessionId,
        cwd: ctx.directory,
        messages: newMessages,
        agentVersion,
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
        await ctx.client.app.log({
          body: {
            service: "code-trace",
            level: "error",
            message: `Failed to spawn code-trace: ${err}`,
          },
        });
        return;
      }

      cursor[sessionId] = { lastIndex: allMessages.length, lastId };
      saveCursor(cursor);
    },
  };
}

export default { id: "code-trace", server: CodeTracePlugin };
