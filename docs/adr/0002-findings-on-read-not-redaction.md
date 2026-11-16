# Detect sensitive text and mask it on read; never rewrite stored text

Sessions hold secrets and personal details. The earlier build rewrote text at export. That protected only exports, destroyed the original in that copy, and made every rule error permanent in the output.

We record findings beside each session instead, and apply them when a reader asks for text: blurred for the owner, placeholders for models. Stored text never changes, detection can be rerun as it improves, a wrong finding only hides too much, and old secrets stay findable so they can be rotated.

## Consequences

- Every reader must get session text through the one read path, or the findings do nothing.
- Detection misses things, so masking is not what protects against untrusted providers. Refusing them is (ADR 0003).
