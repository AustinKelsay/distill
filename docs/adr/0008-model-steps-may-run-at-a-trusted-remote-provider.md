# The model steps may run at a trusted remote provider; embeddings stay local

Cards and the personal-details pass need a capable model, and running one on the owner's machine every hour is costly. `[steward.generation]` lets them run at one remote provider over an OpenAI-compatible HTTPS API. The provider must be marked trusted; otherwise those two steps fail and the rest of the tick still runs.

The model reads sessions through the read path like any trusted reader (ADR 0003): secrets as placeholders, personal details in place, one `served` line per session in the read log. Embeddings stay in Ollama on this machine, so search by meaning never sends a query out.

## Consequences

- DISTILL now makes outbound connections to that provider. It still opens no listener, and ADR 0005's bar is unchanged: nothing the owner has not trusted reads session text.
- Trusting a provider for generation is the same decision as trusting it for MCP reads. There is no separate, narrower trust.
- Without `[steward.generation]`, everything runs in Ollama as before.
