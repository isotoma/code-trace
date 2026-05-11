import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

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

export default async function CodeTracePlugin(ctx: {
  project?: unknown;
  client: {
    session: {
      messages: (opts: { path: { id: string } }) => Promise<{
        data: Array<{ info: { id: string; role: string; model?: string }; parts: unknown[] }>;
      }>;
    };
    app: {
      log: (opts: { body: { service: string; level: string; message: string; extra?: unknown } }) => Promise<boolean>;
    };
  };
  directory: string;
  worktree: string;
  $: {
    command: (cmd: string, args?: string[]) => { stdout: { text: () => string } };
  };
}) => {
  return {
    event: async (event: { type: string; properties?: Record<string, unknown> }) => {
      if (event.type !== "session.idle") return;

      const sessionId = event.properties?.sessionID as string | undefined;
      if (!sessionId) {
        await ctx.client.app.log({
          body: {
            service: "code-trace",
            level: "warn",
            message: "session.idle event missing sessionID",
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
        child.stdin?.destroy();
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
