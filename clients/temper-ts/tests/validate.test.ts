import { describe, expect, it } from "vitest";

import { BearerToken, ClientCredentials } from "../src/credentials.js";
import { createTemperClient } from "../src/client.js";
import { isLoopback, requireEndpoint } from "../src/validate.js";

const ANY_CREDENTIALS = new BearerToken("not-a-real-token");

describe("requireEndpoint", () => {
  it("accepts an https origin with a path prefix", () => {
    expect(() => requireEndpoint("https://temperkb.io", "baseUrl")).not.toThrow();
    expect(() => requireEndpoint("https://temperkb.io/api", "baseUrl")).not.toThrow();
  });

  it("rejects anything that is not an absolute http(s) URL", () => {
    for (const bad of ["", "temperkb.io", "/api/relative", "ftp://temperkb.io"]) {
      expect(() => requireEndpoint(bad, "baseUrl")).toThrow(TypeError);
    }
  });

  it("rejects userinfo because it would ride in every error message", () => {
    expect(() => requireEndpoint("https://id:secret@temperkb.io", "baseUrl")).toThrow(/userinfo/);
  });

  it("rejects a query or fragment that the path join would bury", () => {
    expect(() => requireEndpoint("https://temperkb.io?audience=x", "baseUrl")).toThrow(/query/);
    expect(() => requireEndpoint("https://temperkb.io#section", "baseUrl")).toThrow(/fragment/);
  });

  it("rejects an embedded newline rather than letting the parser strip it", () => {
    expect(() => requireEndpoint("https://temperkb.io/\r\nx-auth", "baseUrl")).toThrow(/whitespace/);
    expect(() => requireEndpoint("https://temper\tkb.io", "baseUrl")).toThrow(/whitespace/);
  });

  it("allows plaintext to the loopback interface", () => {
    for (const url of [
      "http://localhost",
      "http://localhost:3000",
      "http://127.0.0.1",
      "http://127.255.42.42:8123", // the whole 127.0.0.0/8 block, not just .0.0.1
      "http://[::1]:0",
      "http://worker.localhost",
    ]) {
      expect(() => requireEndpoint(url, "baseUrl"), url).not.toThrow();
    }
  });

  it("refuses plaintext to anything else", () => {
    for (const url of ["http://temperkb.io", "http://10.0.0.5:8080", "http://192.168.1.10"]) {
      expect(() => requireEndpoint(url, "baseUrl"), url).toThrow(/non-loopback/);
    }
  });

  it("the opt-out is a keyword the caller has to write", () => {
    expect(() =>
      requireEndpoint("http://temperkb.io", "baseUrl", { allowInsecureHttp: true }),
    ).not.toThrow();
  });
});

describe("isLoopback", () => {
  it("names this machine by literal address or reserved name", () => {
    expect(isLoopback("localhost")).toBe(true);
    expect(isLoopback("LOCALHOST")).toBe(true); // hostname is lowercased by URL; direct calls are not
    expect(isLoopback("localhost.")).toBe(true); // one fully-qualified trailing dot
    expect(isLoopback("app.localhost")).toBe(true);
    expect(isLoopback("127.0.0.1")).toBe(true);
    expect(isLoopback("127.9.9.9")).toBe(true);
    expect(isLoopback("::1")).toBe(true);
    expect(isLoopback("[::1]")).toBe(true);
    expect(isLoopback("temperkb.io")).toBe(false);
    expect(isLoopback("10.0.0.1")).toBe(false);
    expect(isLoopback("localhost.example.com")).toBe(false); // .localhost as a SUFFIX of a longer name
    expect(isLoopback("127.0.0.2.example.com")).toBe(false);
  });
});

describe("the seams", () => {
  it("createTemperClient refuses a plaintext non-loopback origin at creation", () => {
    expect(() =>
      createTemperClient({ baseUrl: "http://temperkb.io", credentials: ANY_CREDENTIALS }),
    ).toThrow(/baseUrl is plaintext http/);
    expect(() =>
      createTemperClient({
        baseUrl: "http://temperkb.io",
        credentials: ANY_CREDENTIALS,
        allowInsecureHttp: true,
      }),
    ).not.toThrow();
  });

  it("ClientCredentials refuses a plaintext non-loopback token URL at construction", () => {
    expect(() =>
      new ClientCredentials({
        tokenUrl: "http://idp.example.com/oauth/token",
        clientId: "cid",
        clientSecret: "sec",
      }),
    ).toThrow(/tokenUrl is plaintext http/);
    expect(() =>
      new ClientCredentials({
        tokenUrl: "http://127.0.0.1:9999/oauth/token",
        clientId: "cid",
        clientSecret: "sec",
      }),
    ).not.toThrow();
  });
});
