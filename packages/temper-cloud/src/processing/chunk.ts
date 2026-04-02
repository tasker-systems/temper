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

interface RawSection {
  headerPath: string;
  lines: string[];
}

/**
 * Maximum tokens per chunk. bge-base-en-v1.5 accepts 512 tokens including
 * special tokens ([CLS] + [SEP] = 2), so usable budget is 510.
 */
const MAX_TOKENS = 510;

/** Conservative chars-per-token for mixed markdown (code/URLs tokenize at ~2.5-3.0). */
const CHARS_PER_TOKEN = 2.8;

/** Approximate max characters that fit within the token budget. */
const MAX_CHARS = Math.floor(MAX_TOKENS * CHARS_PER_TOKEN); // ~1785

const HEADING_RE = /^(#{1,6})\s+(.+)$/;
const BOLD_LINE_RE = /^\s*\*{1,2}([^*]+)\*{1,2}\s*$/;

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

// ---------------------------------------------------------------------------
// Section collection
// ---------------------------------------------------------------------------

function collectSections(text: string): RawSection[] {
  const lines = text.split("\n");
  const sections: RawSection[] = [];
  const headerStack: HeaderState[] = [];
  let currentLines: string[] = [];

  function flush() {
    if (currentLines.length === 0) return;
    const headerPath = headerStack.map((h) => h.text).join(" > ");
    sections.push({ headerPath, lines: [...currentLines] });
    currentLines = [];
  }

  for (const line of lines) {
    const m = line.match(HEADING_RE);
    if (m) {
      flush();
      const level = m[1].length;
      const text = m[2].trim();
      while (headerStack.length > 0 && headerStack[headerStack.length - 1].level >= level) {
        headerStack.pop();
      }
      headerStack.push({ level, text });
    } else {
      currentLines.push(line);
    }
  }

  flush();
  return sections;
}

// ---------------------------------------------------------------------------
// Splitting oversized sections
// ---------------------------------------------------------------------------

function splitParagraphs(lines: string[]): string[] {
  const paragraphs: string[] = [];
  let current: string[] = [];

  for (const line of lines) {
    if (line.trim() === "") {
      if (current.length > 0) {
        paragraphs.push(current.join("\n"));
        current = [];
      }
    } else {
      current.push(line);
    }
  }
  if (current.length > 0) {
    paragraphs.push(current.join("\n"));
  }
  return paragraphs;
}

function splitByLines(text: string): string[] {
  const chunks: string[] = [];
  let current: string[] = [];
  let currentLen = 0;

  for (const line of text.split("\n")) {
    const lineLen = line.length + (currentLen > 0 ? 1 : 0);
    if (currentLen > 0 && currentLen + lineLen > MAX_CHARS) {
      const content = current.join("\n").trim();
      if (content) chunks.push(content);
      current = [];
      currentLen = 0;
    }
    current.push(line);
    currentLen += lineLen;
  }

  const content = current.join("\n").trim();
  if (content) chunks.push(content);
  return chunks;
}

function splitOversized(lines: string[]): string[] {
  const fullText = lines.join("\n").trim();
  if (!fullText) return [];
  if (fullText.length <= MAX_CHARS) return [fullText];

  const paragraphs = splitParagraphs(lines);
  const chunks: string[] = [];
  let accumulator: string[] = [];
  let accLen = 0;

  for (const para of paragraphs) {
    const paraLen = para.length + (accLen > 0 ? 2 : 0); // +2 for \n\n join

    if (accLen > 0 && accLen + paraLen > MAX_CHARS) {
      const content = accumulator.join("\n\n").trim();
      if (content) chunks.push(content);
      accumulator = [];
      accLen = 0;
    }

    if (para.length > MAX_CHARS) {
      // Flush accumulator first.
      if (accumulator.length > 0) {
        const content = accumulator.join("\n\n").trim();
        if (content) chunks.push(content);
        accumulator = [];
        accLen = 0;
      }
      chunks.push(...splitByLines(para));
    } else {
      accumulator.push(para);
      accLen += paraLen;
    }
  }

  if (accumulator.length > 0) {
    const content = accumulator.join("\n\n").trim();
    if (content) chunks.push(content);
  }

  return chunks;
}

// ---------------------------------------------------------------------------
// Emphasis sub-heading splitting
// ---------------------------------------------------------------------------

function splitAtEmphasisSubheadings(lines: string[]): string[][] {
  const fullText = lines.join("\n");
  if (fullText.length <= MAX_CHARS || !lines.some((l) => BOLD_LINE_RE.test(l))) {
    return [lines];
  }

  const groups: string[][] = [];
  let current: string[] = [];

  for (const line of lines) {
    if (BOLD_LINE_RE.test(line) && current.length > 0) {
      groups.push([...current]);
      current = [];
    }
    current.push(line);
  }
  if (current.length > 0) {
    groups.push(current);
  }

  return groups;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export function chunkText(text: string): Chunk[] {
  if (!text.trim()) return [];

  const sections = collectSections(text);
  const chunks: Chunk[] = [];
  let chunkIndex = 0;

  for (const section of sections) {
    const subGroups = splitAtEmphasisSubheadings(section.lines);

    for (const group of subGroups) {
      const groupChunks = splitOversized(group);

      for (const content of groupChunks) {
        const trimmed = content.trim();
        if (!trimmed) continue;
        chunks.push({
          chunk_index: chunkIndex++,
          header_path: section.headerPath,
          content: trimmed,
          content_hash: sha256(trimmed),
        });
      }
    }
  }

  return chunks;
}
