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
is correct witnesses nothing `nav-groups.test.ts` does not already pin; it buys a file and CI
time for no coverage. The test earns its place only where the wiring is the thing under test.

The check that settles it: **apply your bite probe and run the whole suite.** If the pure
tests go red too, the component test is redundant. If only it goes red, it is buying coverage
nothing else has. `FacetChips.component.test.ts` was introduced this way — reverting the
component to iterate its raw histogram fails 3 component tests and **0** of the 40 pure files.

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

**The cost taken, stated plainly.** jsdom is cheap — the `unit` project is unchanged by the
split — but it tests DOM structure and wiring, which is *further from what a reader
experiences* than a real browser would be. That gap is deliberate, and it is why the third
layer above is not optional.

## Writing one

Two vitest projects live in `vite.config.ts`: `unit` (node, everything except
`*.component.test.ts`) and `component` (jsdom, only those). Name the file
`<Component>.component.test.ts` beside the component.

`app-context.ts` stands in for the SvelteKit ambient modules and is also the control surface
the test drives them with — mock with it, then import from it:

```ts
vi.mock('$app/stores', () => import('../../test/app-context'));
vi.mock('$app/navigation', () => import('../../test/app-context'));
import { goto, resetAppContext, setPage } from '../../test/app-context';

beforeEach(resetAppContext);
```

`setPage(href, params)` supplies route params explicitly, because nothing here runs
SvelteKit's router. That is what reaches the one behaviour `/dev/nav` structurally could not:
`isContextLocation` reads real route params, so no place is ever active in the harness — but a
component test can just say which one is.

## Two things jsdom needs, handled in `component-setup.ts`

- **`ResizeObserver`** — jsdom implements none; `wx-svelte-grid` constructs one on mount, so
  `VaultGrid` throws without the stub.
- **`cleanup`** — Testing Library registers auto-cleanup through the *global* `afterEach`,
  which `globals: false` never installs. Without it, each test queries the previous test's DOM.

And one in `vite.config.ts`: the component project sets `resolve.conditions: ['browser']`.
Without it Svelte resolves to `index-server.js` and every `render()` throws
`lifecycle_function_unavailable`.
