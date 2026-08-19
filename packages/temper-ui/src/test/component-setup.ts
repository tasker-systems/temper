import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';

// `@testing-library/svelte` registers its own auto-cleanup through the GLOBAL `afterEach`,
// which `globals: false` never installs. Without this every mounted component stays in the
// document and the next test's queries match the previous test's DOM.
afterEach(cleanup);

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
