# A secret reaches a model only when the owner approves that request in the GUI

Models read placeholders by default, including models at trusted providers. To get a real value, a model files a reveal request naming specific findings. The owner approves or denies it in the GUI, and an approval releases those values once.

We rejected a per-provider "may read secrets" switch. Archived sessions contain web pages and tool output that can carry planted instructions, so a model asking for a secret is not evidence that the owner asked. Approval exists only in the GUI because an agent with a shell could run a command-line approval itself.

This governs reveals made through DISTILL. A process running as the owner can still read stored text directly, which ADR 0005 puts outside what DISTILL defends.
