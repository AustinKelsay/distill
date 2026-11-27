//! `distill findings`: the rotation list, or every session one fingerprint appears in, as the
//! owner. Answers from stored findings through the read path; prints fingerprints, kinds and
//! session facts, never a value or session text.

use anyhow::{Result, bail};
use distill_core::read::{self, Answer, FindingFilter, Kinds, Order, PAGE_MAX, Page};
use distill_core::runtime::LocalRuntime;
use distill_core::trust::Reader;

use super::{Ctx, parse_harness};
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// A fingerprint, or its short form: list every session it appears in
    fingerprint: Option<String>,
    /// `secrets` (the rotation list), `personal`, `all`, or one kind, such as `github_token`
    #[arg(long, default_value = "secrets")]
    kind: String,
    /// Only this harness's sessions
    #[arg(long)]
    harness: Option<String>,
    /// Only this machine's sessions: a spoke by its name, or `hub` for this hub's own
    #[arg(long)]
    machine: Option<String>,
    /// Only sessions that started on or after this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    since: Option<String>,
    /// Only sessions that started on or before this day, YYYY-MM-DD (UTC)
    #[arg(long)]
    until: Option<String>,
    /// `spread` (most sessions first) or `recent` (most recently seen first)
    #[arg(long, default_value = "spread")]
    sort: String,
    /// How many fingerprints to skip
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// How many fingerprints at most
    #[arg(long, default_value_t = PAGE_MAX)]
    limit: usize,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let runtime = LocalRuntime::ollama();
    if let Some(asked) = &a.fingerprint {
        return match read::where_found(&ctx.home, &runtime, &Reader::Owner, asked)? {
            Answer::Served(l) => out(ctx.json, &l, || print!("{}", l.text())),
            Answer::Refused(why) => bail!("refused: {why}"),
        };
    }
    let day = |d: &Option<String>| {
        d.as_deref()
            .map(|d| read::date(d).map_err(|_| anyhow::anyhow!("not a day as YYYY-MM-DD: {d:?}")))
            .transpose()
    };
    let filter = FindingFilter {
        kinds: Kinds::parse(&a.kind)?,
        harness: a.harness.as_deref().map(parse_harness).transpose()?,
        machine: a.machine,
        since: day(&a.since)?,
        until: day(&a.until)?,
    };
    let order = Order::parse(&a.sort)?;
    let page = Page {
        offset: a.offset,
        limit: a.limit,
    };
    match read::fingerprints(&ctx.home, &runtime, &Reader::Owner, &filter, order, page)? {
        Answer::Served(f) => out(ctx.json, &f, || print!("{}", f.text())),
        Answer::Refused(why) => bail!("refused: {why}"),
    }
}
