# Nav render harness (`/dev/nav`)

A **dev-only** route that renders the real `Sidebar` against inline fixtures —
**no auth, no server reads, no merge-to-prod**. Vercel previews can't carry Auth0, so
the authenticated nav was otherwise only observable in prod post-merge (see the
`reference_vercel_preview_no_auth0_verify_in_prod` memory). Same gap `/dev/atlas`
exists to close, same shape of answer.

The route `throw error(404)`s outside `dev`, so it is inert in any deployed build.

## Running

```bash
cd packages/temper-ui
bun run dev
# open http://localhost:5173/dev/nav
```

## Why the fixtures are inline, not captured

`/dev/atlas` captures from a live database because its payloads are large, deeply
nested, and shaped by real corpus statistics that hand-authoring cannot honestly
guess. The nav reads two small flat lists — `ContextRowWithCounts` and `TeamRow` —
whose every field is in the type. There is nothing a capture would tell us that the
type does not, and nothing personal to sanitize.

## What the scenarios pin

| Scenario | Shape it pins |
|---|---|
| `groups` | Both reads answered. Includes a team the reader belongs to that holds **no readable place** (renders as an empty group) and a team-owned place readable **without membership** (renders with no display name, since `/api/teams` never mentions it) |
| `no-teams-read` | The teams read failed: labels degrade to the bare slug and the empty group drops. **No place is lost** — places come from `/api/contexts`, which the grouping is keyed on |
| `empty` | The read answered with nothing |
| `unavailable` | The context read failed — distinct from `empty`, because a nav with nothing in it claims the reader belongs to nothing |

## What it does NOT exercise

**Active-place marking.** `isContextLocation` reads the real route params, which this
harness route does not carry, so no place is ever lit here — including the
collapsed-group-holds-the-active-place mark on a group heading. That path stays
prod-verified.
