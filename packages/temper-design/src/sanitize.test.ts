// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { prepareMarkdown } from './markdown.js';
import { sanitizeMarkdownHtml, WITHHELD_IMAGE } from './sanitize.js';

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

	it('strips DOM-clobbering name attributes from form controls', () => {
		const container = render('<form><input name="attributes"><input name="tagName"></form>');
		expect(container.querySelector('form')).not.toBeNull();
		expect(container.innerHTML).not.toContain('name="attributes"');
		expect(container.innerHTML).not.toContain('name="tagName"');
	});

	it('strips template contents and removes iframe and base wholesale', () => {
		const container = render(
			'Kept.<template><script>alert(1)</script></template><iframe srcdoc="<script>alert(1)</script>"></iframe><base href="https://evil.example/">',
		);
		// DOMPurify keeps an inert empty <template> shell but strips everything inside it;
		// iframe and base are dropped entirely.
		expect(container.textContent).toContain('Kept.');
		expect(container.innerHTML).not.toContain('<script');
		expect(container.innerHTML).not.toContain('alert');
		expect(container.innerHTML).not.toContain('<iframe');
		expect(container.innerHTML).not.toContain('<base');
		expect(container.innerHTML).not.toContain('evil.example');
	});
});
