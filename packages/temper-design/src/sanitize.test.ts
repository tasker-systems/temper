// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { ABSENT_WORDS, type LinkResolution, UNCHECKED_WORDS } from './links.js';
import { prepareMarkdown } from './markdown.js';
import { sanitizeMarkdownHtml, WITHHELD_IMAGE, WITHHELD_MEDIA } from './sanitize.js';

/**
 * The payload battery for `rendering-agrees`: every surface renders a body through this one pass,
 * so what it strips or refuses here is stripped or refused everywhere. A surface's own suite
 * keeps only what its component layer alone can see — the gate before the sanitizer loads, and
 * the fallback for an empty body.
 *
 * Bodies go through `prepareMarkdown` first, as every renderer composes them. Every payload case
 * keeps a retention face, so a pass that stripped everything would not go green.
 */
function render(markdown: string): HTMLElement {
	const container = document.createElement('div');
	container.innerHTML = sanitizeMarkdownHtml(prepareMarkdown(markdown));
	return container;
}

describe('sanitizeMarkdownHtml', () => {
	it('keeps the document and drops the script', () => {
		const container = render(
			'# Temper\n\n<script>alert(1)</script>\n\nA [link](https://example.com).',
		);
		expect(container.querySelector('h1')?.textContent).toBe('Temper');
		expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com');
		expect(container.querySelector('script')).toBeNull();
		expect(container.textContent).not.toContain('alert');
	});

	it('keeps style attributes and non-allowlisted classes out of the rendered document', () => {
		const container = render('<p style="color:red;position:fixed" class="lead">stay</p>');
		expect(container.textContent).toContain('stay');
		expect(container.innerHTML).not.toContain('style=');
		// `lead` is not in the allowlist — its survival would mean the class channel is not closed.
		expect(container.innerHTML).not.toContain('lead');
	});

	it('keeps positioning utilities out of the rendered document — content survives, styling does not', () => {
		const container = render('<div class="fixed inset-0 z-50 bg-black/80">overlay</div>');
		expect(container.textContent).toContain('overlay');
		for (const utility of ['fixed', 'inset-0', 'z-50', 'bg-black']) {
			expect(container.innerHTML).not.toContain(utility);
		}
	});

	it('preserves the classes the markdown pipeline legitimately emits', () => {
		const html = render('```json\n{"a":1}\n```').innerHTML;
		expect(html).toContain('class="hljs language-json"');
		expect(html).toContain('hljs-');
	});

	it('preserves the refusal the pipeline emits for a body it cannot render', () => {
		const container = render('a'.repeat(262_145));
		expect(container.querySelector('.md-refusal')).not.toBeNull();
	});

	it('keeps an element but drops its event-handler attribute', () => {
		const src = 'data:image/png;base64,iVBORw0KGgo=';
		const container = render(`<img src="${src}" onerror="alert(1)">`);
		expect(container.querySelector('img')?.getAttribute('src')).toBe(src);
		expect(container.innerHTML).not.toContain('onerror');
	});

	it('withholds a remote image as a declared refusal, never a broken glyph', () => {
		const container = render(
			'![the plan](https://example.com/plan.png)\n\n<img src="//cdn.example/x.png" onerror="alert(1)">',
		);
		expect(container.querySelector('img')).toBeNull();
		const withheld = [...container.querySelectorAll('.md-withheld')];
		expect(withheld).toHaveLength(2);
		expect(withheld[0].textContent).toBe(`${WITHHELD_IMAGE}: the plan`);
		expect(withheld[0].getAttribute('title')).toBe('https://example.com/plan.png');
		expect(withheld[1].textContent).toBe(WITHHELD_IMAGE);
		expect(container.innerHTML).not.toContain('onerror');
	});

	it('withholds media as a declared refusal, keeping the fallback words and none of its fetches', () => {
		const container = render(
			'Before. <video poster="/p.png" src="/v.mp4"><source src="/s.mp4"><track src="/t.vtt">The walkthrough,\n at <a href="https://example.com/v">its page</a>.</video> <audio><source src="/a.mp3"></audio> After.',
		);
		expect(container.querySelector('video, audio, source, track')).toBeNull();
		const withheld = [...container.querySelectorAll('.md-withheld')];
		expect(withheld.map((s) => s.textContent)).toEqual([
			`${WITHHELD_MEDIA.video}: The walkthrough, at its page.`,
			WITHHELD_MEDIA.audio,
		]);
		expect(withheld.map((s) => s.getAttribute('title'))).toEqual(['/v.mp4', '/a.mp3']);
		expect(container.innerHTML).not.toContain('/p.png');
		expect(container.innerHTML).not.toContain('/t.vtt');
		expect(container.textContent).toContain('Before.');
		expect(container.textContent).toContain('After.');
	});

	it('does not let authored content forge a withheld-image refusal', () => {
		const container = render('<span class="md-withheld">remote image withheld</span>');
		expect(container.textContent).toContain('remote image withheld');
		expect(container.querySelector('.md-withheld')).toBeNull();
	});

	it('defuses namespace-confusion mXSS across math/style boundaries', () => {
		const container = render('<math><mtext><form><mglyph><style></math><img src onerror=alert(1)>');
		expect(container.querySelector('img')).toBeNull();
		expect(container.innerHTML).not.toContain('onerror');
		expect(container.innerHTML).not.toContain('<mglyph');
	});

	it('drops hrefs whose scheme hides behind entities or data payloads', () => {
		const container = render(
			'<a href="jav&#x09;ascript:alert(1)">x</a>\n\n[x](data:text/html;base64,PHNjcmlwdD4=)',
		);
		expect(container.querySelector('a')).not.toBeNull();
		expect(container.innerHTML).not.toContain('javascript:');
		expect(container.innerHTML).not.toContain('alert');
		expect(container.innerHTML).not.toContain('data:text/html');
	});

	it('strips DOM-clobbering name attributes from the controls it keeps', () => {
		const container = render(
			'<input type="checkbox" name="attributes" disabled><input type="checkbox" name="tagName" disabled>',
		);
		expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
		expect(container.innerHTML).not.toContain('name="attributes"');
		expect(container.innerHTML).not.toContain('name="tagName"');
	});

	it('keeps the checkboxes a task list renders', () => {
		const container = render('- [x] done\n- [ ] not yet');
		const boxes = [...container.querySelectorAll('input[type="checkbox"]')];
		expect(boxes).toHaveLength(2);
		expect(boxes[0].hasAttribute('checked')).toBe(true);
		expect(container.textContent).toContain('not yet');
	});

	it('removes template, iframe and base wholesale, contents and all', () => {
		const container = render(
			'Kept.<template><script>alert(1)</script><img src="/t.png"></template><iframe srcdoc="<script>alert(1)</script>"></iframe><base href="https://evil.example/">',
		);
		expect(container.textContent).toContain('Kept.');
		expect(container.innerHTML).not.toContain('<template');
		expect(container.innerHTML).not.toContain('t.png');
		expect(container.innerHTML).not.toContain('<script');
		expect(container.innerHTML).not.toContain('alert');
		expect(container.innerHTML).not.toContain('<iframe');
		expect(container.innerHTML).not.toContain('<base');
		expect(container.innerHTML).not.toContain('evil.example');
	});
});

/**
 * Nothing in a rendered body fetches. A CSP refuses a cross-origin fetch but admits a same-origin
 * one, so every channel here is planted with a same-origin path: the sanitizer must be the gate
 * on its own. Each case keeps a retention face — the text around the channel survives.
 */
describe('sanitizeMarkdownHtml: no fetch channel survives', () => {
	const CHANNELS: [string, string][] = [
		[
			'srcset on an embedded image',
			'<img src="data:image/png;base64,iVBORw0KGgo=" srcset="/fetch 1x">',
		],
		[
			'picture and source',
			'<picture><source srcset="/fetch"><img src="data:image/png;base64,iVBORw0KGgo="></picture>',
		],
		['an image input', '<input type="image" src="/fetch">'],
		['SVG image (href)', '<svg><image href="/fetch"></image></svg>'],
		['SVG image (xlink:href)', '<svg><image xlink:href="/fetch"></image></svg>'],
		['SVG feImage', '<svg><filter><feImage href="/fetch"></feImage></filter></svg>'],
		[
			'a table background',
			'<table background="/fetch"><tr><td background="/fetch">cell</td></tr></table>',
		],
	];

	for (const [channel, html] of CHANNELS) {
		it(`removes ${channel}`, () => {
			const container = render(`Before. ${html} After.`);
			expect(container.textContent).toContain('Before.');
			expect(container.textContent).toContain('After.');
			expect(container.innerHTML).not.toContain('/fetch');
		});
	}

	it('removes what would submit, keeping the words', () => {
		const container = render(
			'<form action="/api/delete" method="post"><input value="1"><button formaction="/api/delete">Click to continue</button></form>',
		);
		expect(container.textContent).toContain('Click to continue');
		for (const gone of ['<form', '<button', '<input', 'action=', '/api/delete']) {
			expect(container.innerHTML).not.toContain(gone);
		}
	});
});

describe('sanitizeMarkdownHtml — body links', () => {
	const ID = '019d4add-f49d-7c43-a87d-dda470e5dd9c';
	const OTHER = '01a16e2a-570d-77fa-9ca5-a905fe467e9a';
	const hrefFor = (id: string) => `/r/${id}`;

	function withLinks(
		markdown: string,
		entries: [string, LinkResolution][] | 'pending' | undefined,
	): HTMLElement {
		const links =
			entries === 'pending' || entries === undefined
				? entries
				: { table: new Map(entries), hrefFor };
		const container = document.createElement('div');
		container.innerHTML = sanitizeMarkdownHtml(prepareMarkdown(markdown), links);
		return container;
	}

	it('a resolved link reads as the held title and follows the surface address', () => {
		const c = withLinks(`See [old hint](./${ID}).`, [[ID, { state: 'resolved', title: 'Held' }]]);
		const a = c.querySelector('a');
		expect(a?.textContent).toBe('Held');
		expect(a?.getAttribute('href')).toBe(`/r/${ID}`);
		expect(a?.hasAttribute('title')).toBe(false);
		expect(c.innerHTML).not.toContain('old hint');
	});

	it('an absent link is inert and carries one sentence for gone and not visible alike', () => {
		const gone = withLinks(`[typed](./${ID})`, [[ID, { state: 'absent' }]]);
		expect(gone.querySelector('a')).toBeNull();
		const span = gone.querySelector('.md-ref-absent');
		expect(span?.textContent).toContain('typed');
		expect(span?.getAttribute('title')).toBe(ABSENT_WORDS);
		// Whatever made it absent, the reader sees the same thing.
		const privateOne = withLinks(`[typed](./${OTHER})`, [[OTHER, { state: 'absent' }]]);
		expect(privateOne.innerHTML).toBe(gone.innerHTML);
	});

	it('an unchecked link stays followable, marked, at the surface address', () => {
		for (const c of [
			withLinks(`[typed](./${ID}/)`, []),
			withLinks(`[typed](./${ID})`, undefined),
		]) {
			const a = c.querySelector('a');
			expect(a?.classList.contains('md-ref-unchecked')).toBe(true);
			expect(a?.getAttribute('title')).toBe(UNCHECKED_WORDS);
			expect(a?.textContent).toContain('typed');
		}
		const unchecked = withLinks(`[t](./${ID})`, []);
		expect(unchecked.querySelector('a')?.getAttribute('href')).toBe(`/r/${ID}`);
		expect(unchecked.querySelector('.md-ref-absent')).toBeNull();
	});

	it('absent and unchecked do not share a mark', () => {
		const absent = withLinks(`[t](./${ID})`, [[ID, { state: 'absent' }]]);
		const unchecked = withLinks(`[t](./${ID})`, []);
		expect(absent.querySelector('.md-ref-marker')?.textContent).not.toBe(
			unchecked.querySelector('.md-ref-marker')?.textContent,
		);
	});

	it('authored state cannot forge a resolved link', () => {
		const forged = withLinks(
			`<a href="./${ID}" class="md-ref-unchecked md-ref" data-ref-state="resolved" title="Held">Held</a> and [plain](./${OTHER})`,
			[[OTHER, { state: 'resolved', title: 'Other' }]],
		);
		const a = forged.querySelector(`a[href="/r/${ID}"]`);
		// The raw-HTML anchor was never in the table, so it is unchecked, with the pass's own
		// class and words — not the author's.
		expect(a?.getAttribute('class')).toBe('md-ref-unchecked');
		expect(a?.getAttribute('title')).toBe(UNCHECKED_WORDS);
		expect(forged.innerHTML).not.toContain('class="md-ref"');
		expect(forged.innerHTML).not.toContain(`href="./${ID}"`);
	});

	it('an author cannot paint the marks on content temper never checked', () => {
		const c = withLinks(
			'<span class="md-ref-absent">x</span> <a href="https://example.com" class="md-ref-unchecked">y</a> <span class="md-ref-marker">z</span>',
			[],
		);
		expect(c.innerHTML).not.toContain('md-ref');
	});

	it("an unanswered link keeps the link's own inline content, marked", () => {
		const c = withLinks(
			`[\`code\` ref](./${ID}) [![pic](data:image/png;base64,iVBORw0KGgo=)](./${OTHER})`,
			[[OTHER, { state: 'absent' }]],
		);
		const unchecked = c.querySelector('a.md-ref-unchecked');
		expect(unchecked?.querySelector('code')?.textContent).toBe('code');
		expect(c.querySelector('.md-ref-absent img')).not.toBeNull();
	});

	it('a held title is text, never markup', () => {
		const c = withLinks(`[t](./${ID})`, [
			[ID, { state: 'resolved', title: '<img src=x onerror=alert(1)> **b**' }],
		]);
		expect(c.querySelector('img')).toBeNull();
		expect(c.querySelector('a')?.textContent).toBe('<img src=x onerror=alert(1)> **b**');
	});

	it('every link to one id settles the same', () => {
		const c = withLinks(`[a](./${ID}) [b](./${ID}) [c](./${ID.toUpperCase()})`, [
			[ID, { state: 'resolved', title: 'Held' }],
		]);
		const anchors = [...c.querySelectorAll('a')];
		expect(anchors).toHaveLength(3);
		for (const a of anchors) expect(a.textContent).toBe('Held');
	});

	it('pending renders inert typed text with no state', () => {
		const c = withLinks(`[typed](./${ID})`, 'pending');
		expect(c.querySelector('a')).toBeNull();
		expect(c.textContent).toContain('typed');
		expect(c.innerHTML).not.toContain('md-ref');
		expect(c.querySelector('[title]')).toBeNull();
	});

	// An SVG anchor is foreign content: putting an HTML span there makes the serialized string
	// parse back into a different DOM than the one the sanitizer approved (the span closes the
	// <svg> on reparse). So a vault-shaped SVG anchor, by `href` or `xlink:href`, becomes its typed
	// text — inert, in place, with no HTML element inside the drawing.
	for (const [label, attr] of [
		['href', 'href'],
		['xlink:href', 'xlink:href'],
	] as const) {
		for (const [state, entries] of [
			['absent', [[ID, { state: 'absent' }]]],
			['unchecked', []],
			['resolved', [[ID, { state: 'resolved', title: 'Held' }]]],
		] as const) {
			it(`an SVG anchor by ${label}, ${state}, becomes inert text and reparses to the same DOM`, () => {
				const html = `<svg><a ${attr}="./${ID}">typed</a><title>t</title></svg>`;
				const c = withLinks(html, entries as unknown as [string, LinkResolution][]);
				const reparsed = document.createElement('div');
				reparsed.innerHTML = c.innerHTML;
				expect(reparsed.innerHTML).toBe(c.innerHTML);
				expect(c.querySelector('svg')?.textContent).toContain('typed');
				expect(c.querySelector('svg a')).toBeNull();
				expect(c.querySelector('svg span')).toBeNull();
			});
		}
	}

	it('leaves non-vault links exactly as they were', () => {
		const md = '[web](https://example.com) [here](#anchor) [rel](./notes.md)';
		const without = document.createElement('div');
		without.innerHTML = sanitizeMarkdownHtml(prepareMarkdown(md));
		expect(withLinks(md, [[ID, { state: 'resolved', title: 'Held' }]]).innerHTML).toBe(
			without.innerHTML,
		);
	});
});
