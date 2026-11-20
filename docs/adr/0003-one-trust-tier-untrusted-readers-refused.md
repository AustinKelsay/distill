# One trust tier; untrusted readers are refused, not served masked text

A provider is either trusted or not: local providers automatically, remote ones when the owner marks them trusted. A reader at an untrusted or unnamed provider gets no session text at all.

## Considered options

- **Serve masked text to untrusted readers.** Rejected: it puts best-effort detection on the most frequent path out of the archive.
- **A second, local-only tier.** Rejected: nothing the owner needs today behaves differently in it.

## Consequences

A reader states its provider when it connects, and DISTILL cannot verify the claim. The rule stops accidental leaks through a harness that was never trusted. It does not stop a process that lies.
