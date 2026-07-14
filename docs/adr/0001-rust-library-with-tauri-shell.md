# Build the product around a Rust Library with a Tauri shell

Distill will be rebuilt around one deep Rust `Library` module used by the desktop shell, CLI, and contract tests. The desktop will use Tauri 2 with a React and TypeScript renderer: Rust keeps capture, projection, curation, audit, and export integrity in one typed native implementation, while the web renderer provides mature accessibility, virtualization, and UI-testing tools without granting Node or filesystem authority to the UI.

## Considered Options

- A TypeScript Library in an Electron utility process offered the fastest reuse of the legacy implementation and fixtures, but retained a mutable runtime dependency and a larger privileged host.
- A Rust engine with Slint avoided a web renderer, but the experiment exposed weaker accessibility/testing ergonomics and encouraged a large synchronous controller.
- Rust plus Tauri keeps the native Library directly in the host process and makes the renderer a thin, sandboxed caller.

## Consequences

- The Rust workspace starts with one public Library crate rather than separate domain, storage, query, and UI-view-model crates.
- Tauri commands adapt renderer intent to Library methods; they do not become a second domain interface.
- Long-running Library commands are asynchronous, cancellable, and never execute on the renderer event loop.
- The renderer stack is React, TypeScript, Vite, accessible primitives, and virtualized lists/transcripts.
