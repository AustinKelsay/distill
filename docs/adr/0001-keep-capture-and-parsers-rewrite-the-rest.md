# Keep capture and the parsers; rewrite everything else

On 2026-11-15 DISTILL was redefined as a private, searchable archive, and most of the two-day-old build served goals it no longer has. We rewrite from the problem statement and keep three things. The raw archive's on-disk format stays, because the existing archive holds sessions that exist nowhere else. The sweep stays, because its append-versus-rewrite and write-ordering rules are subtle and tested. The eleven parsers stay, because each encodes a harness format learned from real data (12,824 sessions, no failures).

The old redactor's patterns are ported into the detector; its text rewriting is not. Everything else is new code. The previous build's last commit is `a6e4be0`.
