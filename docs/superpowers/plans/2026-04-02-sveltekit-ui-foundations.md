# SvelteKit UI Foundations — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a working SvelteKit scaffold at `packages/temper-ui/` with Tailwind v4, ts-rs type generation, and Vercel rewrite configuration, verified locally.

**Architecture:** Two Vercel projects from one repo. temper-ui (SvelteKit) owns temperkb.io with `/api/*` rewritten to temper-cloud (existing Rust Axum). Types generated from Rust structs via ts-rs.

**Tech Stack:** SvelteKit 2, Svelte 5 (runes), Tailwind CSS v4, adapter-vercel (nodejs22.x), postgres.js, ts-rs, cargo-make

**Spec:** `docs/superpowers/specs/2026-04-02-sveltekit-ui-foundations-design.md`

---

## File Map

### New files (packages/temper-ui/)

| File | Responsibility |
|------|---------------|
| `package.json` | Package manifest, scripts, dependencies |
| `svelte.config.js` | SvelteKit config: adapter-vercel, runes mode |
| `vite.config.ts` | Vite config: sveltekit + tailwind plugins |
| `tsconfig.json` | TypeScript strict config |
| `vercel.json` | API rewrite to temper-cloud |
| `src/app.html` | HTML shell |
| `src/app.d.ts` | SvelteKit type declarations (App.Locals) |
| `src/app.css` | Tailwind v4 import + theme customization |
| `src/hooks.server.ts` | Auth middleware stub |
| `src/lib/server/db.ts` | Neon postgres.js connection |
| `src/lib/server/api.ts` | Typed API proxy helpers |
| `src/lib/types/index.ts` | Re-exports generated types |
| `src/routes/+layout.svelte` | Root layout: CSS import, minimal nav |
| `src/routes/+page.svelte` | Landing page placeholder |
| `src/routes/(app)/+layout.svelte` | Authenticated layout group |
| `src/routes/(app)/dashboard/+page.svelte` | Dashboard placeholder |
| `static/robots.txt` | Robots file |

### Modified files (existing)

| File | Change |
|------|--------|
| `package.json` (root) | Add `packages/temper-ui` to workspaces |
| `crates/temper-core/Cargo.toml` | Add `ts-rs` dependency with feature flag |
| `crates/temper-core/src/types/profile.rs` | Add `#[derive(TS)]` to Profile, ProfileAuthLink |
| `crates/temper-core/src/types/resource.rs` | Add `#[derive(TS)]` to ResourceRow, ContentResponse |
| `crates/temper-core/src/types/context.rs` | Add `#[derive(TS)]` to ContextRow |
| `crates/temper-core/src/types/team.rs` | Add `#[derive(TS)]` to Team, TeamMember, TeamRole |
| `crates/temper-core/src/types/api.rs` | Add `#[derive(TS)]` to SearchResultRow |
| `crates/temper-core/src/types/access.rs` | Add `#[derive(TS)]` to AccessLevel |
| `crates/temper-core/src/types/invitation.rs` | Add `#[derive(TS)]` to TeamInvitation, InvitationStatus |
| `crates/temper-core/src/types/transfer.rs` | Add `#[derive(TS)]` to ResourceTransfer, TransferStatus |
| `crates/temper-core/src/types/event.rs` | Add `#[derive(TS)]` to EventRow |
| `Makefile.toml` | Add `generate-ts-types` task |

### Generated files (not hand-authored)

| File | Source |
|------|--------|
| `packages/temper-ui/src/lib/types/generated/*.ts` | ts-rs export from temper-core structs |

---

## Task 1: SvelteKit Project Scaffold

**Files:**
- Create: `packages/temper-ui/package.json`
- Create: `packages/temper-ui/svelte.config.js`
- Create: `packages/temper-ui/vite.config.ts`
- Create: `packages/temper-ui/tsconfig.json`
- Create: `packages/temper-ui/src/app.html`
- Create: `packages/temper-ui/src/app.d.ts`
- Modify: `package.json` (root)

- [ ] **Step 1: Create package.json**

Create `packages/temper-ui/package.json`:

```json
{
  "name": "@temper/ui",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite dev",
    "build": "vite build",
    "preview": "vite preview",
    "prepare": "svelte-kit sync || echo ''",
    "check": "svelte-kit sync && svelte-check --tsconfig ./tsconfig.json"
  },
  "devDependencies": {
    "@sveltejs/adapter-vercel": "^5",
    "@sveltejs/kit": "^2",
    "@sveltejs/vite-plugin-svelte": "^6",
    "@tailwindcss/vite": "^4",
    "svelte": "^5",
    "svelte-check": "^4",
    "tailwindcss": "^4",
    "typescript": "^5",
    "vite": "^7"
  },
  "dependencies": {
    "postgres": "^3"
  }
}
```

- [ ] **Step 2: Create svelte.config.js**

Create `packages/temper-ui/svelte.config.js` — matches storyteller-site pattern:

```javascript
import adapter from '@sveltejs/adapter-vercel';
import { relative, sep } from 'node:path';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	compilerOptions: {
		runes: ({ filename }) => {
			const relativePath = relative(import.meta.dirname, filename);
			const pathSegments = relativePath.toLowerCase().split(sep);
			const isExternalLibrary = pathSegments.includes('node_modules');
			return isExternalLibrary ? undefined : true;
		}
	},
	kit: {
		adapter: adapter({
			runtime: 'nodejs22.x'
		}),
		alias: {
			'$components': 'src/lib/components'
		}
	}
};

export default config;
```

- [ ] **Step 3: Create vite.config.ts**

Create `packages/temper-ui/vite.config.ts`:

```typescript
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()]
});
```

- [ ] **Step 4: Create tsconfig.json**

Create `packages/temper-ui/tsconfig.json`:

```json
{
  "extends": "./.svelte-kit/tsconfig.json",
  "compilerOptions": {
    "rewriteRelativeImportExtensions": true,
    "allowJs": true,
    "checkJs": true,
    "strict": true,
    "moduleResolution": "bundler",
    "sourceMap": true,
    "resolveJsonModule": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  }
}
```

- [ ] **Step 5: Create app.html**

Create `packages/temper-ui/src/app.html`:

```html
<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<meta name="viewport" content="width=device-width, initial-scale=1" />
		<link rel="icon" href="%sveltekit.assets%/favicon.svg" />
		<title>temper — knowledge base with structure</title>
		%sveltekit.head%
	</head>
	<body data-sveltekit-preload-data="hover">
		<div style="display: contents">%sveltekit.body%</div>
	</body>
</html>
```

- [ ] **Step 6: Create app.d.ts**

Create `packages/temper-ui/src/app.d.ts`:

```typescript
declare global {
	namespace App {
		interface Locals {
			user: { profileId: string; email: string; displayName: string } | null;
			accessToken: string | null;
		}
	}
}

export {};
```

- [ ] **Step 7: Add temper-ui to root workspaces**

In root `package.json`, change:
```json
"workspaces": ["packages/temper-cloud"]
```
to:
```json
"workspaces": ["packages/temper-cloud", "packages/temper-ui"]
```

- [ ] **Step 8: Install dependencies**

Run: `cd packages/temper-ui && bun install`

Expected: Dependencies install successfully. `.svelte-kit/` directory is created.

- [ ] **Step 9: Commit**

```bash
git add packages/temper-ui/package.json packages/temper-ui/svelte.config.js packages/temper-ui/vite.config.ts packages/temper-ui/tsconfig.json packages/temper-ui/src/app.html packages/temper-ui/src/app.d.ts package.json
git commit -m "feat(temper-ui): scaffold SvelteKit project with adapter-vercel and Tailwind v4"
```

---

## Task 2: Tailwind v4 and Styles

**Files:**
- Create: `packages/temper-ui/src/app.css`

- [ ] **Step 1: Create app.css with Tailwind v4 import and theme**

Create `packages/temper-ui/src/app.css`:

```css
@import "tailwindcss";

@theme {
	--color-temper-50: #f0f7ff;
	--color-temper-100: #e0effe;
	--color-temper-200: #bae0fd;
	--color-temper-300: #7ccbfc;
	--color-temper-400: #36b2f8;
	--color-temper-500: #0c99e9;
	--color-temper-600: #0079c7;
	--color-temper-700: #0060a1;
	--color-temper-800: #045185;
	--color-temper-900: #09446e;
	--color-temper-950: #062b49;

	--color-ink: #1a1a2e;
	--color-chalk: #f8f9fa;

	--font-sans: "Inter", "system-ui", "sans-serif";
	--font-mono: "JetBrains Mono", "Fira Code", "monospace";
}
```

- [ ] **Step 2: Commit**

```bash
git add packages/temper-ui/src/app.css
git commit -m "feat(temper-ui): add Tailwind v4 styles with temper color palette"
```

---

## Task 3: Placeholder Routes and Layouts

**Files:**
- Create: `packages/temper-ui/src/routes/+layout.svelte`
- Create: `packages/temper-ui/src/routes/+page.svelte`
- Create: `packages/temper-ui/src/routes/(app)/+layout.svelte`
- Create: `packages/temper-ui/src/routes/(app)/dashboard/+page.svelte`
- Create: `packages/temper-ui/static/robots.txt`

- [ ] **Step 1: Create root layout**

Create `packages/temper-ui/src/routes/+layout.svelte`:

```svelte
<script>
	import '../app.css';

	let { children } = $props();
</script>

<div class="min-h-screen bg-chalk text-ink font-sans">
	<header class="border-b border-temper-200 px-6 py-4">
		<nav class="mx-auto flex max-w-6xl items-center justify-between">
			<a href="/" class="text-xl font-semibold text-temper-700">temper</a>
			<div class="flex gap-6 text-sm">
				<a href="/docs" class="text-temper-600 hover:text-temper-800">Docs</a>
				<a href="/dashboard" class="text-temper-600 hover:text-temper-800">Dashboard</a>
			</div>
		</nav>
	</header>

	<main>
		{@render children()}
	</main>
</div>
```

- [ ] **Step 2: Create landing page**

Create `packages/temper-ui/src/routes/+page.svelte`:

```svelte
<div class="mx-auto max-w-4xl px-6 py-24 text-center">
	<h1 class="text-5xl font-bold tracking-tight text-temper-900">
		Your knowledge base, with structure
	</h1>
	<p class="mt-6 text-lg text-temper-600">
		CLI-first knowledge base with semantic search, frontmatter-driven structure, and cloud sync.
	</p>
	<div class="mt-10 flex justify-center gap-4">
		<a
			href="/docs/getting-started"
			class="rounded-lg bg-temper-600 px-6 py-3 text-sm font-medium text-white hover:bg-temper-700"
		>
			Get Started
		</a>
		<a
			href="https://github.com/tasker-systems/temper"
			class="rounded-lg border border-temper-300 px-6 py-3 text-sm font-medium text-temper-700 hover:bg-temper-50"
		>
			View on GitHub
		</a>
	</div>
</div>
```

- [ ] **Step 3: Create authenticated layout group**

Create `packages/temper-ui/src/routes/(app)/+layout.svelte`:

```svelte
<script>
	let { children } = $props();
</script>

<div class="mx-auto flex max-w-7xl gap-6 px-6 py-8">
	<aside class="w-56 shrink-0">
		<nav class="space-y-1 text-sm">
			<a href="/dashboard" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Dashboard
			</a>
			<a href="/resources" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Resources
			</a>
			<a href="/contexts" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Contexts
			</a>
			<a href="/search" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Search
			</a>
			<a href="/teams" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Teams
			</a>
		</nav>
	</aside>

	<div class="flex-1">
		{@render children()}
	</div>
</div>
```

- [ ] **Step 4: Create dashboard placeholder**

Create `packages/temper-ui/src/routes/(app)/dashboard/+page.svelte`:

```svelte
<div>
	<h1 class="text-2xl font-semibold text-temper-900">Dashboard</h1>
	<p class="mt-2 text-temper-600">
		Welcome to temper. Authentication and dashboard data coming in future sessions.
	</p>
</div>
```

- [ ] **Step 5: Create robots.txt**

Create `packages/temper-ui/static/robots.txt`:

```
User-agent: *
Allow: /
Disallow: /dashboard
Disallow: /resources
Disallow: /settings
Sitemap: https://temperkb.io/sitemap.xml
```

- [ ] **Step 6: Verify dev server starts**

Run: `cd packages/temper-ui && bun run dev`

Expected: SvelteKit dev server starts on `http://localhost:5173`. Navigate to it in browser — should see the landing page with "Your knowledge base, with structure" heading and the temper nav header. Navigate to `/dashboard` — should see sidebar layout with dashboard placeholder.

Kill the dev server after verification.

- [ ] **Step 7: Commit**

```bash
git add packages/temper-ui/src/routes/ packages/temper-ui/static/robots.txt
git commit -m "feat(temper-ui): add placeholder routes with root and app layouts"
```

---

## Task 4: Server-Side Stubs (db.ts and api.ts)

**Files:**
- Create: `packages/temper-ui/src/lib/server/db.ts`
- Create: `packages/temper-ui/src/lib/server/api.ts`
- Create: `packages/temper-ui/src/hooks.server.ts`

- [ ] **Step 1: Create Neon database connection**

Create `packages/temper-ui/src/lib/server/db.ts`:

```typescript
import postgres from 'postgres';
import { DATABASE_URL } from '$env/static/private';

export const sql = postgres(DATABASE_URL, {
	max: 10,
	idle_timeout: 20,
	connect_timeout: 10
});
```

- [ ] **Step 2: Create API proxy helpers**

Create `packages/temper-ui/src/lib/server/api.ts`:

```typescript
import { API_BASE_URL } from '$env/static/private';

export class ApiError extends Error {
	status: number;
	body: unknown;

	constructor(status: number, message: string, body?: unknown) {
		super(message);
		this.status = status;
		this.body = body;
	}
}

export async function apiGet<T>(path: string, accessToken: string): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		headers: { Authorization: `Bearer ${accessToken}` }
	});
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (body as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, body);
	}
	return res.json() as Promise<T>;
}

export async function apiPost<T>(path: string, accessToken: string, body: unknown): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'POST',
		headers: {
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(body)
	});
	if (!res.ok) {
		const errBody = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (errBody as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, errBody);
	}
	return res.json() as Promise<T>;
}

export async function apiPatch<T>(path: string, accessToken: string, body: unknown): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'PATCH',
		headers: {
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(body)
	});
	if (!res.ok) {
		const errBody = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (errBody as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, errBody);
	}
	return res.json() as Promise<T>;
}

export async function apiDelete(path: string, accessToken: string): Promise<void> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'DELETE',
		headers: { Authorization: `Bearer ${accessToken}` }
	});
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (body as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, body);
	}
}
```

- [ ] **Step 3: Create hooks.server.ts stub**

Create `packages/temper-ui/src/hooks.server.ts`:

```typescript
import type { Handle } from '@sveltejs/kit';

export const handle: Handle = async ({ event, resolve }) => {
	// Auth middleware — implemented in session 2.
	// For now, locals are null (unauthenticated).
	event.locals.user = null;
	event.locals.accessToken = null;

	return resolve(event);
};
```

- [ ] **Step 4: Verify the project still builds**

Run: `cd packages/temper-ui && bun run check`

Expected: `svelte-kit sync` succeeds and `svelte-check` passes with no errors.

Note: `DATABASE_URL` and `API_BASE_URL` are only used at runtime, not at type-check time, so `$env/static/private` imports resolve via SvelteKit's type generation.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/server/ packages/temper-ui/src/hooks.server.ts
git commit -m "feat(temper-ui): add server stubs for Neon db, API proxy, and auth hooks"
```

---

## Task 5: Vercel Configuration

**Files:**
- Create: `packages/temper-ui/vercel.json`

- [ ] **Step 1: Create vercel.json with API rewrite**

Create `packages/temper-ui/vercel.json`:

```json
{
	"rewrites": [
		{ "source": "/api/:path*", "destination": "https://temper-cloud.vercel.app/api/:path*" }
	]
}
```

Using the stable production URL `temper-cloud.vercel.app` as the rewrite destination. This can be changed to an environment-variable-based approach later if Vercel supports it.

- [ ] **Step 2: Commit**

```bash
git add packages/temper-ui/vercel.json
git commit -m "feat(temper-ui): add vercel.json with API rewrite to temper-cloud"
```

---

## Task 6: ts-rs Type Generation — Rust Side

**Files:**
- Modify: `crates/temper-core/Cargo.toml`
- Modify: `crates/temper-core/src/types/profile.rs`
- Modify: `crates/temper-core/src/types/resource.rs`
- Modify: `crates/temper-core/src/types/context.rs`
- Modify: `crates/temper-core/src/types/team.rs`
- Modify: `crates/temper-core/src/types/api.rs`
- Modify: `crates/temper-core/src/types/access.rs`
- Modify: `crates/temper-core/src/types/invitation.rs`
- Modify: `crates/temper-core/src/types/transfer.rs`
- Modify: `crates/temper-core/src/types/event.rs`

- [ ] **Step 1: Add ts-rs dependency to temper-core**

In `crates/temper-core/Cargo.toml`, add to `[features]`:

```toml
[features]
web-api = ["utoipa"]
typescript = ["ts-rs"]
```

And add to `[dependencies]`:

```toml
ts-rs = { version = "10", optional = true, features = ["chrono-impl", "uuid-impl", "serde-json-impl"] }
```

- [ ] **Step 2: Add ts-rs derives to profile types**

In `crates/temper-core/src/types/profile.rs`, add conditional derive to `Profile`:

After existing derives on the `Profile` struct, add:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "profile.ts"))]
```

Apply the same pattern to `ProfileAuthLink`.

Do NOT add ts-rs to `DeactivationCheck` — it's internal-only.

- [ ] **Step 3: Add ts-rs derives to resource types**

In `crates/temper-core/src/types/resource.rs`, add conditional derives to:

- `ResourceRow`:
```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
```

- `ContentResponse`: same pattern with `export_to = "resource.ts"`

- `ResourceListParams`: same pattern with `export_to = "resource.ts"`

Do NOT add to `ResourceCreateRequest`, `ResourceUpdateRequest` — those are API-internal.

- [ ] **Step 4: Add ts-rs derives to context types**

In `crates/temper-core/src/types/context.rs`, add to `ContextRow`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
```

- [ ] **Step 5: Add ts-rs derives to team types**

In `crates/temper-core/src/types/team.rs`, add to `Team`, `TeamMember`, and `TeamRole`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
```

- [ ] **Step 6: Add ts-rs derives to search result types**

In `crates/temper-core/src/types/api.rs`, add to `SearchResultRow`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
```

Also add to `EventRow` (in `event.rs` or `api.rs`, wherever it lives):

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "event.ts"))]
```

- [ ] **Step 7: Add ts-rs derives to access, invitation, transfer types**

In `crates/temper-core/src/types/access.rs`, add to `AccessLevel`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
```

In `crates/temper-core/src/types/invitation.rs`, add to `TeamInvitation` and `InvitationStatus`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
```

In `crates/temper-core/src/types/transfer.rs`, add to `ResourceTransfer` and `TransferStatus`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "transfer.ts"))]
```

- [ ] **Step 8: Verify Rust compilation with typescript feature**

Run: `cargo check -p temper-core --features typescript`

Expected: Compiles without errors. If any type has fields that ts-rs can't handle (e.g., custom newtypes), adjust with `#[ts(type = "...")]` annotations.

- [ ] **Step 9: Verify existing tests still pass**

Run: `cargo make test`

Expected: All existing tests pass. The `typescript` feature is additive and should not affect existing behavior.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-core/
git commit -m "feat(temper-core): add ts-rs derives for TypeScript codegen behind 'typescript' feature"
```

---

## Task 7: ts-rs Type Generation — Export and Cargo-Make Task

**Files:**
- Modify: `Makefile.toml`
- Create: `packages/temper-ui/src/lib/types/index.ts`
- Generate: `packages/temper-ui/src/lib/types/generated/*.ts`

- [ ] **Step 1: Create the ts-rs export test**

ts-rs generates files when you run tests with the `typescript` feature enabled. The `#[ts(export)]` attribute causes each annotated type to write its `.ts` file during test execution. By default, ts-rs writes to a `bindings/` directory relative to the crate root.

We need to configure the output directory. Add this to `crates/temper-core/Cargo.toml` under `[package.metadata]` or use the `TS_RS_EXPORT_DIR` environment variable.

The cargo-make task will use the environment variable approach.

- [ ] **Step 2: Add generate-ts-types task to Makefile.toml**

In the root `Makefile.toml`, add:

```toml
[tasks.generate-ts-types]
description = "Generate TypeScript types from temper-core Rust structs via ts-rs"
env = { "TS_RS_EXPORT_DIR" = "packages/temper-ui/src/lib/types/generated" }
script = [
    "mkdir -p packages/temper-ui/src/lib/types/generated",
    "cargo test -p temper-core --features typescript ts_export --lib -- --ignored 2>/dev/null || cargo test -p temper-core --features typescript --lib 2>&1 | head -20",
    "echo '// Auto-generated by ts-rs from temper-core. Do not edit.' > packages/temper-ui/src/lib/types/generated/_header.ts"
]
```

Note: ts-rs exports happen as a side effect of running the test suite with the `typescript` feature. The actual test names may vary — ts-rs creates `#[test]` functions automatically for each `#[ts(export)]` type. If the generated tests are named differently, we'll adjust the filter.

- [ ] **Step 3: Run the generation**

Run: `cargo make generate-ts-types`

Expected: TypeScript files appear in `packages/temper-ui/src/lib/types/generated/`:
- `profile.ts` — `Profile`, `ProfileAuthLink`
- `resource.ts` — `ResourceRow`, `ContentResponse`, `ResourceListParams`
- `context.ts` — `ContextRow`
- `team.ts` — `Team`, `TeamMember`, `TeamRole`
- `search.ts` — `SearchResultRow`
- `event.ts` — `EventRow`
- `access.ts` — `AccessLevel`
- `invitation.ts` — `TeamInvitation`, `InvitationStatus`
- `transfer.ts` — `ResourceTransfer`, `TransferStatus`

If files don't appear, check `TS_RS_EXPORT_DIR` is being picked up. ts-rs v10 uses this env var. Adjust the task if needed.

- [ ] **Step 4: Create the types index file**

Create `packages/temper-ui/src/lib/types/index.ts`:

```typescript
// Re-export all generated types from temper-core
export * from './generated/profile.ts';
export * from './generated/resource.ts';
export * from './generated/context.ts';
export * from './generated/team.ts';
export * from './generated/search.ts';
export * from './generated/event.ts';
export * from './generated/access.ts';
export * from './generated/invitation.ts';
export * from './generated/transfer.ts';
```

Note: The exact filenames depend on what ts-rs generates. After step 3, inspect the `generated/` directory and adjust these imports to match the actual output filenames.

- [ ] **Step 5: Add generated directory to .gitignore consideration**

The generated types should be committed to git so that the SvelteKit build on Vercel doesn't need to compile Rust. They are regenerated locally when Rust types change.

Do NOT add `packages/temper-ui/src/lib/types/generated/` to `.gitignore`.

- [ ] **Step 6: Verify SvelteKit check still passes**

Run: `cd packages/temper-ui && bun run check`

Expected: `svelte-check` passes. The generated types are valid TypeScript that SvelteKit's module resolution can find.

- [ ] **Step 7: Commit**

```bash
git add Makefile.toml packages/temper-ui/src/lib/types/
git commit -m "feat(temper-ui): add ts-rs codegen pipeline and generated TypeScript types"
```

---

## Task 8: Final Verification

- [ ] **Step 1: Run full Rust check**

Run: `cargo make check`

Expected: All Rust formatting, clippy, and doc checks pass.

- [ ] **Step 2: Run full Rust tests**

Run: `cargo make test`

Expected: All existing tests pass.

- [ ] **Step 3: Verify SvelteKit dev server**

Run: `cd packages/temper-ui && bun run dev`

Verify in browser:
1. `http://localhost:5173/` — Landing page with hero heading and nav
2. `http://localhost:5173/dashboard` — App layout with sidebar and dashboard placeholder
3. Page styles use Tailwind classes (temper blue colors, Inter font)

Kill the dev server.

- [ ] **Step 4: Verify SvelteKit production build**

Run: `cd packages/temper-ui && bun run build`

Expected: Build succeeds. Output in `.svelte-kit/output/`.

- [ ] **Step 5: Commit any remaining changes**

If the build or check steps required any fixes, commit them:

```bash
git add -A packages/temper-ui/
git commit -m "fix(temper-ui): address build/check issues from final verification"
```

If no fixes were needed, skip this step.

---

## Post-Implementation Notes

### Vercel Project Setup (manual, not automated)

After the code is merged, the temper-ui Vercel project needs to be created via the Vercel dashboard:

1. Import the `tasker-systems/temper` GitHub repo as a new project
2. Set Root Directory to `packages/temper-ui`
3. Framework should auto-detect as SvelteKit
4. Add environment variables: `DATABASE_URL`, `API_BASE_URL` (empty for production), `PUBLIC_APP_URL`
5. Deploy and verify the landing page renders
6. Assign `temperkb.io` domain to this project (domain transfer from temper-cloud)

### Next Session

Session 2: Auth0 Server-Side Integration. See the design spec section 7 for full details.
