//! Guide: what DISTILL does and how to use it. The window opens here before the first setup;
//! afterwards it stays in the navigation. It reads nothing from disk.

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant};

use crate::State;
use crate::nav::{self, Screen};
use crate::ui::Paint;

/// What DISTILL stands for, word by word; a word that gives a letter starts with it.
pub const ACRONYM: [(&str, bool); 8] = [
    ("Data", true),
    ("Infrastructure", true),
    ("for", false),
    ("Storing,", true),
    ("Tagging,", true),
    ("Indexing", true),
    ("& Logging,", true),
    ("Locally", true),
];

/// How DISTILL works, as (step, what happens).
pub const HOW: [(&str, &str); 4] = [
    (
        "Capture",
        "Every hour DISTILL copies the sessions your coding agents keep (Claude Code, Codex, \
         Cursor and others) into one archive on this machine, so none is lost when an agent \
         deletes old ones.",
    ),
    (
        "Find",
        "It finds secrets (keys, tokens, passwords) and personal details (names, email \
         addresses, hosts) in every session.",
    ),
    (
        "Organize (optional)",
        "A card model you choose, here or at an AI service you trust, writes a title and \
         summary for each session. An embedding model, always run here in Ollama, adds search \
         by meaning. Without either, search by words still works.",
    ),
    (
        "Search",
        "Search finds sessions by words, and by meaning once embeddings are set up. You search \
         and read here; agents at an AI service you trust can too, through the distill command \
         or the MCP server.",
    ),
];

/// Who gets what, as (reader, what it gets).
pub const WHO: [(&str, &str); 3] = [
    (
        "You, in this window",
        "Everything, with secrets and personal details blurred until you click them.",
    ),
    (
        "Trusted AI",
        "Session text with each secret replaced by a placeholder and personal details as \
         written. A secret's value only when you approve the request under Approvals.",
    ),
    (
        "Anyone else",
        "Nothing: every request is refused and logged.",
    ),
];

pub fn open(_: &mut State) {}

pub fn view(s: &State, p: &Paint, _: &mut Vec<El>) -> El {
    let set_up = s.config.is_some();
    let mut how = p.section("how", "How it works", None);
    for (i, (name, what)) in HOW.iter().enumerate() {
        if i > 0 {
            how = how.child(p.separator());
        }
        how = how.child(numbered(p, i + 1, name, what));
    }
    let mut who = p.section("who", "Who reads what", None);
    for (i, (reader, what)) in WHO.iter().enumerate() {
        if i > 0 {
            who = who.child(p.separator());
        }
        who = who.child(
            row()
                .key(format!("who-{i}"))
                .w_full()
                .py(6.0)
                .gap(16.0)
                .child(text(*reader).font(p.k.ty.ui_medium).color(p.ink()).w(160.0))
                .child(text(*what).font(p.text()).color(p.muted()).wrap().shrink()),
        );
    }
    who = who.child(p.note(
        "The Trusted AI step of Setup decides which services count as trusted. Every read, \
         refusal and approval is in the Read log.",
    ));
    let usage = p
        .section("use", "Use it", None)
        .child(line(
            p,
            "In this window",
            "Sessions to search and read, Findings for what was found, Approvals when an agent \
             asks for a secret's value, Read log for every read.",
            None,
        ))
        .child(p.separator())
        .child(line(
            p,
            "In a terminal",
            "Search as a model at a trusted service would see it:",
            Some("distill search \"<words>\" --provider <service>"),
        ))
        .child(p.separator())
        .child(line(
            p,
            "For your agents",
            "Give an agent the MCP server, named with the service it runs at:",
            Some("claude mcp add distill -- distill-mcp --provider <service>"),
        ));
    let start = Button::new(if set_up { "Open Setup" } else { "Start setup" })
        .variant(if set_up {
            ButtonVariant::Neutral
        } else {
            ButtonVariant::Submit
        })
        .on_press(|s: &mut State| nav::go(s, Screen::Setup))
        .build(&p.k)
        .key("start");
    let intro = col()
        .key("heading")
        .gap(4.0)
        .child(text("Guide").key("title").font(p.title()).color(p.ink()))
        .child(
            text("How DISTILL works, and who reads what")
                .font(p.text())
                .color(p.muted()),
        );
    col()
        .key("scroll")
        .w_full()
        .px(36.0)
        .pt(28.0)
        .pb(24.0)
        .gap(18.0)
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(intro)
                .child(start),
        )
        .child(hero(p))
        .child(how)
        .child(who)
        .child(usage)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .h_full()
}

/// The brand: the logo, DISTILL, what it stands for with each letter's word starting in green,
/// and what it is.
fn hero(p: &Paint) -> El {
    let mut words = row().key("acronym").w_full().gap(6.0).flex_wrap();
    for (word, gives_letter) in ACRONYM {
        words = words.child(if gives_letter {
            // "& Logging," gives its L: split before the letter, not at the first character.
            let at = word.find(|c: char| c.is_ascii_uppercase()).unwrap_or(0);
            let (before, rest) = word.split_at(at);
            let (letter, after) = rest.split_at(1);
            let mut w = row().gap(0.0);
            if !before.is_empty() {
                w = w.child(text(before).font(p.text()).color(p.muted()));
            }
            w.child(text(letter).font(p.k.ty.ui_medium).color(p.brand))
                .child(text(after).font(p.text()).color(p.ink()))
        } else {
            row().child(text(word).font(p.text()).color(p.muted()))
        });
    }
    p.card()
        .key("hero")
        .role("group")
        .label("DISTILL: Data Infrastructure for Storing, Tagging, Indexing & Logging, Locally")
        .padding(24.0)
        .radial_gradient(
            [0.0, 0.0],
            [420.0, 260.0],
            [(0.0, p.brand.alpha(0.16)), (1.0, p.brand.alpha(0.0))],
        )
        .child(
            row()
                .w_full()
                .gap(18.0)
                .center_y()
                .child(p.logo(64.0))
                .child(
                    col()
                        .gap(6.0)
                        .grow(1.0)
                        .shrink()
                        .child(
                            text("DISTILL")
                                .key("wordmark")
                                .font(p.wordmark(30.0))
                                .color(p.ink())
                                .a11y_hidden(),
                        )
                        .child(words.a11y_hidden())
                        .child(
                            text("A private, searchable archive of your coding agents' sessions")
                                .font(p.caption())
                                .color(p.muted()),
                        ),
                ),
        )
}

fn numbered(p: &Paint, n: usize, name: &str, what: &str) -> El {
    row()
        .key(format!("how-{n}"))
        .w_full()
        .py(6.0)
        .gap(14.0)
        .child(
            row()
                .w(24.0)
                .h(24.0)
                .center()
                .radius(12.0)
                .fill(p.brand_soft())
                .border(p.hair, p.brand.alpha(0.35))
                .child(text(n.to_string()).font(p.k.ty.ui_medium).color(p.brand))
                .a11y_hidden(),
        )
        .child(
            col()
                .gap(2.0)
                .grow(1.0)
                .shrink()
                .child(text(name).font(p.k.ty.ui_medium).color(p.ink()))
                .child(text(what).font(p.text()).color(p.muted()).wrap()),
        )
}

fn line(p: &Paint, name: &str, what: &str, command: Option<&str>) -> El {
    let mut c = col()
        .gap(6.0)
        .grow(1.0)
        .shrink()
        .child(text(what).font(p.text()).color(p.muted()).wrap());
    if let Some(cmd) = command {
        c = c.child(
            row()
                .w_full()
                .h(32.0)
                .px(10.0)
                .center_y()
                .radius(7.0)
                .fill(p.t.control_track)
                .child(text(cmd).font(p.mono()).color(p.ink()).truncate()),
        );
    }
    row()
        .w_full()
        .py(6.0)
        .gap(16.0)
        .child(text(name).font(p.k.ty.ui_medium).color(p.ink()).w(160.0))
        .child(c)
}
