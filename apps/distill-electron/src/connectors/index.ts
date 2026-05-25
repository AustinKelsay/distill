import { detectClaudeCodeSource } from "./claude_code/detect";
import { discoverClaudeCodeCaptures } from "./claude_code/discover";
import { parseClaudeCodeCapture } from "./claude_code/parse";
import { snapshotClaudeCodeCapture } from "./claude_code/snapshot";
import { detectCodexSource } from "./codex/detect";
import { discoverCodexCaptures } from "./codex/discover";
import { parseCodexCapture } from "./codex/parse";
import { snapshotCodexCapture } from "./codex/snapshot";
import { detectOpenCodeSource } from "./opencode/detect";
import { discoverOpenCodeCaptures } from "./opencode/discover";
import { parseOpenCodeCapture } from "./opencode/parse";
import { snapshotOpenCodeCapture } from "./opencode/snapshot";
import { SourceConnector } from "./types";

export const sourceConnectors: SourceConnector[] = [
  {
    kind: "codex",
    detect: detectCodexSource,
    discoverCaptures: discoverCodexCaptures,
    snapshotCapture: snapshotCodexCapture,
    parseCapture: parseCodexCapture
  },
  {
    kind: "claude_code",
    detect: detectClaudeCodeSource,
    discoverCaptures: discoverClaudeCodeCaptures,
    snapshotCapture: snapshotClaudeCodeCapture,
    parseCapture: parseClaudeCodeCapture
  },
  {
    kind: "opencode",
    detect: detectOpenCodeSource,
    discoverCaptures: discoverOpenCodeCaptures,
    snapshotCapture: snapshotOpenCodeCapture,
    parseCapture: parseOpenCodeCapture
  }
];
