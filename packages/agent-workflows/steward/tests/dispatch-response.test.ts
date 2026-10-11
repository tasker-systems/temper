import { afterEach, describe, expect, it, vi } from "vitest";

const { temperFetchMock, auditorFetchMock } = vi.hoisted(() => ({
  temperFetchMock: vi.fn(),
  auditorFetchMock: vi.fn(),
}));

// The schedules are code `run` handlers: eve's `defineSchedule` returns the definition unchanged,
// and the channels the fan-out targets are opaque first arguments to the mocked `to`.
vi.mock("eve/schedules", () => ({
  defineSchedule: <T>(definition: T) => definition,
}));
vi.mock("../agent/channels/worker.js", () => ({ default: {} }));
vi.mock("../agent/channels/auditor-worker.js", () => ({ default: {} }));
vi.mock("../agent/lib/optional-agent.js", () => ({
  AUDITOR_ENABLED: "TEMPER_AUDITOR_ENABLED",
  agentEnabled: () => true,
  tokenIssuanceUnavailable: () => false,
}));
vi.mock("../agent/lib/temper-auth.js", () => ({
  requireEndpointEnv: (name: string) => {
    if (name === "TEMPER_API_URL") return "https://temper.test";
    throw new Error(`Missing required environment variable: ${name}`);
  },
  temperFetch: temperFetchMock,
  auditorFetch: auditorFetchMock,
  AUDITOR_CREDENTIALS: {},
  credentialConfigured: () => true,
}));

import auditorDispatch from "../agent/schedules/auditor.js";
import stewardDispatch from "../agent/schedules/steward.js";

/** A `to(channel, target)` mock whose returned handle records each `send(message, options)`. */
function fanOut() {
  const send = vi.fn(async (_message: string, _options: { auth: unknown }) => ({}));
  const to = vi.fn((_channel: unknown, _target: unknown) => ({ send }));
  return { to, send };
}

/** Handler args whose `waitUntil` parks the tick's work until `awaited()` is called. */
function tickArgs(to: ReturnType<typeof fanOut>["to"]) {
  const pending: Promise<unknown>[] = [];
  return {
    args: {
      to,
      waitUntil: (task: Promise<unknown>) => {
        pending.push(task);
      },
      appAuth: {},
    } as unknown as Parameters<typeof stewardDispatch.run>[0],
    awaited: () => Promise.all(pending),
  };
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("steward dispatch response boundary", () => {
  it("fans out one session per claimed job on a well-formed response", async () => {
    const { to, send } = fanOut();
    const { args, awaited } = tickArgs(to);
    temperFetchMock.mockResolvedValue(
      new Response(
        JSON.stringify({
          claimed: [{ id: "job-1", cogmap_id: "map-1", attempts: 1 }],
          correlation_id: "tick-1",
        }),
        { status: 200 },
      ),
    );

    await stewardDispatch.run(args);
    await awaited();

    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0][0]).toContain("map-1");
  });

  it("refuses a body without a claimed list at the boundary instead of fanning out", async () => {
    const { to } = fanOut();
    const { args, awaited } = tickArgs(to);
    temperFetchMock.mockResolvedValue(new Response(JSON.stringify({}), { status: 200 }));

    await stewardDispatch.run(args);
    await expect(awaited()).rejects.toThrow(
      "steward dispatch returned an unrecognized response",
    );
    expect(to).not.toHaveBeenCalled();
  });
});

describe("auditor dispatch response boundary", () => {
  const CLAIMED_JOB = {
    id: "job-1",
    cogmap_id: "map-1",
    attempts: 1,
    citations: [{ finding_id: "finding-1", block_id: "block-1", source_id: "source-1" }],
  };

  it("fans out one audit session per claimed job on a well-formed response", async () => {
    const { to, send } = fanOut();
    const { args, awaited } = tickArgs(to);
    auditorFetchMock.mockResolvedValue(
      new Response(JSON.stringify({ claimed: [CLAIMED_JOB] }), { status: 200 }),
    );

    await auditorDispatch.run(args);
    await awaited();

    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0][0]).toContain("finding-1");
  });

  it("refuses a body without a claimed list at the boundary instead of fanning out", async () => {
    const { to } = fanOut();
    const { args, awaited } = tickArgs(to);
    auditorFetchMock.mockResolvedValue(new Response(JSON.stringify({}), { status: 200 }));

    await auditorDispatch.run(args);
    await expect(awaited()).rejects.toThrow(
      "auditor dispatch returned an unrecognized response",
    );
    expect(to).not.toHaveBeenCalled();
  });

  it("refuses a claimed job whose citations are not citation-shaped", async () => {
    const { to } = fanOut();
    const { args, awaited } = tickArgs(to);
    auditorFetchMock.mockResolvedValue(
      new Response(
        JSON.stringify({
          claimed: [{ ...CLAIMED_JOB, citations: [{ finding_id: "finding-1" }] }],
        }),
        { status: 200 },
      ),
    );

    await auditorDispatch.run(args);
    await expect(awaited()).rejects.toThrow(
      "auditor dispatch returned an unrecognized response",
    );
    expect(to).not.toHaveBeenCalled();
  });
});
