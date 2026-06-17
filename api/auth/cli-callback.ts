/**
 * CLI auth callback relay (Vercel entry point). Thin wrapper — relay logic and
 * tests live in `packages/temper-cloud/src/cli-callback.ts`.
 */

import { buildCliCallbackResponse } from "../../packages/temper-cloud/src/cli-callback.js";

export function GET(req: Request): Response {
  return buildCliCallbackResponse(req.url, req.headers.get("host"));
}
