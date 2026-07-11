# Distill

Distill is a local conversation refinery for preserving, understanding, curating, and deliberately exporting AI-tool histories. This glossary defines the product language shared by specifications, issues, tests, and implementation.

## Sources And Evidence

**Source**:  
A supported local AI tool installation or local data root that can expose conversation history.  
_Avoid_: Provider, integration

**Capture Candidate**:  
A source item that Distill can attempt to preserve but has not yet accepted into canonical history.  
_Avoid_: Capture, import

**Capture**:  
An immutable source version whose checksum-verified content is owned and recoverable by Distill.  
_Avoid_: Snapshot failure, import

**Normalization Attempt**:  
One parser version's recorded attempt to derive capture facts and a session projection from a capture.  
_Avoid_: Capture status

**Capture Fact**:  
An immutable provider-shaped record derived by a successful normalization attempt.  
_Avoid_: Transcript message, raw record

## Current Conversation View

**Session Identity**:  
The stable combination of source kind and a source-provided or deterministic synthetic session identifier.  
_Avoid_: Database row ID

**Session Projection**:  
The latest successful normalized view of a session, including its metadata, transcript messages, and artifacts.  
_Avoid_: Session history, capture

**Transcript Message**:  
Ordered user-visible text in the current session projection.  
_Avoid_: Every provider event

**Artifact**:  
Structured or non-transcript content associated with a projected message, a capture fact, or both.  
_Avoid_: Attachment

## Human Review And Output

**Curation**:  
A human's reversible session-level tags and policy-driving labels.

**Dataset Label**:  
One of the mutually exclusive export-routing labels `train`, `holdout`, or `exclude`.

**Modifier Label**:  
An orthogonal label such as `sensitive` or `favorite` that changes review or export behavior without selecting a dataset.

**Workflow State**:  
A review and export state derived from the current manual labels.  
_Avoid_: Stored workflow status

**Export Artifact**:  
A versioned file produced from a stable read of eligible current session projections, with durable bookkeeping and audit.  
_Avoid_: Export row

## Operations And Audit

**Activity Event**:  
An append-only domain fact describing a meaningful capture, projection, curation, export, or sync transition.  
_Avoid_: Log, job

**Sync Run**:  
One operational attempt to discover and ingest capture candidates from selected sources.  
_Avoid_: Import, activity event
