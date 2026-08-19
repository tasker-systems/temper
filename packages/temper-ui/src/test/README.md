# Component tests (`*.component.test.ts`)

Three layers verify this surface, and each exists because the other two structurally cannot
see what it sees. **State which one you are writing before you write it.**

| Layer | What it is for | Where it runs |
|---|---|---|
| **Pure module test** (`*.test.ts`) | Any decision expressible as a value: given these inputs, what is the answer. Still the default — keep extracting decisions out of components | vitest, `node` |
| **Component test** (`*.component.test.ts`) | The **wiring**. The click that mutates the URL. The control that is *absent* rather than disabled. The branch that renders X *instead of* Y. The persisted store state that reaches the DOM | vitest, `jsdom` |
| **Render harness** (`/dev/*`) | Appearance and legibility, on realistic data, judged by a human eye | `bun run dev`, never CI |

## What a component test may NOT do

**Re-assert what a pure module already asserts.** Mounting `Sidebar` to check that grouping
is correct witnesses nothing `nav-groups.test.ts` does not already pin. The test earns its
place only where the wiring is the thing under test.

The check that settles it: **apply your bite probe and run the whole suite.** If the pure
tests go red too, the component test is redundant. If only it goes red, it is buying coverage
nothing else has. `FacetChips.component.test.ts` was introduced this way — reverting the
component to iterate its raw histogram fails 4 of its 6 tests and **0** of the 40 pure files.

**Read the green tests in a probe run, not only the red ones.** A probe result is two facts,
and the second is easy to skip: which tests bit, *and which stayed green when they should not
have*. This file shipped with 3 of its 6 inert against the exact defect it exists to catch,
because a fixture's insertion order (`{ task: 7, goal: 2 }`) happened to match the order the
component sorts into, so the assertion could not tell the two apart. The probe output said
"3 failed" and looked clean. Choose fixture values that disagree with the shape under test.

**Assert on appearance.** No test here may claim a thing is legible, readable, or correctly
sized. jsdom computes no layout and `ResizeObserver` is a no-op stub (below). `/dev/nav`
caught an 8px caret nobody could see, places indented level with their own heading, and a
heading dimmer than the places it held — **no component test would catch any of them**, and
one written as though it might would be lying.

## Why jsdom, and not browser mode

`vitest-browser-svelte@3.0.0` peers `vitest: ^4.0.0`; this package is on **3.2.4**. Browser
mode is therefore a vitest major migration across the whole suite, not a harness choice.
`@testing-library/svelte@5.4.2` peers `vitest: '*'` and drops onto what is already here.

`playwright` being in `devDependencies` is **not** evidence of appetite for browser testing:
it was added by `e9a16b1f` "for rendered verification" of one diagram, and nothing in the repo
references it. That lineage is `/dev/*`, not CI.

**The cost taken, stated plainly. It is per FILE, not per mount.** Each
`*.component.test.ts` pays roughly **280ms of jsdom environment plus 285ms of setup — ~565ms
before a single component renders**, against about 0.08ms per file for the `node` project.
Mounting is the cheap half: `VaultGrid.component.test.ts`'s three full `wx-svelte-grid`
mounts total **89ms**. Measure it yourself — `bun run test --project component` reports
`environment` and `setup` as per-project sums; divide by the file count.

So the rule is the opposite of the intuitive one: **prefer another `it` in an existing
component file over a new file.** A further assertion is nearly free; a further file is half
a second.

The other cost is not time. jsdom tests DOM structure and wiring, which is *further from what
a reader experiences* than a real browser would be. That gap is deliberate, and it is why the
third layer above is not optional.

## Writing one

Two vitest projects live in `vite.config.ts`: `unit` (node, everything except
`*.component.test.ts`) and `component` (jsdom, only those). Name the file
`<Component>.component.test.ts` beside the component.

`app-context.ts` stands in for the SvelteKit ambient modules and is also the control surface
the test drives them with — mock with it, then import from it:

```ts
vi.mock('$app/stores', () => import('../../test/app-context'));
vi.mock('$app/navigation', () => import('../../test/app-context'));
import { goto, gotoTarget, resetAppContext, setPage } from '../../test/app-context';

beforeEach(resetAppContext);
```

**The depth is relative and varies** — `../../test/…` from `src/lib/components/`,
`../../../test/…` from `src/lib/components/vault/`. There is no alias for `src/test`; it sits
outside `$lib`.

`setPage(href, params)` supplies route params explicitly, because nothing here runs
SvelteKit's router. That is what reaches the one behaviour `/dev/nav` structurally could not:
`isContextLocation` reads real route params, so no place is ever active in the harness — but a
component test can just say which one is.

`gotoTarget(n)` parses the URL of the nth `goto` call. There is deliberately no way to read a
"current" URL back: `goto` is a no-op mock and never moves the page store, so *did the
component navigate* is answered by asserting on `goto` itself.

`fixtures.ts` holds `makeRow` for `ResourceView`. Use it rather than spelling the type out —
it is ts-rs-generated, so a field added on the Rust side breaks every literal that does.

**Dispatching an interaction:** a raw `.click()` is fine when the assertion lands on a mock.
If you assert on the DOM *after* an interaction, use `await fireEvent.*`, which wraps in
`act()` and awaits Svelte's tick; `.click()` does neither, and you will read stale DOM.

## Two things jsdom needs, handled in `component-setup.ts`

- **`ResizeObserver`** — jsdom implements none; `wx-svelte-grid` constructs one on mount, so
  `VaultGrid` throws without the stub.
- **Testing Library's own hooks** — its default entry registers them through the *global*
  `beforeEach`/`afterEach`, which `globals: false` never installs. Importing
  `@testing-library/svelte/vitest` installs them from an explicit `vitest` import instead.
  This is not only `cleanup` (without which each test queries the previous test's DOM); it is
  also `setup`, which configures `asyncWrapper`/`eventWrapper` so `findBy*` and `fireEvent`
  flush Svelte. Hand-rolling `afterEach(cleanup)` gets the first and silently loses the
  second — which this harness did until review caught it.

And one in `vite.config.ts`: the component project sets `resolve.conditions: ['browser']`.
Without it Svelte resolves to `index-server.js` and every `render()` throws
`lifecycle_function_unavailable`.
