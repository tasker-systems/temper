import { defineAgent } from "eve";

export default defineAgent({
  model: "anthropic/claude-sonnet-4.5",
  description:
    "The @temper Slack agent: answers app mentions in a team's workspace. T1 proves the inbound pipe — it resolves the mentioning Slack user to an opaque eve principal and prompts to connect a temper account. Temper reach arrives in a later task.",
});
