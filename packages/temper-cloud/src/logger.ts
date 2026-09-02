import pino from "pino";

/**
 * Bound-field backstop: a log call that binds an object carrying one of these keys has the
 * value replaced with the censor before serialization, wherever the key sits on the object.
 * Bare names and their one-level wildcards are both listed because call sites on this surface
 * bind flat shorthand objects (`{ token }`) as often as nested ones, and the AS signing env
 * names appear in both cases so a binding is censored no matter which form it inherited its
 * name from. A path that matches nothing costs nothing, so the list is written for the
 * bindings a future call site might create, not only the ones on disk today.
 *
 * Scope: this governs structured fields only. Text a call site interpolates into a message is
 * outside its reach, and the list is never a substitute for deciding, per line, what the line
 * should carry.
 */
export const REDACT_PATHS: string[] = [
  "token",
  "*.token",
  "access_token",
  "*.access_token",
  "refresh_token",
  "*.refresh_token",
  "secret",
  "*.secret",
  "client_secret",
  "*.client_secret",
  "authorization",
  "*.authorization",
  "cookie",
  "*.cookie",
  "AS_SIGNING_KEY_PKCS8",
  "*.AS_SIGNING_KEY_PKCS8",
  "as_signing_key_pkcs8",
  "*.as_signing_key_pkcs8",
  "AS_SIGNING_KID",
  "*.AS_SIGNING_KID",
];

export const loggerOptions = {
  level: process.env.LOG_LEVEL || "info",
  redact: {
    paths: REDACT_PATHS,
    censor: "[Redacted]",
  },
};

export const logger = pino(loggerOptions);
