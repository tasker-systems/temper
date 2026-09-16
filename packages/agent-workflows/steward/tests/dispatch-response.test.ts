import { afterEach, describe, expect, it, vi } from "vitest";

const { temperFetchMock, auditorFetchMock } = vi.hoisted(() => ({
  temperFetchMock: vi.fn(),
  auditorFetchMock: vi.fn(),
}));

// The schedules are code `run` handlers: eve's `defineSchedule` returns the definition unchanged,
// and the channels the fan-out targets are opaque first arguments to the mocked `receive`.
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
  requireEnv: (name: string) => {
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

type ReceiveMock = ReturnType<
  typeof vi.fn<(channel: unknown, input: { message: string }) => Promise<unknown>>
>;

/** Handler args whose `waitUntil` parks the tick's work until `awaited()` is called. */
function tickArgs(receive: ReceiveMock) {
  const pending: Promise<unknown>[] = [];
  return {
    args: {
      receive,
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
    const receive = vi.fn(async (_channel: unknown, input: { message: string }) => ({}));
    const { args, awaited } = tickArgs(receive);
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

    expect(receive).toHaveBeenCalledTimes(1);
    expect(receive.mock.calls[0][1].message).toContain("map-1");
  });

  it("refuses a body without a claimed list at the boundary instead of fanning out", async () => {
    const receive = vi.fn(async (_channel: unknown, input: { message: string }) => ({}));
    const { args, awaited } = tickArgs(receive);
    temperFetchMock.mockResolvedValue(new Response(JSON.stringify({}), { status: 200 }));

    await stewardDispatch.run(args);
    await expect(awaited()).rejects.toThrow(
      "steward dispatch returned an unrecognized response",
    );
    expect(receive).not.toHaveBeenCalled();
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
    const receive = vi.fn(async (_channel: unknown, input: { message: string }) => ({}));
    const { args, awaited } = tickArgs(receive);
    auditorFetchMock.mockResolvedValue(
      new Response(JSON.stringify({ claimed: [CLAIMED_JOB] }), { status: 200 }),
    );

    await auditorDispatch.run(args);
    await awaited();

    expect(receive).toHaveBeenCalledTimes(1);
    expect(receive.mock.calls[0][1].message).toContain("finding-1");
  });

  it("refuses a body without a claimed list at the boundary instead of fanning out", async () => {
    const receive = vi.fn(async (_channel: unknown, input: { message: string }) => ({}));
    const { args, awaited } = tickArgs(receive);
    auditorFetchMock.mockResolvedValue(new Response(JSON.stringify({}), { status: 200 }));

    await auditorDispatch.run(args);
    await expect(awaited()).rejects.toThrow(
      "auditor dispatch returned an unrecognized response",
    );
    expect(receive).not.toHaveBeenCalled();
  });

  it("refuses a claimed job whose citations are not citation-shaped", async () => {
    const receive = vi.fn(async (_channel: unknown, input: { message: string }) => ({}));
    const { args, awaited } = tickArgs(receive);
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
    expect(receive).not.toHaveBeenCalled();
  });
});
