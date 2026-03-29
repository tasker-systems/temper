import { createHash } from "node:crypto";

export interface Chunk {
  chunk_index: number;
  header_path: string;
  content: string;
  content_hash: string;
}

interface HeaderState {
  level: number;
  text: string;
}

export function chunkText(text: string): Chunk[] {
  if (!text.trim()) return [];

  const lines = text.split("\n");
  const chunks: Chunk[] = [];
  const headerStack: HeaderState[] = [];
  let currentContent: string[] = [];
  let chunkIndex = 0;

  function flushChunk() {
    const content = currentContent.join("\n").trim();
    if (!content) return;

    const headerPath = headerStack.map((h) => h.text).join(" > ");
    chunks.push({
      chunk_index: chunkIndex++,
      header_path: headerPath,
      content,
      content_hash: createHash("sha256").update(content).digest("hex"),
    });
    currentContent = [];
  }

  for (const line of lines) {
    const headerMatch = line.match(/^(#{1,6})\s+(.+)$/);

    if (headerMatch) {
      flushChunk();

      const level = headerMatch[1].length;
      const text = headerMatch[2].trim();

      // Pop headers at same or deeper level
      while (headerStack.length > 0 && headerStack[headerStack.length - 1].level >= level) {
        headerStack.pop();
      }
      headerStack.push({ level, text });
    } else {
      currentContent.push(line);
    }
  }

  flushChunk();
  return chunks;
}
