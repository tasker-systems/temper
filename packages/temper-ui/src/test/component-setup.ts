// Testing Library's default entry registers its hooks against the GLOBAL `beforeEach`/`afterEach`,
// which `globals: false` never installs. The `/vitest` entry exists for exactly this case — it
// imports them from `vitest` explicitly — and installs BOTH halves. Registering `afterEach(cleanup)`
// by hand gets only the second: it drops `setup()`, which configures Testing Library's
// `asyncWrapper` to `act` and its `eventWrapper` to `flushSync`, so `findBy*`/`waitFor` would not
// flush Svelte updates and the first async query written here would misbehave for no visible reason.
import '@testing-library/svelte/vitest';

// jsdom implements no `ResizeObserver`; `wx-svelte-grid` constructs one unconditionally when
// its layout mounts (`wx-svelte-grid/src/helpers/actions/onresize.js:2`), so `VaultGrid` throws
// on render without this. The stub observes nothing on purpose — no test here asserts on a
// measured size, and one that needed to would belong in a `/dev/*` harness (see README.md).
class ResizeObserverStub {
	observe() {}
	unobserve() {}
	disconnect() {}
}

globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;
