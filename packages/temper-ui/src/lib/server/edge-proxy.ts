import { env } from '$env/dynamic/private';

/**
 * Header that marks a request this server sends to the API, carrying a deployment secret
 * (`TEMPER_EDGE_PROXY_SECRET`). Both of this server's routes to the API send it: the reverse
 * proxy (`proxy.ts`) and the server-side data loaders (`api.ts`).
 *
 * All of that traffic reaches the API from this function's few egress addresses, so a per-IP
 * rate limit at the API's edge would put every user of the UI origin into a handful of shared
 * buckets. The API's firewall rules exempt requests carrying this header and its value; the
 * people behind them already met the UI origin's own per-IP rules, which see their real address.
 *
 * It is an edge-firewall condition only. Nothing in the API reads it, and it confers no access:
 * a leaked value lets its holder skip the API project's per-IP rules on its public host, which are
 * the control for unauthenticated volume there, and nothing else. The proxy always deletes an
 * inbound copy, so a caller cannot pass one through.
 */
export const EDGE_PROXY_HEADER = 'x-temper-edge-proxy';

/**
 * The configured marker value, or `undefined` when this deployment sends none.
 *
 * A value that cannot be a header value (an internal newline from a wrapped paste, a control
 * character) is not sent, and a fixed sentence is logged. Sending it would make `Headers.set`
 * throw on every request, and the runtime's error message quotes the value, so it would both
 * break every forwarded request and write the secret to the logs. Without the marker the
 * requests still go through; they just meet the API's per-IP limits.
 */
export function edgeProxySecret(): string | undefined {
	const value = env.TEMPER_EDGE_PROXY_SECRET?.trim();
	if (!value) return undefined;
	try {
		new Headers().set(EDGE_PROXY_HEADER, value);
	} catch {
		console.error(
			'TEMPER_EDGE_PROXY_SECRET is not a valid header value; not sending the edge-proxy marker',
		);
		return undefined;
	}
	return value;
}
