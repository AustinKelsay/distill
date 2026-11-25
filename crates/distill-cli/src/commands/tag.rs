//! `distill tag <session>`: the owner adds tags to a session or hides tags on it (issue #158).
//! The edits are applied last wherever tags are shown and filtered, and outlive a new card. Only
//! the owner's faces, this command and the window, edit tags; the MCP server can't.

use anyhow::{Result, bail};
use distill_core::tags;

use super::Ctx;
use crate::output::out;

#[derive(clap::Args)]
pub struct Args {
    /// The session: its key (harness/id), or its id when only one harness has it
    session: String,
    /// A tag to add (repeat for more); adding a tag you hid shows it again
    #[arg(long)]
    add: Vec<String>,
    /// A tag to hide (repeat for more); hiding a tag you added takes it off
    #[arg(long)]
    hide: Vec<String>,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    if a.add.is_empty() && a.hide.is_empty() {
        bail!("name a tag with --add or --hide");
    }
    let config = ctx.home.load_config()?;
    let (key, edit) = tags::tag_session(&ctx.home, &config, &a.session, &a.add, &a.hide)?;
    out(
        ctx.json,
        &serde_json::json!({ "session": key, "add": edit.add, "hide": edit.hide }),
        || {
            let list = |t: &[String]| {
                t.iter()
                    .map(|t| format!("#{t}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            match (edit.add.is_empty(), edit.hide.is_empty()) {
                (true, true) => {
                    println!("{key}: no tags of yours; its card's tags show as written")
                }
                _ => {
                    if !edit.add.is_empty() {
                        println!("{key}: added {}", list(&edit.add));
                    }
                    if !edit.hide.is_empty() {
                        println!("{key}: hidden {}", list(&edit.hide));
                    }
                }
            }
        },
    )
}
