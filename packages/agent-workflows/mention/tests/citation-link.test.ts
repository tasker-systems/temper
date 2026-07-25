import { describe, expect, it } from "vitest";

import {
  VAULT_RESOURCE_PATH,
  citationLinkInstruction,
  normalizeSiteBaseUrl,
} from "../agent/lib/citation-link.js";

describe("normalizeSiteBaseUrl", () => {
  it("passes a clean https base through unchanged", () => {
    expect(normalizeSiteBaseUrl("https://temperkb.io")).toBe("https://temperkb.io");
  });

  // FAILS IF: a trailing slash on the configured base survives, which would double the slash
  // in `${base}/vault/r/...` and produce a URL Slack still links but that reads as broken.
  it("strips trailing slashes so the vault path never doubles one", () => {
    expect(normalizeSiteBaseUrl("https://temperkb.io/")).toBe("https://temperkb.io");
    expect(normalizeSiteBaseUrl("https://temperkb.io///")).toBe("https://temperkb.io");
  });

  it("trims surrounding whitespace", () => {
    expect(normalizeSiteBaseUrl("  https://temper.acme.com  ")).toBe("https://temper.acme.com");
  });

  // FAILS IF: an unset/blank base is treated as a real URL. The null return is what routes
  // `citationLinkInstruction` to its link-less fallback instead of emitting `</vault/r/...>`.
  it("returns null for absent or blank values", () => {
    expect(normalizeSiteBaseUrl(undefined)).toBeNull();
    expect(normalizeSiteBaseUrl(null)).toBeNull();
    expect(normalizeSiteBaseUrl("")).toBeNull();
    expect(normalizeSiteBaseUrl("   ")).toBeNull();
  });
});

describe("citationLinkInstruction", () => {
  it("binds {BASE} to the deployment's site URL and gives the full link shape", () => {
    const md = citationLinkInstruction("https://temperkb.io");
    expect(md).toContain("https://temperkb.io");
    // The exact clickable form the model must emit, host substituted, path verbatim.
    expect(md).toContain(`<https://temperkb.io${VAULT_RESOURCE_PATH}{ref}|{title}>`);
  });

  // FAILS IF: the instruction is host-hardcoded. A self-hosted deployment must get its OWN host
  // in the rule, since the committed instructions.md carries only the {BASE} placeholder.
  it("uses whatever base it is handed, not a hardcoded temperkb.io", () => {
    const md = citationLinkInstruction("https://temper.acme.com");
    expect(md).toContain("https://temper.acme.com/vault/r/{ref}|{title}");
    expect(md).not.toContain("temperkb.io");
  });

  it("normalizes a trailing slash before building the link shape", () => {
    const md = citationLinkInstruction("https://temperkb.io/");
    expect(md).toContain("https://temperkb.io/vault/r/{ref}|{title}");
    expect(md).not.toContain("temperkb.io//vault");
  });

  // FAILS IF: an absent base still emits a link template — the model would then invent or emit a
  // broken host. The fallback must tell it to cite by title and skip the link, never guess a URL.
  it("falls back to a link-less, no-invention instruction when the base is absent", () => {
    const md = citationLinkInstruction(undefined);
    expect(md).not.toContain(VAULT_RESOURCE_PATH);
    expect(md.toLowerCase()).toContain("title");
    expect(md.toLowerCase()).toContain("never guess or invent");
  });
});
