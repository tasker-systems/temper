# The rendering approach — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or
> `superpowers:executing-plans` to implement this plan task-by-task — **as modified by this repo's
> `hybrid-execution` skill**, which overrides the default per-task reviewer dispatch. Steps use
> checkbox (`- [ ]`) syntax.

**Goal:** Every interaction acknowledges itself immediately, and every region of the screen says
which of four states it is in — arriving, present, empty, or failed — so none of them can be
mistaken for another.

**Architecture:** Two mechanisms, split by clause rather than by route. **Navigation feedback** at
the layout serves `every-act-is-acknowledged` across all twelve routes. **Streaming from the server
load** (`{#await}` over unawaited promises) serves the rest, and only on the two route shapes that
have a remainder to stream. A shared four-state region primitive is the vocabulary both use.

**Tech Stack:** SvelteKit 2.x, Svelte 5 runes, Vitest + `@testing-library/svelte`, Biome.

**Spec:** [`internal/superpowers/specs/2026-08-21-the-rendering-approach-design.md`](../specs/2026-08-21-the-rendering-approach-design.md)

**Register:** [The surface tells the reader what it is doing](./01a02654-a920-7123-9c24-328e50022ffe)

---

## Global Constraints

Every task's requirements implicitly include these.

- **Every streamed promise carries `.catch()` attached at creation.** An unawaited promise that
  rejects **crashes the server** with an unhandled rejection. SvelteKit's own `fetch` is handled;
  every read in this codebase is a manually created promise and is **not**. This is a *different
  mechanism* from the `{:catch}` that renders the failure — having one does not give you the other.
  (Spec §5.3.)
- **The `await` goes last in a load.** *"Make sure the `await` happens at the end, otherwise we
  can't start loading comments until we've loaded the post."*
- **The four states must differ by more than one channel** — words, not only colour; a marker, not
  only opacity. A test asserting two renders differ is satisfied by one pixel; the clause is about
  what a reader can resolve. (Spec §3.3.)
- **A pending region carries text**, so it reaches the accessibility tree. Never a bare shimmer.
- **A failed region names what failed** — "history unavailable", never "something went wrong".
- **Never degrade a failed read to `null` or `[]`.** That asserts "there is nothing here", which is
  a claim about the reader's material that nothing verified.
- **Probe every contract test.** Break the exact invariant asserted, confirm the failure lands in
  the right direction, restore. A probe script must `assert` its own pattern matched before running —
  a probe that silently does not apply reports green and reads as "this test cannot fail".
- **No API call changes in this plan.** Phase 2 owns the reads.
- Run tests from `packages/temper-ui`: `bunx vitest run --project unit <path>` /
  `--project component <path>`. Do not run the workspace Rust suites; nothing here is Rust.
- Verify with `bunx svelte-check --threshold error` and `bunx biome check src`.
  **Baselines, measured 2026-08-21 — earlier text in this plan stated them wrongly twice, so these
  are the ones that were actually run:**
  - `bunx svelte-check --threshold error` → **0 errors, 0 warnings.** Any error is yours.
  - `bunx biome check src --diagnostic-level=error` → **0 errors.** Any error is yours.
  - `bunx biome check src` → **~47 pre-existing warnings**, all `noNonNullAssertion` across the
    test files, plus one `noUnusedFunctionParameters` at `src/lib/server/proxy.test.ts:265`. These
    are **warnings, not errors**. Do not fix them; do not count them as yours.
  - `biome.json` scopes to `src/**/*.ts`, so **`.svelte` files are neither linted nor formatted**
    by it, and no pre-commit hook covers them.
- **No API call changes in this plan.** Phase 2 owns the reads.
- Run tests from `packages/temper-ui`: `bunx vitest run --project unit <path>` /
  `--project component <path>`. Do not run the workspace Rust suites; nothing here is Rust.
- Verify with `bunx svelte-check --threshold error` and `bunx biome check src`.
  **Baselines, corrected 2026-08-21 — the original text attributed the error to the wrong tool,
  which would have masked a real one:**
  - `svelte-check --threshold error` → **0 errors**. Any error is yours.
  - `biome check src` → **1 pre-existing error** (`src/lib/server/proxy.test.ts:265`,
    `noUnusedFunctionParameters`) and **~50 pre-existing `noNonNullAssertion` warnings** across
    the test files. Do not fix either; do not count them as yours.
  - `biome.json` scopes to `src/**/*.ts`, so **`.svelte` files are not linted or formatted** by it
    and no pre-commit hook covers them.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/lib/components/RegionState.svelte` | **New.** The four-state vocabulary — the single place arriving / empty / failed are given their appearance and words |
| `src/lib/components/RegionState.component.test.ts` | **New.** C2 and C4, including the differential test |
| `src/lib/components/NavProgress.svelte` | **New.** Navigation feedback (C5) |
| `src/lib/components/NavProgress.component.test.ts` | **New.** C5 |
| `src/routes/(app)/+layout.svelte` | Modify — mount `NavProgress` |
| `src/test/app-context.ts` | Modify — export a controllable `navigating` |
| `src/lib/server/bounded.ts` | **New.** The refusal — a bounded wait that fails as "gave up" rather than hanging |
| `src/lib/server/bounded.test.ts` | **New.** |
| `src/routes/(app)/vault/r/[ident]/+page.server.ts` | Modify — stream `content`, `trail`, `edges`; stop degrading to `null`/`[]` |
| `src/routes/(app)/vault/r/[ident]/+page.svelte` | Modify — `{#await}` the three streamed regions |
| `src/routes/(app)/vault/r/[ident]/page.server.test.ts` | **New.** The route-level guard |
| `src/routes/(app)/graph/[owner]/+page.server.ts` | Modify — stream the rail's two reads |
| `src/lib/components/graph/NodeRail.svelte` | Modify — `{#await}` excerpt and trail |
| `src/routes/(app)/graph/[owner]/+page.server.ts` | Modify — stream the model and the readout |
| `src/routes/(app)/graph/[owner]/analysis/+page.server.ts` | Modify — stream the measurements |

**Why a shared `RegionState` rather than per-page markup:** the negative-face clause is about states
never presenting alike. If each page spells its own pending and failed states, they drift, and the
clause is enforced by nobody. One component means the four states are defined once and every
consumer inherits the distinction.

---

## Task 1: Navigation feedback

Serves `every-act-is-acknowledged` on **all twelve routes** — including the eight that gain nothing
from streaming. It is first because it is the smallest change with the widest reach.

**Files:**
- Create: `src/lib/components/NavProgress.svelte`
- Create: `src/lib/components/NavProgress.component.test.ts`
- Modify: `src/routes/(app)/+layout.svelte`
- Modify: `src/test/app-context.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `NavProgress` (no props). `app-context.ts` gains
  `export const navigating` (a store with `.subscribe`, mirroring the existing `page` mock at
  `src/test/app-context.ts:35`) and `export function setNavigating(target: { to?: { url: URL } } | null): void`.

- [ ] **Step 1: Extend the test harness with a controllable `navigating`**

`src/test/app-context.ts` already mocks `$app/stores` for `page` — follow that exact shape. Add a
writable-backed `navigating` whose value is `null` when idle, and a `setNavigating` helper. Add
`setNavigating(null)` to the existing `resetAppContext()` (`src/test/app-context.ts:72`) so tests do
not leak state into each other.

- [ ] **Step 2: Write the failing test**

```ts
import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { resetAppContext, setNavigating } from '../../test/app-context';
import NavProgress from './NavProgress.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));

describe('navigation is acknowledged', () => {
  beforeEach(() => resetAppContext());

  it('C5: shows nothing while idle', () => {
    const { container } = render(NavProgress);
    expect(container.querySelector('[data-testid="nav-progress"]')).toBeNull();
  });

  it('C5: acknowledges a navigation the moment it starts', () => {
    setNavigating({ to: { url: new URL('http://localhost/vault/all') } });
    const { container } = render(NavProgress);
    const el = container.querySelector('[data-testid="nav-progress"]');
    expect(el).not.toBeNull();
    // Carries words, not only a visual bar — the accessibility tree is the point.
    expect(el?.textContent?.trim()).not.toBe('');
  });
});
```

- [ ] **Step 3: Run it and confirm it fails**

Run: `bunx vitest run --project component src/lib/components/NavProgress.component.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 4: Implement `NavProgress.svelte`**

Read `navigating` from `$app/stores` and render the indicator only when it is non-null. It must
carry a text label (an `aria-live` region or visually-hidden text) alongside whatever visual
treatment you choose. Follow the layout's Tailwind idiom (`src/routes/(app)/+layout.svelte:30`),
not the graph components' scoped `<style>`.

- [ ] **Step 5: Run and confirm both tests pass**

- [ ] **Step 6: Mount it in the app layout**

`src/routes/(app)/+layout.svelte` — import and render inside the top-level `div`.

- [ ] **Step 7: Probe**

Make `NavProgress` render unconditionally. Confirm the idle test fails. Restore. The probe script
must `assert` its edit applied before running the suite.

- [ ] **Step 8: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run && bunx svelte-check --threshold error
git add src/lib/components/NavProgress.svelte src/lib/components/NavProgress.component.test.ts \
        "src/routes/(app)/+layout.svelte" src/test/app-context.ts
git commit -m "feat(ui): a navigation acknowledges itself before its data arrives"
```

---

## Task 2: The four-state region vocabulary

The primitive every later task consumes. Serves `arriving-and-settled-are-distinguishable` and
`no-two-region-states-present-alike`.

**Files:**
- Create: `src/lib/components/RegionState.svelte`
- Create: `src/lib/components/RegionState.component.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `RegionState`, with props
  `{ state: 'arriving' | 'empty' | 'failed', label: string, children?: Snippet }`.
  `label` names the region ("history", "excerpt", "connections") and is composed into the words for
  every state — so a failure reads *"history unavailable"* and an arrival reads *"loading history"*,
  from one source. Later tasks pass only `state` and `label`.

- [ ] **Step 1: Write the failing tests, including the differential one**

```ts
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import RegionState from './RegionState.svelte';

const html = (state: 'arriving' | 'empty' | 'failed') =>
  render(RegionState, { props: { state, label: 'history' } }).container.innerHTML;

describe('the four-state vocabulary', () => {
  it('C2: an arriving region says so, in words', () => {
    const el = render(RegionState, { props: { state: 'arriving', label: 'history' } }).container;
    expect(el.textContent?.toLowerCase()).toContain('history');
    expect(el.textContent?.trim()).not.toBe('');
  });

  it('C2: a failed region names WHAT failed, not "something went wrong"', () => {
    const el = render(RegionState, { props: { state: 'failed', label: 'history' } }).container;
    expect(el.textContent?.toLowerCase()).toContain('history');
    expect(el.textContent?.toLowerCase()).not.toContain('something went wrong');
  });

  // C4 — the differential test. It asserts the three states are pairwise unlike, without
  // asserting what any of them looks like, so it survives a redesign of all three.
  it('C4: no two states present alike', () => {
    const [arriving, empty, failed] = [html('arriving'), html('empty'), html('failed')];
    expect(arriving).not.toBe(empty);
    expect(empty).not.toBe(failed);
    expect(arriving).not.toBe(failed);
  });

  // The clause is about what a READER can resolve; markup differing by one attribute is not that.
  it('C4: the states differ in their WORDS, not only their styling', () => {
    const text = (s: 'arriving' | 'empty' | 'failed') =>
      render(RegionState, { props: { state: s, label: 'history' } }).container.textContent?.trim();
    const [a, e, f] = [text('arriving'), text('empty'), text('failed')];
    expect(new Set([a, e, f]).size).toBe(3);
  });
});
```

- [ ] **Step 2: Run and confirm they fail**

Run: `bunx vitest run --project component src/lib/components/RegionState.component.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `RegionState.svelte`**

Three branches on `state`, each rendering `data-testid="region-{state}"`, a distinct sentence built
from `label`, and a distinct visual treatment. The `present` state is not a branch here — it is the
consumer's own content, rendered via `children`.

- [ ] **Step 4: Run and confirm all four pass**

- [ ] **Step 5: Probe — this is the one that matters**

Make `failed` render the same markup as `arriving`. Confirm **both** C4 tests fail. Restore. Then
make them differ only by a CSS class with identical text; confirm the words test fails while the
markup test passes — that is the gap §3.3 describes, demonstrated rather than argued.

- [ ] **Step 6: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run && bunx svelte-check --threshold error
git add src/lib/components/RegionState.svelte src/lib/components/RegionState.component.test.ts
git commit -m "feat(ui): four states, four appearances — the region vocabulary"
```

---

## Task 3: The refusal — a bounded wait

Closes the declared hole in `working-and-stopped-are-distinguishable`. Without it, **a read that
never answers presents as arriving forever**, which is that clause's exact failure mode.

> **The duration is ruled.** `[ruled — 2026-08-21, Pete]` **8 seconds**, on the reasoning that
> *"we have to bound it somewhere."* The register still specifies none and never will — a budget is
> a build decision, not an invariant — so this changes no clause. `ms` stays a parameter.

**Files:**
- Create: `src/lib/server/bounded.ts`
- Create: `src/lib/server/bounded.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `export class GaveUp extends Error` (carries `label: string`), and
  `export function bounded<T>(p: Promise<T>, label: string, ms?: number): Promise<T>` — resolves as
  `p` does, rejects with `GaveUp` if `ms` elapses first, and **attaches a `.catch()` to `p`
  internally** so a late rejection cannot become an unhandled rejection after the bound fires.

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it, vi } from 'vitest';
import { GaveUp, bounded } from './bounded';

describe('a read the system stops waiting for', () => {
  it('resolves normally when the read answers in time', async () => {
    await expect(bounded(Promise.resolve('ok'), 'history', 50)).resolves.toBe('ok');
  });

  it('rejects with a NAMED give-up, so the region can say which read stopped', async () => {
    vi.useFakeTimers();
    const never = new Promise<string>(() => {});
    const p = bounded(never, 'history', 1000);
    vi.advanceTimersByTime(1001);
    await expect(p).rejects.toBeInstanceOf(GaveUp);
    await expect(p).rejects.toMatchObject({ label: 'history' });
    vi.useRealTimers();
  });

  it('a real failure still surfaces as itself, not as a give-up', async () => {
    const boom = Promise.reject(new Error('503'));
    await expect(bounded(boom, 'history', 50)).rejects.not.toBeInstanceOf(GaveUp);
  });

  // The trap, tested: a rejection arriving AFTER the bound fired must not become an
  // unhandled rejection and take the server down.
  it('swallows a late rejection rather than crashing the process', async () => {
    vi.useFakeTimers();
    let reject!: (e: Error) => void;
    const late = new Promise<string>((_, r) => { reject = r; });
    const p = bounded(late, 'history', 10);
    vi.advanceTimersByTime(11);
    await expect(p).rejects.toBeInstanceOf(GaveUp);
    expect(() => reject(new Error('too late'))).not.toThrow();
    vi.useRealTimers();
  });
});
```

- [ ] **Step 2: Run and confirm they fail**

Run: `bunx vitest run --project unit src/lib/server/bounded.test.ts`

- [ ] **Step 3: Implement `bounded.ts`**

Race the promise against a timer. Clear the timer on settle so it does not hold the process open.
Attach a no-op `.catch()` to the input promise so a late rejection is absorbed.

- [ ] **Step 4: Run and confirm all four pass**

- [ ] **Step 5: Probe**

Remove the internal `.catch()` and confirm the late-rejection test fails. Restore.

- [ ] **Step 6: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run --project unit src/lib/server/bounded.test.ts && bunx svelte-check --threshold error
git add src/lib/server/bounded.ts src/lib/server/bounded.test.ts
git commit -m "feat(ui): the system may stop waiting, and says that it did"
```

---

## Task 4: The resource detail page streams

The textbook scaffold-and-fill shape, and it carries **two live instances** of the failed-vs-empty
defect.

**Files:**
- Modify: `src/routes/(app)/vault/r/[ident]/+page.server.ts`
- Modify: `src/routes/(app)/vault/r/[ident]/+page.svelte`
- Create: `src/routes/(app)/vault/r/[ident]/page.server.test.ts`

**Interfaces:**
- Consumes: `RegionState` (Task 2), `bounded` / `GaveUp` (Task 3).
- Produces: the load's return type changes from
  `{ resource, content, trail, edges }` to `{ resource, content: Promise<…>, trail: Promise<…>, edges: Promise<…> }`.
  `resource` stays **awaited** — it is the scaffold, and its 404 must still be a real 404.

**The defect being fixed, quoted from the file as it stands:**

```ts
// src/routes/(app)/vault/r/[ident]/+page.server.ts — current
readTrail(accessToken, 'node', id).catch((): EventTrail | null => null),
readResourceEdges(accessToken, id).catch((): GraphEdgeRow[] => []),
```

A failed trail becomes `null`; a resource with no history is also `null`. A failed edges read
becomes `[]`; a resource with no connections is also `[]`. **The same file already articulates the
right principle one line above** — *"The content read is deliberately NOT caught — an API error must
surface as an error, not render as an empty document"* — and applies it to one read of three.

- [ ] **Step 1: Write the failing route-level guard**

```ts
import { describe, expect, it, vi } from 'vitest';

const apiGet = vi.fn();
vi.mock('$lib/server/api', () => ({
  apiGet: (...a: unknown[]) => apiGet(...a),
  ApiError: class extends Error { status = 500; },
}));
vi.mock('$lib/server/graph-reads', () => ({
  readTrail: () => new Promise(() => {}),
  readResourceEdges: () => new Promise(() => {}),
}));

const { load } = await import('./+page.server');

describe('the resource page does not block on its fill', () => {
  it('C1: returns the scaffold with the fill still unsettled', async () => {
    apiGet.mockResolvedValueOnce({ id: 'r1', title: 'A resource' });
    const data = await (load as (e: unknown) => Promise<Record<string, unknown>>)({
      locals: { accessToken: 'tok' },
      params: { ident: 'r1' },
    });
    // The scaffold is a value; the fill is still a promise. If someone adds an `await`,
    // this load never returns and the test times out — which is the regression to catch.
    expect(data.resource).toMatchObject({ title: 'A resource' });
    expect(data.trail).toBeInstanceOf(Promise);
    expect(data.edges).toBeInstanceOf(Promise);
    expect(data.content).toBeInstanceOf(Promise);
  });
});
```

- [ ] **Step 2: Run and confirm it fails**

Run: `bunx vitest run --project unit "src/routes/(app)/vault/r/[ident]/page.server.test.ts"`
Expected: FAIL — the load currently awaits, so `trail` is not a Promise.

- [ ] **Step 3: Change the load to stream**

Await `resource` only. Return the other three as promises, each wrapped in `bounded(…, '<label>')`
and each carrying `.catch()` at creation per the global constraint. **Delete the
`.catch(() => null)` and `.catch(() => [])` degradations** — failure now travels to the template.

- [ ] **Step 4: Run and confirm it passes**

- [ ] **Step 5: Consume the promises in the page**

`+page.svelte` — wrap each of the three regions in `{#await}` / `{:then}` / `{:catch}`, using
`RegionState` for the arriving and failed branches, and for the `{:then}` branch when the result is
genuinely empty. Pass a `label` per region: `excerpt`, `history`, `connections`.

- [ ] **Step 6: Probe**

Re-add `await` in front of one streamed read. Confirm the guard fails (or times out). Restore.

- [ ] **Step 7: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run && bunx svelte-check --threshold error && bunx biome check src
git add "src/routes/(app)/vault/r/[ident]"
git commit -m "fix(ui): a failed read on the resource page no longer reads as an empty one"
```

---

## Task 5: The graph rail streams, and stops conflating failure with absence

The panel the goal's first reports were filed against.

**Files:**
- Modify: `src/routes/(app)/graph/[owner]/+page.server.ts` (the `resolveSelection` helper)
- Modify: `src/lib/components/graph/NodeRail.svelte`
- Modify: `src/lib/components/graph/GraphPage.component.test.ts`

**Interfaces:**
- Consumes: `RegionState`, `bounded`.
- Produces: `GraphViewData.selectedExcerpt` and `.selectedTrail` change from settled values to
  promises. **`selected` stays a settled string** — it is the scaffold, resolved against the model,
  and `GraphRead`'s subtraction type must keep working unchanged.

**Grounding:** the rail's title, doc type, home and neighbour rows all come from `model.nodes`,
which is **already in `data`**. Only the excerpt and trail are reads. So the rail frame paints
fully populated with exactly two arriving regions.

**Current conflation** (`src/lib/components/graph/NodeRail.svelte:39`):

```ts
const history = $derived(trail ? trailModel(trail) : []);
```

- [ ] **Step 1: Write the failing component tests**

Extend `GraphPage.component.test.ts`, following the `describe('a traversed view')` idiom already in
that file — build `data` with `selectedTrail` as a never-settling promise, then as a rejected one.

```ts
// `tick` flushes Svelte's reactivity so a settled promise's branch has rendered.
// `view()` is the existing fixture builder at the top of this file; take the id from
// the fixture's own model rather than hard-coding one.
import { tick } from 'svelte';

describe('the rail declares what is still arriving', () => {
  const selectedId = () => view().model.nodes[0].id;

  it('C1: the rail frame and title paint while its reads are still in flight', () => {
    const pending = new Promise<never>(() => {});
    const data = { ...view(), selected: selectedId(),
      selectedExcerpt: pending, selectedTrail: pending };
    const { container } = render(GraphPage, { data });
    const rail = container.querySelector('[data-testid="node-rail"]');
    expect(rail).not.toBeNull();
    expect(rail?.textContent).toContain('NEIGHBORS');       // from the model, already held
    expect(container.querySelector('[data-testid="region-arriving"]')).not.toBeNull();
  });

  it('C3: a failed trail read says so, and does NOT read as still arriving', async () => {
    const failed = Promise.reject(new Error('503'));
    failed.catch(() => {});   // the global constraint, in the test too
    const data = { ...view(), selected: selectedId(),
      selectedExcerpt: Promise.resolve(null), selectedTrail: failed };
    const { container } = render(GraphPage, { data });
    await tick();
    await tick();
    const rail = container.querySelector('[data-testid="node-rail"]');
    expect(rail?.querySelector('[data-testid="region-failed"]')).not.toBeNull();
    expect(rail?.querySelector('[data-testid="region-arriving"]')).toBeNull();
    expect(rail?.textContent?.toLowerCase()).toContain('history');
  });
});
```

- [ ] **Step 2: Run and confirm they fail**

- [ ] **Step 3: Stream the two reads in `resolveSelection`**

Return `selectedExcerpt` and `selectedTrail` as `bounded(...)` promises with `.catch()` attached.
Keep `selected` resolved against the model exactly as now — a `sel` the read does not contain still
opens nothing.

- [ ] **Step 4: Consume them in `NodeRail.svelte`**

Replace the `trail ? … : []` derivation with `{#await}` blocks using `RegionState`. The
`{:then}` branch distinguishes a resolved-but-empty trail (`RegionState state="empty"`) from a
populated one.

- [ ] **Step 5: Run and confirm the whole component suite passes**

The existing rail tests must keep passing — if any of them asserted on the old `[]` collapse, that
is a real finding, not a test to paper over. Report it rather than adjusting it silently.

- [ ] **Step 6: Probe**

Make the `{:catch}` branch render `RegionState state="arriving"`. Confirm C3 fails. Restore.

- [ ] **Step 7: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run && bunx svelte-check --threshold error && bunx biome check src
git add "src/routes/(app)/graph/[owner]/+page.server.ts" src/lib/components/graph/
git commit -m "fix(graph): the rail paints before its reads answer, and a failure is not an absence"
```

---

## Task 6: The graph page streams its model and readout

**Files:**
- Modify: `src/routes/(app)/graph/[owner]/+page.server.ts`
- Modify: `src/lib/components/graph/GraphPage.svelte`
- Modify: `src/routes/(app)/graph/[owner]/page.server.test.ts`

**Interfaces:**
- Consumes: `RegionState`, `bounded`.
- Produces: `GraphViewData.model` and `.bound` become promises; `readout` becomes a promise on the
  composition path. `question`, `refusal`, `placesAsked` and `owner` stay settled — they are the
  scaffold, and `refusal` in particular must render **before** any read, since a refusal is the
  answer rather than a delay.

**What must still be awaited, and why:** `readAnchorSources` feeds `buildGraphPlan`, and the plan
decides which read runs at all. It stays awaited. Everything downstream of the plan streams.

- [ ] **Step 1: Extend the route test with the guard**

Add to the existing `describe('the three-way split — which read an address gets')` block:

```ts
it('C1: returns the page scaffold with the model still in flight', async () => {
  readEntry.mockReturnValue(new Promise(() => {}));   // never settles
  const data = await run();
  expect(data.model).toBeInstanceOf(Promise);
  expect(data.question).not.toBeInstanceOf(Promise);  // the scaffold is a value
});
```

- [ ] **Step 2: Run and confirm it fails**

- [ ] **Step 3: Stream the model, bound and readout**

Each `bounded(...)` with `.catch()` at creation. The three-way split is unchanged — only what the
branches return changes shape.

- [ ] **Step 4: Consume in `GraphPage.svelte`**

`{#await data.model}` around the canvas and the accessibility list; `RegionState` for arriving and
failed. The ask box, the refusal, and the page chrome render outside the await.

- [ ] **Step 5: Run the whole suite**

The 45 existing `GraphPage.component.test.ts` tests must be updated to pass settled promises where
they currently pass values. **Prefer a helper in the test file over editing 45 call sites.**

- [ ] **Step 6: Probe**

Re-add `await` before the model read. Confirm the guard fails. Restore.

- [ ] **Step 7: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run && bunx svelte-check --threshold error && bunx biome check src
git add "src/routes/(app)/graph/[owner]" src/lib/components/graph/GraphPage.svelte
git commit -m "feat(graph): the page paints its chrome before the graph arrives"
```

---

## Task 7: The analysis page streams its measurements

The last route with a remainder. Smaller than Task 6 and the same shape.

**Files:**
- Modify: `src/routes/(app)/graph/[owner]/analysis/+page.server.ts`
- Modify: `src/lib/components/graph/AnalysisPage.svelte`
- Modify: `src/lib/components/graph/AnalysisPage.component.test.ts`

**Interfaces:**
- Consumes: `RegionState`, `bounded`.
- Produces: `AnalysisViewData.regions` and the map-level analytics become promises. `place`,
  `alsoNamed`, `choices` and `refusal` stay settled — the page must be able to say *which place it
  is measuring* before any measurement arrives.

- [ ] **Step 1: Write the failing test**

Mirror Task 6's guard against this route's load: the place is a value, the measurements are a
promise.

- [ ] **Step 2: Run and confirm it fails**
- [ ] **Step 3: Stream the measurements** — `bounded(...)`, `.catch()` at creation
- [ ] **Step 4: Consume with `{#await}` + `RegionState`**
- [ ] **Step 5: Run the 19 existing analysis component tests and update the fixtures**
- [ ] **Step 6: Probe** — re-add the `await`, confirm the guard fails, restore
- [ ] **Step 7: Verify and commit**

```bash
cd packages/temper-ui
bunx vitest run && bunx svelte-check --threshold error && bunx biome check src
git add "src/routes/(app)/graph/[owner]/analysis" src/lib/components/graph/AnalysisPage.svelte
git commit -m "feat(graph): the analysis door names its place before its measurements land"
```

---

## What this plan does NOT do

Carried so a reviewer does not read completion as coverage.

- **`acting-on-a-part-does-not-discard-the-whole` gets nothing here.** Streaming changes what a load
  returns, not whether a navigation redraws the page around it. Spec §7; filed as
  [Opening a panel re-runs the whole page load](./01a0265c-7064-7371-993c-b907dd46f9f5).
- **The streaming transport is unwitnessed.** Every test here is jsdom; none exercises the server
  serializing an unsettled promise or the client reattaching. Spec §6.1.
- **No read gets faster.** Phase 2.
- **The refusal duration is unruled.** Task 3 ships a default and flags it.
- **The public route group is untouched.**
