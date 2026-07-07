import { describe, expect, it } from "vitest";
import { type ClientRegistry, isRedirectUriAllowed } from "../../src/oauth/clients.js";

describe("isRedirectUriAllowed", () => {
  it("rejects an unregistered client_id", () => {
    const registry: ClientRegistry = { "temper-ui": ["https://app.example.com/auth/callback"] };
    expect(
      isRedirectUriAllowed(registry, "unknown-client", "https://app.example.com/auth/callback"),
    ).toBe(false);
  });

  describe("exact match (non-loopback clients)", () => {
    const registry: ClientRegistry = {
      "temper-ui": ["https://app.example.com/auth/callback"],
      "temper-mcp": [
        "https://claude.ai/api/mcp/auth_callback",
        "https://claude.com/api/mcp/auth_callback",
      ],
    };

    it("allows an exact allowlisted HTTPS redirect", () => {
      expect(
        isRedirectUriAllowed(registry, "temper-ui", "https://app.example.com/auth/callback"),
      ).toBe(true);
      expect(
        isRedirectUriAllowed(registry, "temper-mcp", "https://claude.ai/api/mcp/auth_callback"),
      ).toBe(true);
    });

    it("rejects a non-loopback redirect that is not exactly allowlisted", () => {
      // A different path — no port-flexible matching applies to HTTPS, so it must exact-match.
      expect(
        isRedirectUriAllowed(registry, "temper-ui", "https://app.example.com/other/callback"),
      ).toBe(false);
      // An attacker-controlled host must never match, even with an allowlisted path.
      expect(
        isRedirectUriAllowed(registry, "temper-ui", "https://evil.example.com/auth/callback"),
      ).toBe(false);
    });

    it("does not port-flex a non-loopback (HTTPS) redirect", () => {
      // Same host/path, different port — HTTPS gets no port flexibility (only loopback http does).
      expect(
        isRedirectUriAllowed(registry, "temper-ui", "https://app.example.com:8443/auth/callback"),
      ).toBe(false);
    });
  });

  describe("loopback port-flexible matching (native CLI clients)", () => {
    // Operator allowlists a port-less loopback entry; any ephemeral port on the same host/path matches.
    const registry: ClientRegistry = { "temper-mcp": ["http://127.0.0.1/callback"] };

    it("matches an ephemeral-port loopback URI against a port-less allowlist entry", () => {
      expect(isRedirectUriAllowed(registry, "temper-mcp", "http://127.0.0.1:53682/callback")).toBe(
        true,
      );
    });

    it("matches any loopback host against an allowlisted loopback entry (localhost <-> 127.0.0.1)", () => {
      // Chosen host-normalization decision (#2): any loopback host matches any loopback host.
      expect(isRedirectUriAllowed(registry, "temper-mcp", "http://localhost:5173/callback")).toBe(
        true,
      );
      const localhostRegistry: ClientRegistry = { "temper-mcp": ["http://localhost/callback"] };
      expect(
        isRedirectUriAllowed(localhostRegistry, "temper-mcp", "http://127.0.0.1:8080/callback"),
      ).toBe(true);
    });

    it("rejects a loopback URI whose path does not match", () => {
      expect(isRedirectUriAllowed(registry, "temper-mcp", "http://127.0.0.1:53682/evil")).toBe(
        false,
      );
    });

    it("rejects a non-loopback host even when the allowlist has a loopback entry", () => {
      // Port-flexible matching is loopback-only; a remote host must never borrow it.
      expect(
        isRedirectUriAllowed(registry, "temper-mcp", "http://attacker.example.com/callback"),
      ).toBe(false);
    });

    it("rejects an https loopback URI (loopback matching is http-only)", () => {
      // A registered client sending https://127.0.0.1 is not a native loopback callback; require exact.
      expect(isRedirectUriAllowed(registry, "temper-mcp", "https://127.0.0.1/callback")).toBe(
        false,
      );
    });

    it("rejects a malformed redirect URI", () => {
      expect(isRedirectUriAllowed(registry, "temper-mcp", "not-a-url")).toBe(false);
    });
  });
});
