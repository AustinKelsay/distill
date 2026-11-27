use anyhow::Result;
use distill_core::cards::CardProgress;
use distill_core::status;

use super::Ctx;
use crate::output::{human_bytes, out};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let s = status::status(&ctx.home)?;
    out(ctx.json, &s, || {
        if !s.configured {
            println!("DISTILL is not set up here yet: run `distill init`.");
        }
        println!(
            "{} sessions · {} archived · last sweep: {}",
            s.sessions,
            human_bytes(s.archive_bytes),
            s.last_sweep.as_deref().unwrap_or("never")
        );
        println!(
            "scan: {} of {} sessions current, {} waiting · {} secrets and {} personal details found",
            s.scan.current,
            s.scan.sessions,
            s.scan.pending,
            s.scan.secrets,
            s.scan.personal_details
        );
        println!(
            "word index: {} of {} sessions current, {} waiting · {}",
            s.index.current,
            s.index.sessions,
            s.index.pending,
            human_bytes(s.index.bytes)
        );
        println!("{}", cards_line(&s.cards));
        match &s.vectors.skipped {
            Some(why) => println!(
                "vectors: skipped ({why}) · {} of {} sessions have them · search by meaning is off",
                s.vectors.current, s.vectors.sessions
            ),
            None => println!(
                "vectors: {} of {} sessions current, {} waiting · {} · model {}{}",
                s.vectors.current,
                s.vectors.sessions,
                s.vectors.pending,
                human_bytes(s.vectors.bytes),
                s.vectors.model.as_deref().unwrap_or("none"),
                stopped(s.vectors.stopped_after)
            ),
        }
        match &s.details.skipped {
            Some(why) => println!(
                "model detection: skipped ({why}) · {} of {} sessions current",
                s.details.current, s.details.sessions
            ),
            None => println!(
                "model detection: {} of {} sessions current, {} waiting, {} to retry · {} found · model {}{}",
                s.details.current,
                s.details.sessions,
                s.details.pending,
                s.details.failing,
                distill_core::util::count(s.details.found, "personal detail"),
                s.details.model.as_deref().unwrap_or("none"),
                stopped(s.details.stopped_after)
            ),
        }
        println!(
            "reveals: {} waiting for the owner in the GUI",
            distill_core::util::count(s.pending_reveals, "request")
        );
        for t in &s.backups {
            println!(
                "backup {}: last whole run {}{}",
                t.name,
                t.last_ok.as_deref().unwrap_or("never"),
                t.captures_behind
                    .map_or(String::new(), |n| format!(", {n} captures behind"))
            );
        }
        for line in left_lines(&s.left) {
            println!("{line}");
        }
        for h in &s.harnesses {
            println!("{}", harness_line(h));
        }
        for m in &s.machines {
            println!(
                "machine {}{}: {} · {} sessions · last capture {} · last pull {}{}",
                m.machine,
                m.host
                    .as_deref()
                    .map_or(String::new(), |h| format!(" ({h})")),
                format!("{:?}", m.health).to_lowercase(),
                m.sessions,
                m.last_capture.as_deref().unwrap_or("never"),
                m.last_pull.as_deref().unwrap_or("never"),
                m.note
                    .as_deref()
                    .map_or(String::new(), |n| format!(" · {n}"))
            );
            for h in &m.harnesses {
                println!("  {}", harness_line(h));
            }
        }
        for w in &s.warnings {
            println!("warning: {}", w.message);
        }
    })
}

/// A harness's row: its health, sessions, captures, bytes and note.
fn harness_line(h: &status::HarnessStatus) -> String {
    format!(
        "  {:<12} {:<8} {:>6} sessions {:>6} captures {:>10}  {}",
        h.harness.id(),
        format!("{:?}", h.health).to_lowercase(),
        h.sessions,
        h.captures,
        human_bytes(h.archive_bytes),
        h.note.as_deref().unwrap_or("")
    )
}

/// The cards' line. A stale card (written with another prompt version or `card_model`) is still
/// shown, so it counts among the sessions that have one, apart from the current and the waiting.
fn cards_line(c: &CardProgress) -> String {
    match &c.skipped {
        Some(why) => format!(
            "cards: skipped ({why}) · {} of {} sessions have one",
            c.current + c.stale,
            c.sessions
        ),
        None => format!(
            "cards: {} of {} sessions current, {} waiting, {} stale, {} to retry · model {}{}",
            c.current,
            c.sessions,
            c.pending,
            c.stale,
            c.failing,
            c.model.as_deref().unwrap_or("none"),
            stopped(c.stopped_after)
        ),
    }
}

/// What a model step's line adds when the last tick stopped it at its time limit.
fn stopped(after: Option<u64>) -> String {
    after.map_or_else(String::new, |n| {
        format!(
            " · the last tick stopped it at its time limit after {}",
            distill_core::util::count(n, "session")
        )
    })
}

/// What left this machine, a line per place, each span as "last day · last 7 days" (#162).
fn left_lines(left: &distill_core::readlog::Left) -> Vec<String> {
    use distill_core::readlog::{PlaceKind, Sent};
    use distill_core::util::count;
    let read = |s: &Sent| {
        let mut t = count(s.times, "read");
        if s.sessions > 0 {
            t += &format!(", {}", count(s.sessions, "session"));
        }
        if s.listed > 0 {
            t += &format!(", {} listed", s.listed);
        }
        t
    };
    let ran = |s: &Sent| format!("{}, {}", count(s.times, "run"), human_bytes(s.bytes));
    let exported = |s: &Sent| format!("{}, {}", count(s.times, "export"), human_bytes(s.bytes));
    let mut lines = vec!["left this machine (last day · last 7 days):".to_string()];
    if left.places.is_empty() {
        lines.push("  nothing in the last 7 days".into());
    }
    for p in &left.places {
        let (name, f): (String, &dyn Fn(&Sent) -> String) = match p.kind {
            PlaceKind::Reader => (p.name.clone(), &read),
            PlaceKind::Backup => (format!("backup {}", p.name), &ran),
            PlaceKind::Hub => ("the hub, over ssh".into(), &exported),
        };
        lines.push(format!(
            "  {name}: {} · {}",
            f(&p.spans.day),
            f(&p.spans.week)
        ));
    }
    lines.push(format!(
        "  stayed on this machine: {} · {}",
        read(&left.here.day),
        read(&left.here.week)
    ));
    if left.maybe_short {
        lines.push("  the read log has been edited, so these counts may be short".into());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cards_line_counts_stale_cards_apart_from_missing_ones() {
        let mut c = CardProgress {
            model: Some("small:8b".into()),
            sessions: 10,
            current: 5,
            stale: 3,
            pending: 2,
            failing: 1,
            ..Default::default()
        };
        assert_eq!(
            cards_line(&c),
            "cards: 5 of 10 sessions current, 2 waiting, 3 stale, 1 to retry · model small:8b"
        );
        c.skipped = Some("the local runtime is not running".into());
        assert_eq!(
            cards_line(&c),
            "cards: skipped (the local runtime is not running) · 8 of 10 sessions have one"
        );
    }
}
