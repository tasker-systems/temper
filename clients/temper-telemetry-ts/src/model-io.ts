/**
 * The one place model-I/O recording is decided.
 *
 * The AI SDK's `experimental_telemetry` defaults `recordInputs`/`recordOutputs` to
 * `true` — full model message history on every step span, exported wherever spans
 * go. Every temper agent reads a user's data under that user's own credential, so
 * the default is exactly backwards: the rule is fail-CLOSED, and it lives here so
 * an agent that forgets to spread it fails a test instead of exporting message
 * history to the span backend.
 *
 * Spread into every `defineInstrumentation` call:
 *
 * ```ts
 * export default defineInstrumentation({ ...NEVER_RECORD_MODEL_IO, setup });
 * ```
 *
 * Widening this for one agent is a decision about exporting user content to a
 * third-party backend — it means changing this constant's consumers deliberately,
 * not defaulting a new agent into the SDK's own default. Each agent's test suite
 * asserts its instrumentation still pins both fields to `false`.
 */
export const NEVER_RECORD_MODEL_IO = {
	recordInputs: false,
	recordOutputs: false
} as const;
