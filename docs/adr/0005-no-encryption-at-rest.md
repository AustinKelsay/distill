# No encryption at rest; the bar is "no more exposed than the harness folders"

The archive concentrates every credential that ever crossed a terminal, so it must not become an extra way in. The bar is that Distill adds no exposure beyond what the harnesses' own folders already have. We do not encrypt the archive.

Encryption would protect only sessions a harness has already pruned, since the rest sit readable in the harness folders, and it would end plain files as the source of truth. A process running as the owner can read the archive directly. That is outside what Distill defends.

## Consequences

Distill meets the bar another way: owner-only file modes, no network listener, secrets kept out of everything derived from sessions, fingerprints that cannot be reversed, and a rotation list. It relies on the machine's disk encryption and on backups being encrypted.
