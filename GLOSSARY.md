# Distill

A private, searchable archive of every session the owner's local agent harnesses record. It exists so that distilling, training and analysis can start later from a search, without any session being lost or leaked first.

## Language

### Capture

**Harness**:
A local agent program that records sessions on this machine, such as Claude Code or Codex.
_Avoid_: Source, tool, agent app

**Session**:
One recorded run of work between the owner and an agent in a harness.
_Avoid_: Conversation, thread, chat, transcript

**Capture**:
An exact copy of one harness file as it stood at one moment.
_Avoid_: Snapshot, blob, backup

**Raw archive**:
The append-only store of every capture. It is the only part of Distill that cannot be rebuilt.
_Avoid_: Raw, store, vault

**Event**:
One step of a session in Distill's common shape: a message, a tool call, a tool result or reasoning.
_Avoid_: Record, turn, entry

**Mirror**:
A session that repeats another session already in the archive, because one harness ran another.
_Avoid_: Duplicate, copy

**Format drift**:
A harness changing how it records sessions, so that its captures no longer become events.

### Organization

**Card**:
A short model-written description of a session: title, summary, tags and outcome.
_Avoid_: Summary (one field of a card), label, annotation

**Index**:
The rebuildable search structure over events and cards, searchable by exact words and by meaning.
_Avoid_: Database, cache

**Steward**:
The scheduled background work that scans sessions, writes cards and builds the index without being asked.
_Avoid_: Agent, daemon, worker

### Privacy

**Owner**:
The person whose sessions these are, and the only one who approves anything.
_Avoid_: User, human, admin

**Provider**:
Whoever runs a model: a runtime on this machine or a remote service.
_Avoid_: Vendor, backend, harness

**Local provider**:
A provider that runs its models on this machine.

**Trusted provider**:
A provider whose models may read session text. Local providers always are; a remote provider is once the owner tags it.
_Avoid_: Private provider, allowed provider, designated provider

**Reader**:
Whoever asks Distill for session text: the owner, or a model at a named provider.
_Avoid_: Client, caller, consumer

**Read log**:
The record of which reader read which sessions, and when.
_Avoid_: Audit trail, activity

**Finding**:
A detected piece of sensitive text in a session: where it is, what kind it is, and its fingerprint.
_Avoid_: Mark, redaction, hit, detection

**Secret**:
A finding that is a credential: a key, token or password.

**Personal detail**:
A finding that identifies a person or their systems: a name, email address, hostname and the like.
_Avoid_: PII

**Fingerprint**:
A short identifier for a finding's value. It is the same wherever that value appears, and the value cannot be recovered from it.
_Avoid_: Hash, ID

**Placeholder**:
What a model reads in place of a secret: the secret's kind and fingerprint.
_Avoid_: Mask, redaction

**Blur**:
Hiding a finding on screen until the owner clicks it.

**Reveal**:
Releasing a secret's real value to a model, once, after the owner approves that request. The owner clicking a blur is not a reveal.
_Avoid_: Unmask, unredact

**Rotation list**:
The secrets found in the archive, grouped by fingerprint, for the owner to replace.
