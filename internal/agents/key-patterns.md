# Key Patterns

Invariants that are easy to break and not obvious from the code at hand. Each points at its
authority; read that before changing the thing it describes.

## Identity and addressing

- **IDs are UUIDv7.** Never abbreviate one: the leading bits are a timestamp, so neighbours share
  a prefix.
- **A ref resolves on its trailing UUID only.** `temper_workflow::operations::parse_ref` ignores
  the slug half (a stale slug is harmless) and has no by-slug lookup. Anything that accepts a new
  identifier form must check its other components before reaching this parser.
- **A context rename re-addresses it.** The slug is `sluggify(name)`, with no `--slug` (rejected),
  so every stored `@owner/slug` goes stale; the context UUID is the stable thing to store. Nothing
  reconciles the local vault, by decision: the old projection directory survives beside the new
  one, so check `temper context list`, not the directory. Refusal order and the no-op rule are
  documented on `context_service::rename`.

## Cloud and vault

- **The cloud is authoritative; the vault is a read-only projection.** Every write goes through
  the API. Removing a projected file is a cache miss with no server effect (`temper pull`
  restores it), and `show` writes no file.
- **CLI output is agent-first.** It resolves flag → env (`TEMPER_FORMAT`, `TEMPER_COLOR`,
  `NO_COLOR`) → `[cli]` config → tty-aware default, once, in `main`. Never emit raw ANSI; route all
  styling through `temper-cli/src/output/`.

## Authorization and disclosure

- **Machine principals are registered, never discovered.** `resolve_machine_from_claims` is
  lookup-or-401 against `kb_machine_clients`, in temper-services so the API and MCP cannot drift.
  `--owner-team` records ownership and is never read for authorization; reach is granted
  explicitly. Rotating the IdP application needs `temper admin machine rebind`. See
  `internal/auth/machine-token-contract.md`.
- **`Surface` is provenance, never an authorization input.** The `X-Temper-Surface` header is
  claimable by anyone by design, and `relay_trust` degrades rather than rejects. The day a gate,
  rate limit or route key reads `Surface` as trust, that becomes a vulnerability. Access is always
  `require_auth` on the caller's own bearer.
- **A returned value is a disclosure about every row that can move it.** The anchor gate decides
  whether there is an answer; the caller's read predicate decides which rows count; two doors onto
  one anchor never disagree about which rows exist. The only exception is a value about the
  anchor itself, stated at the field. Fold (`is_folded`, `is_current`) is currency, not
  visibility — but `is_active` is both, so check what a predicate carries before composing it into
  another question. The open set is task `01a03637-85a4-7e03-b3d0-969e36e05c54`.
- **A new authority that overrides `ScopedAuthority::denial_for` is caught by nothing.** The
  regression suite in `authz/mod.rs` covers the authorities that exist; this is a declared gap.

## Evidence and ingest

- **Evidential standing is three axes and a band, never one number.** Recomputed at read
  (`resource_standing_shape`); the memo columns are a write optimization, never the authority.
  Quality is over the audited subset only, so unaudited citations move coverage, not quality. The
  aggregation order — within an auditor, then across auditors weighted by freshest audit, then
  across sources — is what defeats actor-count inflation; do not flatten it. **Agents with partial
  visibility do not read stored aggregates**, and that covers `StandingShape` and the evidence
  doors: the components are reader-independent, bounded only by the finding's gate.
  See `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`.
- **Citation audits are append-only events** (they cannot be edges), with no supersession.
  `AuditAuthority` adds a self-audit denial to readability, and both denials render 404. The
  auditor needs **read-only** cogmap reach: `--cogmap` defaults to write, which makes every audit
  a self-audit.
- **`ingest_state = 'in_progress'` hides a resource from list and search, never from `show`.** The
  gate goes wherever `r.is_active` goes, in each search arm — never in `resources_visible_to`
  (authorization and completeness are different predicates).
- **Multi-block does not mean segmented.** Charters are multi-block and never finalize, so "many
  blocks and no finalize event" matches every cognitive map. There is no `ingest_state` backfill
  for that reason.
- **The L0 kernel map** (`system-default`, cogmap `00000000-0000-0000-0005-000000000001`) is public
  and joined to the root team, so every profile sees its telos — a visibility test that asserts an
  empty result is testing the kernel's existence. It evolves only through new additive migrations,
  never by editing its birth migration.

## Build, CI and release

- **"Docs-only" can lie.** Some `.md`/`.txt` files are compiled in (`include_str!`) or read as a
  gate's fixture; `test-detect-ci-scope.sh` holds the veto to the class. Never widen
  `NON_PRODUCT_ROOTS` beyond a tree proven inert. `ci-success` is the branch-protection gate. See
  `.github/workflows/CLAUDE.md`.
- **The skill ships twice.** `temper skill install` renders the CLI copy (per-user, never
  committed); `temper skill emit` renders the committed MCP projection under `skills/`, which is
  gated by `skills-drift` and must stay config-free. See the `generated-artifacts` skill.
- **Release verification is `verified` / `mismatch` / `unverifiable`, never collapsed.** The
  producer refuses to publish an archive without its manifest. The Sigstore trust root is compiled
  in, so a root rotation is a standing release obligation. Manifest path containment exists in
  Rust and in `install.sh`, both held to `scripts/install/containment-corpus.txt`. See
  `internal/development/releasing.md`.
