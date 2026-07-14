# Domain Docs

Distill uses a single product-domain context even though the implementation may contain multiple applications or packages. Technical package boundaries do not create separate domain contexts by themselves.

## Before Exploring

- Read root `CONTEXT.md` when it exists.
- Read relevant system decisions under `docs/adr/` when they exist.
- Proceed silently when either location does not exist; `domain-modeling` creates them only when language or durable decisions are actually resolved.

## Layout

```text
/
├── CONTEXT.md
└── docs/
    └── adr/
```

`CONTEXT.md` is an implementation-free ubiquitous-language glossary. It must not become a specification, scratch pad, roadmap, or list of technology choices.

`docs/adr/` contains only decisions that are hard to reverse, surprising without context, and the result of a genuine trade-off.

## Consumer Rules

- Use glossary terms in issues, specs, tests, and code-facing descriptions.
- Do not substitute a synonym that `CONTEXT.md` explicitly marks as one to avoid.
- If required language is missing, route it through `domain-modeling` rather than inventing terminology silently.
- Surface any conflict with an ADR explicitly instead of overriding it.
