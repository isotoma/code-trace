import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";

interface CursorEntry {
  lastIndex: number;
}

interface CursorStore {
  [sessionId: string]: CursorEntry;
}

function getCursorPath(): string {
  const home = process.env.HOME ?? process.env.USERPROFILE ?? "~";
  return join(home, ".local", "share", "code-trace", "pi_agent_cursor.json");
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

export default function (pi: any) {
  pi.on("agent_end", async (event: any, ctx: any) => {
    const allEntries: any[] = ctx.sessionManager.getEntries();
    const sessionId: string | undefined = allEntries[0]?.id;
    if (!sessionId) return;

    const cursor = loadCursor();
    const startIndex = cursor[sessionId]?.lastIndex ?? 0;

    if (allEntries.length <= startIndex) return;

    const newEntries = allEntries.slice(startIndex);
    if (newEntries.length === 0) return;

    let agentVersion: string | undefined;
    try {
      const result = await pi.exec("pi", ["--version"], { signal: ctx.signal });
      agentVersion = result.stdout?.trim() || undefined;
    } catch {
      // ignore
    }

    const payload = {
      source: "pi-agent",
      sessionId,
      cwd: ctx.cwd,
      messages: newEntries,
      agentVersion,
    };

    const binPath = process.env.CODE_TRACE_BIN ?? "code-trace";

    const child = spawn(binPath, [], {
      stdio: ["pipe", "ignore", "ignore"],
      detached: true,
      shell: true,
    });

    child.stdin?.end(JSON.stringify(payload), "utf-8");
    child.stdin?.destroy();
    child.unref();

    cursor[sessionId] = { lastIndex: allEntries.length };
    saveCursor(cursor);
  });
}
