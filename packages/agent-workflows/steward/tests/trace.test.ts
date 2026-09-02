
import { beforeEach, afterEach, describe, expect, it } from "vitest";
import { makeTraceparent, otlpExportConfigured } from "../agent/lib/trace.js";

const traceId = (tp: string) => tp.split("-")[1];
const spanId = (tp: string) => tp.split("-")[2];

describe("makeTraceparent", () => {
  it("produces a well-formed W3C traceparent (version 00, sampled)", () => {
    expect(makeTraceparent("session-1")).toMatch(/^00-[0-9a-f]{32}-[0-9a-f]{16}-01$/);
  });

  it("derives a stable trace-id from the session id, so a session's calls group", () => {
    const a = makeTraceparent("session-abc");
    const b = makeTraceparent("session-abc");
    expect(traceId(a)).toBe(traceId(b));
  });

  it("mints a fresh span-id per call — each outbound request is its own span", () => {
    const a = makeTraceparent("session-abc");
    const b = makeTraceparent("session-abc");
    expect(spanId(a)).not.toBe(spanId(b));
  });

  it("gives different sessions different traces", () => {
    expect(traceId(makeTraceparent("session-a"))).not.toBe(traceId(makeTraceparent("session-b")));
  });
});

describe("otlpExportConfigured", () => {
  const KEYS = ["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "OTEL_SDK_DISABLED"] as const;
  let saved: Record<string, string | undefined>;

  beforeEach(() => {
    saved = {};
    for (const k of KEYS) saved[k] = process.env[k];
    for (const k of KEYS) delete process.env[k];
  });

  afterEach(() => {
    for (const k of KEYS) {
      if (saved[k] === undefined) delete process.env[k];
      else process.env[k] = saved[k];
    }
  });

  it("keeps the static traceparent when the kill switch silences export", () => {
    // With OTEL_SDK_DISABLED=true the provider never registers and undici never
    // injects — so if the connections ALSO dropped their static header here, outbound
    // MCP calls would carry no traceparent at all and the cross-service join key
    // would die exactly when an operator turns telemetry off.
    process.env.OTEL_EXPORTER_OTLP_ENDPOINT = "http://collector";
    process.env.OTEL_SDK_DISABLED = "true";
    expect(otlpExportConfigured()).toBe(false);
  });

  it("mirrors the provider bootstrap: a signal-specific endpoint counts as configured", () => {
    process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = "http://collector";
    expect(otlpExportConfigured()).toBe(true);
  });
});
