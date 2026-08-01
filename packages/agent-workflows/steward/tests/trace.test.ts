import { describe, expect, it } from "vitest";

import { makeTraceparent } from "../agent/lib/trace.js";

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
