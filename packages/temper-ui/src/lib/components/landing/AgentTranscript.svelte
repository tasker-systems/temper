<div class="transcript">
  <div class="message">
    <div class="role">you</div>
    <div class="content user">/temper task start api-v2-migration</div>
  </div>

  <div class="message">
    <div class="role">agent</div>
    <div class="content agent">
      <div class="agent-text">Loading task context...</div>
      <div class="context-summary">
        Goal: api-v2-migration (3 tasks, 2 complete)<br />
        Prior sessions: 4 (last: Mar 28 — auth middleware)<br />
        Key decisions: REST over GraphQL, JWT rotation<br />
        Deferred: rate limiting, webhook signatures
      </div>
      <div class="agent-text">
        This is a <span class="highlight">build/medium</span> task. Based on prior sessions,
        you've completed the auth middleware and route migration. The remaining work is the
        client SDK update.
      </div>
      <div class="agent-text">
        I've read the research doc and the two deferred decisions. Ready to plan the implementation.
      </div>
    </div>
  </div>

  <div class="message">
    <div class="role">you</div>
    <div class="content user">let's start with the client SDK</div>
  </div>
</div>

<style>
  .transcript {
    border: 1px solid rgba(255, 255, 255, 0.06);
    padding: 1.2rem;
    margin-top: 1.5rem;
    font-family: var(--mono);
    font-size: 0.7rem;
    line-height: 1.8;
  }

  .message {
    margin-bottom: 1.2rem;
  }

  .message:last-child {
    margin-bottom: 0;
  }

  .message + .message {
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    padding-top: 1rem;
  }

  .role {
    color: var(--blue-dim);
    font-size: 0.6rem;
    margin-bottom: 0.4rem;
  }

  .content.user {
    color: rgba(255, 255, 255, 0.7);
  }

  .content.agent {
    font-family: var(--serif);
    font-size: 0.8rem;
    line-height: 1.7;
  }

  .agent-text {
    color: rgba(255, 255, 255, 0.55);
    margin-bottom: 0.6rem;
  }

  .agent-text:last-child {
    margin-bottom: 0;
  }

  .context-summary {
    font-family: var(--mono);
    font-size: 0.65rem;
    color: rgba(255, 255, 255, 0.35);
    margin-bottom: 0.8rem;
    padding: 0.6rem;
    background: rgba(255, 255, 255, 0.02);
    border-left: 2px solid rgba(126, 184, 218, 0.3);
    line-height: 1.8;
  }

  .highlight {
    color: var(--blue);
  }
</style>
