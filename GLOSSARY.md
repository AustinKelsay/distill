# DISTILL

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
An exact copy of one harness file as it stood at one moment, or one session exported from a harness's database.
_Avoid_: Snapshot, blob, backup

**Capture hash**:
The content hash that names a capture in the raw archive.
_Avoid_: Fingerprint (a finding's), checksum

**Raw archive**:
The append-only store of every capture. Everything DISTILL knows about a session can be rebuilt from it, and it cannot be rebuilt from anything but a backup of it.
_Avoid_: Raw, store, vault

**Event**:
One step of a session in DISTILL's common shape: a message, a tool call, a tool result or reasoning.
_Avoid_: Record, turn, entry

**Session facts**:
What DISTILL records about a session without holding any of its text: harness, models, project, branch, times and counts.
_Avoid_: Metadata, meta

**Project**:
The folder a session worked in.
_Avoid_: Repo, workspace, cwd

**Mirror**:
A session that repeats another session already in the archive, because one harness ran another.
_Avoid_: Duplicate, copy

**Started from**:
For a session an agent started (a sub-agent, a delegated task), the session the owner started that led to it: the top of its chain of parents.
_Avoid_: Root, thread, origin

**Format drift**:
A harness changing how it records sessions, so that its captures, or some of their records, no longer become events. Seen as a sizable capture that parses to nothing, or as records of a type the parser doesn't know.

### Organization

**Card**:
A short model-written description of a session: title, summary, tags and outcome.
_Avoid_: Summary (one field of a card), label, annotation

**Stale card**:
A card written with another prompt or model than the steward's now. It still describes its session, so it is shown, and the steward writes it again once no session is waiting for a card. A card whose session has changed since is out of date instead, and is not shown.
_Avoid_: Old card, outdated card

**Index**:
The rebuildable search structure over events and cards, searchable by exact words and by meaning.
_Avoid_: Database, cache

**Steward**:
The scheduled background work that scans sessions, writes cards and builds the index without being asked.
_Avoid_: Agent, daemon, worker

**Tick**:
One scheduled run of the steward's steps, from sweep to the last model step.
_Avoid_: Run, job, cycle

**Run log**:
The record of what each tick did, and what it skipped or failed.
_Avoid_: Activity, history

### Privacy

**Owner**:
The person whose sessions these are, and the only one who approves anything.
_Avoid_: User, human, admin

**Provider**:
Whoever runs a model: a runtime on this machine or a remote service. The window lists them under Trusted AI.
_Avoid_: Vendor, backend, harness

**Local provider**:
A provider that runs its models on this machine.

**Trusted provider**:
A provider whose models may read session text. Local providers always are; a remote provider is once the owner marks it trusted.
_Avoid_: Private provider, allowed provider, designated provider

**Reader**:
Whoever asks DISTILL for session text: the owner, a model at a named provider, or DISTILL itself for work that uses no model.
_Avoid_: Client, caller, consumer

**Read log**:
The record of which reader read which sessions, and when.
_Avoid_: Audit trail, activity

**Read path**:
The one way session text leaves DISTILL readable. It decides what a given reader gets and records the read. A backup is the only other way out, and leaves only as ciphertext.
_Avoid_: API, accessor, gate

**Detector**:
The part of DISTILL that examines text and produces findings.
_Avoid_: Redactor, scanner, scrubber

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

### Backups

**Backup**:
An encrypted copy of the backup set at a backup target, made on this machine with the backup key. A backup is not a read: no reader gets session text from it.
_Avoid_: Snapshot, export, sync

**Backup set**:
What a backup holds: everything in the home that cannot be rebuilt, and optionally cards and vectors.

**Backup target**:
One place a backup goes, set up as one `[[backup]]` entry: a folder, another machine, an object store. It holds only ciphertext under keyed names.
_Avoid_: Store, remote, destination

**Adapter**:
What moves opaque, encrypted objects to and from one kind of backup target, through a small write-once contract. It never sees a key.
_Avoid_: Plug-in, driver, backend

**Backup key**:
The key that encrypts and names a home's backups. It is separate from the fingerprint key, and the target never sees it.

**Recovery key**:
The backup key as given to the owner at setup, to keep somewhere other than this machine. Without it or the home's copy, the backups cannot be opened.
_Avoid_: Recovery phrase, password

**Backup log**:
The record of each backup run, per backup target: when, what was sent and what failed.

**Restore**:
Rebuilding a home from a backup, into an empty home only. The only way a backup comes back in.
