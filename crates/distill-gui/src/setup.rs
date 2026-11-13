//! The one-time Setup flow: five steps over a draft `Config`, written on Finish.

use distill_core::config::{Config, Harness, StewardConfig};
use distill_core::source::{Detection, SourceKind};
use rune::{El, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, SegmentedControl, Switch, TextInput};

use crate::State;
use crate::data::{human_bytes, thousands, tilde};
use crate::ui::{Paint, dot, pill};

/// (name, what the step decides).
pub const STEPS: [(&str, &str); 5] = [
    ("Sources", "Which agents' sessions to archive"),
    ("Storage", "Where the archive lives"),
    ("Agent", "Who writes session cards"),
    ("Privacy", "What never gets archived"),
    ("Schedule", "How often it runs"),
];

pub const HARNESSES: [(Harness, &str); 3] = [
    (Harness::Claude, "Claude Code"),
    (Harness::Codex, "Codex"),
    (Harness::None, "None"),
];
pub const BUDGETS: [f64; 5] = [0.5, 1.0, 2.0, 5.0, 10.0];
pub const SWEEPS: [(u32, &str); 4] = [
    (15, "15 min"),
    (30, "30 min"),
    (60, "1 hour"),
    (180, "3 hours"),
];
pub const ENRICHES: [(u32, &str); 4] = [
    (6, "6 hours"),
    (12, "12 hours"),
    (24, "Daily"),
    (168, "Weekly"),
];

/// Setup's working copy. Nothing reaches disk until Finish.
pub struct SetupDraft {
    pub step: usize,
    pub config: Config,
    pub model: TextEdit,
    pub exclude: TextEdit,
    /// Opened from the dashboard over a saved config, so it can be left without saving.
    pub cancellable: bool,
}

impl SetupDraft {
    pub fn from_config(config: &Config, cancellable: bool) -> SetupDraft {
        let mut config = config.clone();
        // Every source gets a row, even ones an older config doesn't list.
        for kind in SourceKind::ALL {
            if config.source(kind).is_none() {
                config.source_mut(kind).enabled = false;
            }
        }
        let model = TextEdit::new(config.steward.model.clone().unwrap_or_default());
        SetupDraft {
            step: 0,
            config,
            model,
            exclude: TextEdit::new(""),
            cancellable,
        }
    }

    pub fn last(&self) -> bool {
        self.step + 1 == STEPS.len()
    }
    pub fn next(&mut self) {
        self.step = (self.step + 1).min(STEPS.len() - 1);
    }
    pub fn back(&mut self) {
        self.step = self.step.saturating_sub(1);
    }

    pub fn enabled(&self, kind: SourceKind) -> bool {
        self.config.source(kind).is_some_and(|s| s.enabled)
    }
    pub fn toggle_source(&mut self, kind: SourceKind) {
        let s = self.config.source_mut(kind);
        s.enabled = !s.enabled;
    }

    /// Pick a harness. A model left at the old harness's default follows to the new one's.
    pub fn set_harness(&mut self, h: Harness) {
        let default_for = |h: Harness| match h {
            Harness::Claude => StewardConfig::default().model.unwrap_or_default(),
            _ => String::new(),
        };
        if self.model.text.trim() == default_for(self.config.steward.harness) {
            self.model.replace_text(default_for(h));
        }
        self.config.steward.harness = h;
    }

    /// Add what the exclude field holds to the never-archive list.
    pub fn add_exclude(&mut self) {
        let path = self.exclude.text.trim().to_string();
        if path.is_empty() {
            return;
        }
        if !self.config.privacy.exclude_paths.contains(&path) {
            self.config.privacy.exclude_paths.push(path);
        }
        self.exclude.clear();
    }

    /// The config Finish writes.
    pub fn finish(&self) -> Config {
        let mut c = self.config.clone();
        let model = self.model.text.trim();
        c.steward.model = if model.is_empty() {
            None
        } else {
            Some(model.to_string())
        };
        c
    }
}

// ---------------------------------------------------------------------------------------------
// View

pub fn view(s: &State, p: &Paint) -> El {
    let d = &s.setup;
    let (name, about) = STEPS[d.step];
    let body = match d.step {
        0 => sources(s, p),
        1 => storage(s, p),
        2 => agent(d, p),
        3 => privacy(d, p),
        _ => schedule(s, p),
    };
    let content = col()
        .key("content")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(
            col()
                .key("scroll")
                .w_full()
                .px(40.0)
                .pt(36.0)
                .pb(24.0)
                .gap(20.0)
                .child(
                    col()
                        .key("heading")
                        .gap(4.0)
                        .child(
                            text(format!("Step {} of {}", d.step + 1, STEPS.len()))
                                .font(p.caption())
                                .color(p.faint()),
                        )
                        .child(text(name).key("step-title").font(p.title()).color(p.ink()))
                        .child(text(about).font(p.text()).color(p.muted())),
                )
                .child(body.key(format!("step-{}", d.step)))
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .grow(1.0)
                .min_h(0.0),
        )
        .child(footer(d, p));
    let rule = el().w(p.hair).h_full().fill(p.t.separator).a11y_hidden();
    row()
        .key("setup")
        .w_full()
        .h_full()
        .child(rail(d, p))
        .child(rule)
        .child(content)
}

fn rail(d: &SetupDraft, p: &Paint) -> El {
    let mut steps = col()
        .key("steps")
        .gap(2.0)
        .role("list")
        .label("Setup steps");
    for (i, (name, _)) in STEPS.iter().enumerate() {
        let current = i == d.step;
        let done = i < d.step;
        let accent = rune::tok::accent(0, p.dark);
        let badge = if done {
            el().size(20.0, 20.0)
                .radius(10.0)
                .fill(accent)
                .center()
                .child(icon(p.k.glyph.check_small, 12.0).color(rune::Color::WHITE))
        } else {
            el().size(20.0, 20.0)
                .radius(10.0)
                .fill(if current {
                    accent
                } else {
                    rune::Color::TRANSPARENT
                })
                .border(if current { 0.0 } else { 1.0 }, p.t.separator)
                .center()
                .child(
                    text((i + 1).to_string())
                        .font(p.k.ty.small_medium.lh(12.0))
                        .color(if current {
                            rune::Color::WHITE
                        } else {
                            p.muted()
                        }),
                )
        };
        steps = steps.child(
            p.k.interactive(
                row()
                    .key(format!("rail-{i}"))
                    .role("button")
                    .label(*name)
                    .checked(current)
                    .h(34.0)
                    .px(10.0)
                    .gap(10.0)
                    .center_y()
                    .radius(8.0)
                    .fill(if current {
                        p.t.hover
                    } else {
                        rune::Color::TRANSPARENT
                    })
                    .focusable(true)
                    .on_press(move |s: &mut State| s.setup.step = i)
                    .child(badge.a11y_hidden())
                    .child(
                        text(*name)
                            .font(if current { p.k.ty.ui_medium } else { p.text() })
                            .color(if current || done { p.ink() } else { p.muted() })
                            .a11y_hidden(),
                    ),
            ),
        );
    }
    col()
        .key("rail")
        .w(220.0)
        .h_full()
        .px(16.0)
        .pt(36.0)
        .gap(24.0)
        .fill(p.t.card)
        .child(
            col()
                .px(10.0)
                .gap(2.0)
                .child(text("Distill").font(p.title()).color(p.ink()))
                .child(
                    text("Set up your session archive")
                        .font(p.caption())
                        .color(p.muted()),
                ),
        )
        .child(steps)
}

fn footer(d: &SetupDraft, p: &Paint) -> El {
    let mut left = row().gap(8.0);
    if d.cancellable {
        left = left.child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.screen = crate::Screen::Dashboard)
                .build(&p.k)
                .key("cancel"),
        );
    }
    let back = Button::new("Back")
        .disabled(d.step == 0)
        .on_press(|s: &mut State| s.setup.back())
        .build(&p.k)
        .key("back");
    let next = if d.last() {
        Button::new("Finish")
            .variant(ButtonVariant::Submit)
            .on_press(|s: &mut State| s.finish_setup())
            .build(&p.k)
            .key("finish")
    } else {
        Button::new("Next")
            .variant(ButtonVariant::Submit)
            .on_press(|s: &mut State| s.setup.next())
            .build(&p.k)
            .key("next")
    };
    let bar = row()
        .key("footer")
        .w_full()
        .h(60.0)
        .px(40.0)
        .center_y()
        .justify_between()
        .child(left)
        .child(row().gap(8.0).child(back).child(next));
    col().w_full().child(p.separator()).child(bar)
}

/// A labelled setting: name and explanation on the left, its control on the right.
fn setting(p: &Paint, key: &str, name: &str, help: &str, control: El) -> El {
    row()
        .key(key)
        .w_full()
        .py(12.0)
        .gap(24.0)
        .center_y()
        .justify_between()
        .child(
            col()
                .gap(2.0)
                .grow(1.0)
                .shrink()
                .child(text(name).font(p.k.ty.ui_medium).color(p.ink()))
                .child(text(help).font(p.caption()).color(p.muted()).wrap()),
        )
        .child(control)
}

fn sources(s: &State, p: &Paint) -> El {
    let d = &s.setup;
    let mut card = p.card().key("sources").gap(0.0).py(4.0);
    for (i, kind) in SourceKind::ALL.into_iter().enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        let found = s
            .detections
            .as_ref()
            .and_then(|v| v.iter().find(|x| x.kind == kind));
        card = card.child(source_row(
            p,
            kind,
            d.enabled(kind),
            found,
            &d.config.roots_for(kind),
            s.detections.is_some(),
        ));
    }
    col()
        .gap(12.0)
        .child(card)
        .child(
            text("Raw session files are copied byte for byte and never edited. Sources without a parser are still archived, so nothing is lost while one is written.")
                .font(p.caption())
                .color(p.muted())
                .wrap(),
        )
}

fn source_row(
    p: &Paint,
    kind: SourceKind,
    on: bool,
    found: Option<&Detection>,
    roots: &[std::path::PathBuf],
    scanned: bool,
) -> El {
    let where_ = roots.first().map(|r| tilde(r)).unwrap_or_default();
    let status = match found {
        Some(f) if f.found => text(format!(
            "{} · {} {}",
            human_bytes(f.bytes),
            thousands(f.files),
            if f.files == 1 { "file" } else { "files" }
        ))
        .font(p.k.ty.ui.features("'tnum' 1"))
        .color(p.ink()),
        Some(_) => text("not found").font(p.text()).color(p.faint()),
        None if scanned => text("not found").font(p.text()).color(p.faint()),
        None => text("scanning…").font(p.text()).color(p.faint()),
    };
    let mut right = row().gap(10.0).center_y().child(status.key("status"));
    if !kind.has_parser() {
        right =
            right.child(pill(p, "no parser yet, archived only", p.k.c.warning).key("no-parser"));
    }
    let present = found.is_some_and(|f| f.found);
    row()
        .key(format!("source-{}", kind.id()))
        .w_full()
        .h(56.0)
        .px(4.0)
        .gap(14.0)
        .center_y()
        .child(
            Switch::new(on)
                .label(kind.label())
                .hide_label()
                .control_size(ControlSize::Small)
                .on_toggle(move |s: &mut State| s.setup.toggle_source(kind))
                .build(&p.k)
                .key("switch"),
        )
        .child(dot(p.source_color(kind), 8.0).opacity(if present { 1.0 } else { 0.35 }))
        .child(
            col()
                .grow(1.0)
                .shrink()
                .gap(1.0)
                .child(text(kind.label()).font(p.k.ty.ui_medium).color(if on {
                    p.ink()
                } else {
                    p.muted()
                }))
                .child(
                    text(where_)
                        .font(p.k.ty.small.mono().lh(15.0))
                        .color(p.faint())
                        .truncate(),
                ),
        )
        .child(right)
}

fn storage(s: &State, p: &Paint) -> El {
    let root = tilde(&s.home.root);
    let layout = [
        ("raw/", "exact source bytes, content-addressed"),
        ("sessions/", "normalized sessions and their cards"),
        ("recipes/", "dataset definitions"),
        ("exports/", "dataset builds, held until you approve them"),
        ("runs/", "what each sweep and steward run did"),
    ];
    let mut tree = col().gap(6.0);
    for (dir, what) in layout {
        tree = tree.child(
            row()
                .gap(12.0)
                .child(text(dir).font(p.mono()).color(p.ink()).w(90.0))
                .child(text(what).font(p.text()).color(p.muted())),
        );
    }
    p.card()
        .key("storage")
        .gap(16.0)
        .child(
            col()
                .gap(6.0)
                .child(text("Archive folder").font(p.k.ty.ui_medium).color(p.ink()))
                .child(
                    row()
                        .key("home")
                        .w_full()
                        .h(36.0)
                        .px(12.0)
                        .center_y()
                        .radius(8.0)
                        .fill(p.t.control_track)
                        .child(text(root).key("home-path").font(p.mono()).color(p.ink()).truncate()),
                )
                .child(
                    text("Set DISTILL_HOME before launching to keep the archive somewhere else. The folder is created with owner-only permissions.")
                        .font(p.caption())
                        .color(p.muted())
                        .wrap(),
                ),
        )
        .child(p.separator())
        .child(tree)
}

fn agent(d: &SetupDraft, p: &Paint) -> El {
    let h = d.config.steward.harness;
    let picked = HARNESSES.iter().position(|(x, _)| *x == h);
    let labels: Vec<&str> = HARNESSES.iter().map(|(_, l)| *l).collect();
    let harness = SegmentedControl::new(&labels, picked)
        .label("Steward harness")
        .width(Some(340.0))
        .on_pick(|s: &mut State, i, _: &mut Tx| s.setup.set_harness(HARNESSES[i].0))
        .build(&p.k);
    let off = h == Harness::None;
    let model = TextInput::new(&d.model)
        .label("Model")
        .placeholder(if h == Harness::Codex {
            "harness default"
        } else {
            "claude-haiku-5-5"
        })
        .width(Some(340.0))
        .disabled(off)
        .on_edit(|s: &mut State| &mut s.setup.model)
        .build(&p.k);
    let budget_labels: Vec<String> = BUDGETS
        .iter()
        .map(|b| {
            if b.fract() == 0.0 {
                format!("${b:.0}")
            } else {
                format!("${b:.2}")
            }
        })
        .collect();
    let budget_at = BUDGETS
        .iter()
        .position(|b| (b - d.config.steward.daily_budget_usd).abs() < 1e-9);
    let budget = SegmentedControl::new(&budget_labels, budget_at)
        .label("Daily budget")
        .width(Some(340.0))
        .on_pick(|s: &mut State, i, _: &mut Tx| {
            s.setup.config.steward.daily_budget_usd = BUDGETS[i]
        })
        .build(&p.k);
    let mut card = p
        .card()
        .key("agent")
        .gap(0.0)
        .py(4.0)
        .child(setting(
            p,
            "harness",
            "Harness",
            "The steward runs this agent headless to read new sessions and write a card for each: title, summary, outcome, tags.",
            harness,
        ))
        .child(p.separator())
        .child(setting(p, "model", "Model", "Leave empty for the harness's default.", model))
        .child(p.separator())
        .child(setting(
            p,
            "budget",
            "Daily budget",
            "Spending stops for the day at this amount. Only Claude Code reports cost; other harnesses are capped by sessions per run.",
            budget,
        ));
    if off {
        card = card.child(p.separator()).child(
            row()
                .key("off-note")
                .py(12.0)
                .gap(8.0)
                .center_y()
                .child(dot(p.k.c.warning.icon, 6.0))
                .child(
                    text("Sessions are archived and searchable, but no cards are written.")
                        .font(p.caption())
                        .color(p.muted()),
                ),
        );
    }
    card
}

fn privacy(d: &SetupDraft, p: &Paint) -> El {
    let field = TextInput::new(&d.exclude)
        .label("Path to never archive")
        .placeholder("/Users/me/work/secret-client")
        .width(None)
        .on_edit(|s: &mut State| &mut s.setup.exclude)
        .on_submit(|s: &mut State, _: &mut Tx| s.setup.add_exclude())
        .build(&p.k)
        .grow(1.0)
        .shrink();
    let add = Button::new("Add")
        .disabled(d.exclude.text.trim().is_empty())
        .on_press(|s: &mut State| s.setup.add_exclude())
        .build(&p.k)
        .key("add-exclude");
    let mut list = col()
        .key("excluded")
        .role("list")
        .label("Excluded paths")
        .gap(0.0);
    if d.config.privacy.exclude_paths.is_empty() {
        list = list.child(p.empty("none", "Nothing excluded yet."));
    }
    for (i, path) in d.config.privacy.exclude_paths.iter().enumerate() {
        if i > 0 {
            list = list.child(p.separator());
        }
        list = list.child(
            row()
                .key(format!("excluded-{i}"))
                .h(40.0)
                .gap(12.0)
                .center_y()
                .child(
                    text(path.clone())
                        .font(p.mono())
                        .color(p.ink())
                        .grow(1.0)
                        .shrink()
                        .truncate(),
                )
                .child(
                    Button::new("Remove")
                        .variant(ButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut State| {
                            if i < s.setup.config.privacy.exclude_paths.len() {
                                s.setup.config.privacy.exclude_paths.remove(i);
                            }
                        })
                        .build(&p.k)
                        .key("remove"),
                ),
        );
    }
    let redact = Switch::new(d.config.privacy.redact_exports)
        .label("Redact exports")
        .hide_label()
        .control_size(ControlSize::Small)
        .on_toggle(|s: &mut State| {
            s.setup.config.privacy.redact_exports = !s.setup.config.privacy.redact_exports
        })
        .build(&p.k)
        .key("redact");
    col()
        .gap(16.0)
        .child(
            p.section("exclude", "Never archive", None)
                .child(
                    text("Sessions whose working directory is inside one of these folders are skipped before anything is copied.")
                        .font(p.caption())
                        .color(p.muted())
                        .wrap(),
                )
                .child(row().w_full().gap(8.0).center_y().child(field).child(add))
                .child(list),
        )
        .child(p.card().key("redaction").gap(0.0).py(4.0).child(setting(
            p,
            "redact-row",
            "Redact exports",
            "Scan dataset builds for secrets (keys, tokens, emails) and replace them. Raw captures are never edited.",
            redact,
        )))
}

fn schedule(s: &State, p: &Paint) -> El {
    let c = &s.setup.config.schedule;
    let sweep_labels: Vec<&str> = SWEEPS.iter().map(|(_, l)| *l).collect();
    let sweep = SegmentedControl::new(
        &sweep_labels,
        SWEEPS.iter().position(|(m, _)| *m == c.sweep_every_minutes),
    )
    .label("Sweep every")
    .width(Some(340.0))
    .on_pick(|s: &mut State, i, _: &mut Tx| {
        s.setup.config.schedule.sweep_every_minutes = SWEEPS[i].0
    })
    .build(&p.k);
    let enrich_labels: Vec<&str> = ENRICHES.iter().map(|(_, l)| *l).collect();
    let enrich = SegmentedControl::new(
        &enrich_labels,
        ENRICHES
            .iter()
            .position(|(h, _)| *h == c.enrich_every_hours),
    )
    .label("Enrich every")
    .width(Some(340.0))
    .on_pick(|s: &mut State, i, _: &mut Tx| {
        s.setup.config.schedule.enrich_every_hours = ENRICHES[i].0
    })
    .build(&p.k);
    let status = match s.schedule_installed {
        Some(true) => pill(p, "timer installed", p.k.c.success),
        Some(false) => pill(p, "not installed", p.k.c.info),
        None => text("").font(p.caption()),
    };
    col()
        .gap(16.0)
        .child(
            p.card()
                .key("intervals")
                .gap(0.0)
                .py(4.0)
                .child(setting(p, "sweep", "Sweep", "Copy new and changed session files into the archive.", sweep))
                .child(p.separator())
                .child(setting(p, "enrich", "Enrich", "Let the steward write cards for new sessions, within the budget.", enrich)),
        )
        .child(
            p.section("timer", "Run in the background", Some(status.key("timer-status")))
                .child(
                    text("A system timer (launchd on macOS, systemd on Linux) runs the sweep on this schedule while the window is closed. Finish installs it; to remove it later:")
                        .font(p.caption())
                        .color(p.muted())
                        .wrap(),
                )
                .child(
                    row()
                        .key("command")
                        .w_full()
                        .h(36.0)
                        .px(12.0)
                        .center_y()
                        .radius(8.0)
                        .fill(p.t.control_track)
                        .child(text("distill schedule uninstall").font(p.mono()).color(p.ink())),
                ),
        )
}
