//! Setup: six steps over a draft `Config`, written on Finish together with the timer.
//!
//! Harnesses (detected, untick to skip), the home folder, providers (`trust::add` and
//! `trust::set_trusted`, shown as `distill providers` lists them, with the local runtime's
//! models), the steward's models (chosen from the models stored on this machine), personal terms
//! and excluded paths, and the schedule. Finish calls `setup::finish`, which writes
//! `config.toml` and installs the timer. Nothing reaches disk before Finish.

use distill_core::config::ProviderKind;
use distill_core::harness::{self, Detection};
use distill_core::runtime::{RuntimeState, Stored};
use distill_core::trust::{self, OLLAMA, OLLAMA_CLOUD, Provider, Trust};
use distill_core::{Config, Harness, schedule};
use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, SegmentedControl, Select, SelectAppearance, Switch,
    TextInput,
};

use crate::State;
use crate::jobs::{self, Job};
use crate::nav::{self, Screen};
use crate::ui::{Paint, count, dot, human_bytes, pill, tilde};

/// (name, what the step decides).
pub const STEPS: [(&str, &str); 6] = [
    ("Harnesses", "Which harnesses' sessions to archive"),
    ("Home folder", "Where the archive lives"),
    ("Providers", "Which models may read your sessions"),
    ("Models", "The local models that organize the archive"),
    ("Privacy", "Names and places that always count as personal"),
    ("Schedule", "How often Distill runs"),
];

pub const SWEEPS: [(u32, &str); 4] = [
    (15, "15 min"),
    (30, "30 min"),
    (60, "1 hour"),
    (180, "3 hours"),
];

/// The three model settings, as (select key, name, what it does, what "not set" means).
const MODELS: [(&str, &str, &str, &str); 3] = [
    (
        "card-model",
        "Cards",
        "Writes each session's title, summary, tags and outcome.",
        "Not set",
    ),
    (
        "embedding-model",
        "Embeddings",
        "Turns what you wrote and each card into vectors for search by meaning.",
        "Not set",
    ),
    (
        "detail-model",
        "Detection pass",
        "Finds names, addresses, phone numbers and client names that no pattern can.",
        "Same as the card model",
    ),
];

/// Setup's working copy. Nothing reaches disk until Finish.
pub struct SetupState {
    pub step: usize,
    pub draft: Config,
    /// Per harness, what detection found; None until it answers.
    pub detections: Option<Vec<Detection>>,
    /// What the local runtime is doing, with its models; None until it answers.
    pub runtime: Option<RuntimeState>,
    pub timer_installed: Option<bool>,
    pub provider: TextEdit,
    pub term: TextEdit,
    pub exclude: TextEdit,
    pub provider_error: Option<String>,
    /// The model menu that is open, by key.
    pub open: Option<&'static str>,
    pub finishing: bool,
    pub error: Option<String>,
    /// No config yet: detection decides which harnesses start ticked.
    fresh: bool,
}

impl SetupState {
    pub fn new(config: Option<&Config>) -> SetupState {
        let mut draft = config.cloned().unwrap_or_default();
        // Every harness gets a row, even one an older config doesn't list.
        for kind in Harness::ALL {
            if draft.harness(kind).is_none() {
                draft.harness_mut(kind).enabled = false;
            }
        }
        SetupState {
            step: 0,
            draft,
            detections: None,
            runtime: None,
            timer_installed: None,
            provider: TextEdit::new(""),
            term: TextEdit::new(""),
            exclude: TextEdit::new(""),
            provider_error: None,
            open: None,
            finishing: false,
            error: None,
            fresh: config.is_none(),
        }
    }

    /// Detection answered. Before the first setup, a harness is ticked when it was found.
    pub fn detected(&mut self, found: Vec<Detection>) {
        if self.fresh {
            for d in &found {
                self.draft.harness_mut(d.kind).enabled = d.found;
            }
        }
        self.detections = Some(found);
    }

    fn last(&self) -> bool {
        self.step + 1 == STEPS.len()
    }

    fn toggle(&mut self, kind: Harness) {
        let h = self.draft.harness_mut(kind);
        h.enabled = !h.enabled;
    }

    /// The models the steward may use: those stored on this machine.
    fn local_models(&self) -> Vec<String> {
        match &self.runtime {
            Some(RuntimeState::Running(models)) => models
                .iter()
                .filter(|m| m.stored == Stored::Here)
                .map(|m| m.name.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    fn model(&mut self, key: &str) -> &mut Option<String> {
        let st = &mut self.draft.steward;
        match key {
            "card-model" => &mut st.card_model,
            "embedding-model" => &mut st.embedding_model,
            _ => &mut st.detail_model,
        }
    }

    fn add_provider(&mut self) {
        let name = self.provider.text.trim().to_string();
        if name.is_empty() {
            return;
        }
        match trust::add(&mut self.draft, &name, false) {
            Ok(()) => {
                self.provider.clear();
                self.provider_error = None;
            }
            Err(e) => self.provider_error = Some(jobs::line(e)),
        }
    }

    fn set_trusted(&mut self, name: &str, trusted: bool) {
        self.provider_error = trust::set_trusted(&mut self.draft, name, trusted)
            .err()
            .map(jobs::line);
    }

    fn add_term(&mut self) {
        let term = self.term.text.trim().to_string();
        let terms = &mut self.draft.privacy.personal_terms;
        if !term.is_empty() && !terms.contains(&term) {
            terms.push(term);
        }
        self.term.clear();
    }

    fn add_exclude(&mut self) {
        let path = self.exclude.text.trim().to_string();
        let paths = &mut self.draft.privacy.exclude_paths;
        if !path.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
        self.exclude.clear();
    }
}

/// Moving to Setup asks for what it shows: what each harness has on this machine, the local
/// runtime's models, and whether the timer is installed.
pub fn open(s: &mut State) {
    let roots: Vec<_> = Harness::ALL
        .into_iter()
        .map(|k| (k, s.setup.draft.roots_for(k)))
        .collect();
    s.ask(Job::new("detect harnesses", move |_| {
        let found: Vec<Detection> = roots
            .into_iter()
            .map(|(kind, roots)| harness::detect(kind, &roots))
            .collect();
        Box::new(move |s: &mut State| s.setup.detected(found))
    }));
    s.ask(Job::new("local runtime", |core| {
        let state = core.runtime.state();
        Box::new(move |s: &mut State| s.setup.runtime = Some(state))
    }));
    s.ask(Job::new("timer", |core| {
        let on = (core.timer.installed)();
        Box::new(move |s: &mut State| s.setup.timer_installed = Some(on))
    }));
}

/// Finish: write the draft and install the timer, then show the overview.
fn finish(s: &mut State) {
    if s.setup.finishing {
        return;
    }
    s.setup.finishing = true;
    s.setup.error = None;
    let config = s.setup.draft.clone();
    s.ask(Job::new("finish setup", move |core| {
        let r = distill_core::setup::finish(&core.home, &config, |h, c| (core.timer.install)(h, c))
            .map_err(jobs::line);
        Box::new(move |s: &mut State| {
            s.setup.finishing = false;
            match r {
                Ok(_) => {
                    s.config = Some(config);
                    s.config_error = None;
                    s.setup.fresh = false;
                    s.setup.timer_installed = Some(true);
                    s.overview.notice = Some("Setup saved and the timer installed".into());
                    nav::go(s, Screen::Overview);
                }
                Err(e) => s.setup.error = Some(e),
            }
        })
    }));
}

// ---------------------------------------------------------------------------------------------
// View

pub fn view(s: &State, p: &Paint, layer: &mut Vec<El>) -> El {
    let d = &s.setup;
    let (name, about) = STEPS[d.step];
    let body = match d.step {
        0 => harnesses(d, p),
        1 => home(s, p),
        2 => providers(d, p),
        3 => models(d, p, layer),
        4 => privacy(d, p),
        _ => timer(s, p),
    };
    let mut scroll = col()
        .key("scroll")
        .w_full()
        .px(36.0)
        .pt(28.0)
        .pb(24.0)
        .gap(18.0)
        .child(stepper(d, p))
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
        );
    if let Some(e) = &s.config_error {
        scroll = scroll.child(
            p.note(format!(
                "The config could not be read, so setup starts over: {e}"
            ))
            .key("config-error"),
        );
    }
    let scroll = scroll
        .child(body.key(format!("body-{}", d.step)))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .grow(1.0)
        .min_h(0.0);
    col().w_full().h_full().child(scroll).child(footer(d, p))
}

/// The steps as a row of chips, each a button to jump to it.
fn stepper(d: &SetupState, p: &Paint) -> El {
    let mut r = row()
        .key("steps")
        .role("list")
        .label("Setup steps")
        .gap(6.0)
        .wrap();
    for (i, (name, _)) in STEPS.iter().enumerate() {
        let current = i == d.step;
        r = r.child(
            p.k.interactive(
                row()
                    .key(format!("step-{i}"))
                    .role("button")
                    .label(*name)
                    .checked(current)
                    .h(28.0)
                    .px(10.0)
                    .gap(6.0)
                    .center_y()
                    .radius(14.0)
                    .fill(if current {
                        p.t.hover
                    } else {
                        rune::Color::TRANSPARENT
                    })
                    .border(p.hair, p.t.separator)
                    .focusable(true)
                    .on_press(move |s: &mut State| {
                        s.setup.step = i;
                        s.setup.open = None;
                    })
                    .child(
                        text(format!("{} {}", i + 1, name))
                            .font(if current { p.k.ty.ui_medium } else { p.text() })
                            .color(if current { p.ink() } else { p.muted() })
                            .a11y_hidden(),
                    ),
            ),
        );
    }
    r
}

fn footer(d: &SetupState, p: &Paint) -> El {
    let mut left = row().gap(10.0).center_y().grow(1.0).shrink();
    if let Some(e) = &d.error {
        left = left.child(
            text(format!("Could not finish: {e}"))
                .key("error")
                .font(p.caption())
                .color(p.bad())
                .wrap()
                .shrink(),
        );
    }
    let back = Button::new("Back")
        .disabled(d.step == 0)
        .on_press(|s: &mut State| {
            s.setup.step = s.setup.step.saturating_sub(1);
            s.setup.open = None;
        })
        .build(&p.k)
        .key("back");
    let next = if d.last() {
        Button::new(if d.finishing {
            "Finishing…"
        } else {
            "Finish"
        })
        .variant(ButtonVariant::Submit)
        .disabled(d.finishing)
        .on_press(finish)
        .build(&p.k)
        .key("finish")
    } else {
        Button::new("Next")
            .variant(ButtonVariant::Submit)
            .on_press(|s: &mut State| {
                s.setup.step = (s.setup.step + 1).min(STEPS.len() - 1);
                s.setup.open = None;
            })
            .build(&p.k)
            .key("next")
    };
    let bar = row()
        .key("footer")
        .w_full()
        .h(60.0)
        .px(36.0)
        .gap(16.0)
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
                .child(p.note(help)),
        )
        .child(control)
}

fn harnesses(d: &SetupState, p: &Paint) -> El {
    let mut card = p.card().key("harnesses").gap(0.0).py(4.0);
    for (i, kind) in Harness::ALL.into_iter().enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        let found = d
            .detections
            .as_ref()
            .and_then(|v| v.iter().find(|x| x.kind == kind));
        card = card.child(harness_row(
            p,
            kind,
            d.draft.enabled(kind),
            found,
            d.detections.is_some(),
            &d.draft.roots_for(kind),
        ));
    }
    col().gap(12.0).child(card).child(p.note(
        "Each harness's files are copied into the raw archive exactly as they are and never \
         edited. Untick a harness to skip it.",
    ))
}

fn harness_row(
    p: &Paint,
    kind: Harness,
    on: bool,
    found: Option<&Detection>,
    scanned: bool,
    roots: &[std::path::PathBuf],
) -> El {
    let where_ = roots
        .iter()
        .map(|r| tilde(r))
        .collect::<Vec<_>>()
        .join(", ");
    let status = match found {
        Some(f) if f.found => text(format!(
            "{} · {}",
            human_bytes(f.bytes),
            count(f.files, "file")
        ))
        .font(p.num())
        .color(p.ink()),
        Some(_) => text("not found").font(p.text()).color(p.faint()),
        None if scanned => text("not found").font(p.text()).color(p.faint()),
        None => text("looking…").font(p.text()).color(p.faint()),
    };
    let mut right = row().gap(10.0).center_y().child(status.key("found"));
    if !kind.has_parser() {
        right = right.child(pill(p, "archived, no parser", p.k.c.warning).key("no-parser"));
    }
    let present = found.is_some_and(|f| f.found);
    row()
        .key(format!("harness-{}", kind.id()))
        .w_full()
        .h(52.0)
        .px(4.0)
        .gap(14.0)
        .center_y()
        .child(
            Switch::new(on)
                .label(kind.label())
                .hide_label()
                .control_size(ControlSize::Small)
                .on_toggle(move |s: &mut State| s.setup.toggle(kind))
                .build(&p.k)
                .key("switch"),
        )
        .child(dot(if present { p.good() } else { p.off() }, 8.0))
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

fn home(s: &State, p: &Paint) -> El {
    let layout = [
        ("config.toml", "what this setup writes"),
        (
            "raw/",
            "every harness file exactly as captured, never edited",
        ),
        (
            "sessions/",
            "each session's facts, events, findings and card",
        ),
        ("index/", "the search index"),
        ("log/", "the read log: every read of session text"),
        ("reveals/", "reveal requests waiting for you"),
        ("keys/", "the key that fingerprints findings"),
    ];
    let mut tree = col().key("layout").gap(6.0);
    for (dir, what) in layout {
        tree = tree.child(
            row()
                .gap(12.0)
                .child(text(dir).font(p.mono()).color(p.ink()).w(110.0))
                .child(text(what).font(p.text()).color(p.muted())),
        );
    }
    p.card()
        .key("home")
        .gap(16.0)
        .child(
            col()
                .gap(6.0)
                .child(text("Home folder").font(p.k.ty.ui_medium).color(p.ink()))
                .child(
                    row()
                        .key("path")
                        .w_full()
                        .h(36.0)
                        .px(12.0)
                        .center_y()
                        .radius(8.0)
                        .fill(p.t.control_track)
                        .child(
                            text(tilde(&s.home.root))
                                .key("home-path")
                                .font(p.mono())
                                .color(p.ink())
                                .truncate(),
                        ),
                )
                .child(p.note(
                    "Everything in it is readable by you alone. To keep the archive somewhere \
                     else, start Distill with DISTILL_HOME set to that folder; the timer uses \
                     the same folder.",
                )),
        )
        .child(p.separator())
        .child(tree)
}

/// What the providers step shows for each provider, as `distill providers` lists it.
fn provider_label(pr: &Provider) -> String {
    format!(
        "{} {} {}",
        pr.name,
        match pr.kind {
            ProviderKind::Local => "local",
            ProviderKind::Remote => "remote",
        },
        if pr.trusted { "trusted" } else { "not trusted" }
    )
}

/// The local runtime's state under `ollama`, worded as `distill providers` words it.
fn runtime_lines(state: Option<&RuntimeState>) -> Vec<(String, String)> {
    match state {
        None => vec![("Asking Ollama…".into(), String::new())],
        Some(RuntimeState::NotInstalled) => {
            vec![(
                "Ollama is not installed on this machine".into(),
                String::new(),
            )]
        }
        Some(RuntimeState::NotRunning) => vec![(
            "Ollama is installed but not running (start the Ollama app or `ollama serve`)".into(),
            String::new(),
        )],
        Some(RuntimeState::Running(models)) if models.is_empty() => {
            vec![("running, with no models installed".into(), String::new())]
        }
        Some(RuntimeState::Running(models)) => models
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    match m.stored {
                        Stored::Here => "stored on this machine".to_string(),
                        Stored::Cloud => format!("served from a cloud (reads as {OLLAMA_CLOUD})"),
                    },
                )
            })
            .collect(),
    }
}

fn providers(d: &SetupState, p: &Paint) -> El {
    let trust = Trust::new(&d.draft);
    let mut card = p.card().key("providers").gap(0.0).py(4.0);
    for (i, pr) in trust.providers().iter().enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        let control = match pr.kind {
            ProviderKind::Local => pill(p, "Trusted", p.k.c.success).key("trust"),
            ProviderKind::Remote => {
                let name = pr.name.clone();
                let trusted = pr.trusted;
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text(if trusted { "Trusted" } else { "Not trusted" })
                            .font(p.text())
                            .color(if trusted { p.ink() } else { p.muted() }),
                    )
                    .child(
                        Switch::new(trusted)
                            .label(format!("Trust {}", pr.name))
                            .hide_label()
                            .control_size(ControlSize::Small)
                            .on_toggle(move |s: &mut State| s.setup.set_trusted(&name, !trusted))
                            .build(&p.k)
                            .key("switch"),
                    )
            }
        };
        let mut about = col().gap(2.0).grow(1.0).shrink().child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(pr.name.clone()).font(p.k.ty.ui_medium).color(p.ink()))
                .child(
                    text(match pr.kind {
                        ProviderKind::Local => "local, on this machine",
                        ProviderKind::Remote => "remote",
                    })
                    .font(p.caption())
                    .color(p.muted()),
                ),
        );
        if pr.name == OLLAMA {
            for (name, where_) in runtime_lines(d.runtime.as_ref()) {
                about = about.child(
                    row()
                        .gap(10.0)
                        .child(text(name).font(p.mono()).color(p.muted()))
                        .child(text(where_).font(p.caption()).color(p.faint())),
                );
            }
        }
        card = card.child(
            row()
                .key(format!("provider-{}", pr.name))
                .label(provider_label(pr))
                .w_full()
                .py(10.0)
                .px(4.0)
                .gap(16.0)
                .center_y()
                .child(about)
                .child(control),
        );
    }
    let field = TextInput::new(&d.provider)
        .label("Remote provider")
        .placeholder("a provider's name, e.g. anthropic")
        .width(None)
        .on_edit(|s: &mut State| &mut s.setup.provider)
        .on_submit(|s: &mut State, _: &mut Tx| s.setup.add_provider())
        .build(&p.k)
        .key("new-provider")
        .grow(1.0)
        .shrink();
    let add = Button::new("Add")
        .disabled(d.provider.text.trim().is_empty())
        .on_press(|s: &mut State| s.setup.add_provider())
        .build(&p.k)
        .key("add-provider");
    let mut c = col()
        .gap(12.0)
        .child(card)
        .child(row().w_full().gap(8.0).center_y().child(field).child(add));
    if let Some(e) = &d.provider_error {
        c = c.child(
            text(e.clone())
                .key("trust-error")
                .font(p.caption())
                .color(p.bad())
                .wrap(),
        );
    }
    for w in trust.warnings() {
        c = c.child(p.note(format!("Ignored: {w}")));
    }
    c.child(p.note(
        "Models on this machine are trusted. A model at a remote provider may read your \
         sessions, with every secret replaced by a placeholder, only once you mark the provider \
         trusted. Anything else is refused.",
    ))
}

fn models(d: &SetupState, p: &Paint, layer: &mut Vec<El>) -> El {
    let installed = d.local_models();
    let mut card = p.card().key("models").gap(0.0).py(4.0);
    for (i, (key, name, help, unset)) in MODELS.into_iter().enumerate() {
        if i > 0 {
            card = card.child(p.separator());
        }
        let current = match key {
            "card-model" => d.draft.steward.card_model.clone(),
            "embedding-model" => d.draft.steward.embedding_model.clone(),
            _ => d.draft.steward.detail_model.clone(),
        };
        let mut options = vec![unset.to_string()];
        options.extend(installed.iter().cloned());
        // A model set before but not offered now (the runtime stopped, or the model removed).
        if let Some(m) = &current
            && !options.contains(m)
        {
            options.push(m.clone());
        }
        let shown = current.clone().unwrap_or_else(|| unset.to_string());
        let select = Select::new(key, &options)
            .label(name)
            .appearance(SelectAppearance::Field)
            .width(240.0)
            .current(Some(&shown))
            .open(d.open == Some(key))
            .on_toggle(move |s: &mut State| {
                s.setup.open = if s.setup.open == Some(key) {
                    None
                } else {
                    Some(key)
                }
            })
            .on_close(|s: &mut State| s.setup.open = None)
            .on_choose(move |s: &mut State, v: &str, _: &mut Tx| {
                *s.setup.model(key) = (v != unset).then(|| v.to_string());
                s.setup.open = None;
            })
            .build(&p.k, layer);
        card = card.child(setting(p, key, name, help, select));
    }
    let note = match &d.runtime {
        Some(RuntimeState::Running(_)) if installed.is_empty() => {
            "Ollama has no models stored on this machine: pull one (`ollama pull <model>`), then \
             open Setup again."
                .to_string()
        }
        Some(RuntimeState::Running(_)) => format!(
            "Stored on this machine: {}. Models Ollama serves from a cloud are not offered.",
            installed.join(", ")
        ),
        Some(RuntimeState::NotRunning) => {
            "Ollama is installed but not running: start it, then open Setup again.".to_string()
        }
        Some(RuntimeState::NotInstalled) => {
            "Ollama is not installed. Capture, findings and word search work without a model."
                .to_string()
        }
        None => "Asking Ollama for its models…".to_string(),
    };
    col()
        .gap(12.0)
        .child(card)
        .child(p.note(note).key("models-note"))
        .child(p.note(
            "A model that is not set means its step is skipped. Each runs on this machine, newest \
         sessions first, a few per run.",
        ))
}

fn list_of(
    p: &Paint,
    key: &str,
    items: &[String],
    empty: &str,
    remove: fn(&mut State, usize),
) -> El {
    let mut list = col().key(key).role("list").gap(0.0);
    if items.is_empty() {
        list = list.child(p.empty("none", empty));
    }
    let prefix = key.trim_end_matches('s');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            list = list.child(p.separator());
        }
        list = list.child(
            row()
                .key(format!("{prefix}-{i}"))
                .h(38.0)
                .gap(12.0)
                .center_y()
                .child(
                    text(item.clone())
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
                        .on_press(move |s: &mut State| remove(s, i))
                        .build(&p.k)
                        .key("remove"),
                ),
        );
    }
    list
}

fn privacy(d: &SetupState, p: &Paint) -> El {
    let term = TextInput::new(&d.term)
        .label("Personal term")
        .placeholder("a name, a client, a place")
        .width(None)
        .on_edit(|s: &mut State| &mut s.setup.term)
        .on_submit(|s: &mut State, _: &mut Tx| s.setup.add_term())
        .build(&p.k)
        .grow(1.0)
        .shrink();
    let add_term = Button::new("Add")
        .disabled(d.term.text.trim().is_empty())
        .on_press(|s: &mut State| s.setup.add_term())
        .build(&p.k)
        .key("add-term");
    let exclude = TextInput::new(&d.exclude)
        .label("Folder to never archive")
        .placeholder("/path/to/a/project")
        .width(None)
        .on_edit(|s: &mut State| &mut s.setup.exclude)
        .on_submit(|s: &mut State, _: &mut Tx| s.setup.add_exclude())
        .build(&p.k)
        .grow(1.0)
        .shrink();
    let add_exclude = Button::new("Add")
        .disabled(d.exclude.text.trim().is_empty())
        .on_press(|s: &mut State| s.setup.add_exclude())
        .build(&p.k)
        .key("add-exclude");
    col()
        .gap(16.0)
        .child(
            p.section("personal-terms", "Personal terms", None)
                .child(p.note(
                    "Words and names that always count as personal details, matched as whole \
                     words whatever their case.",
                ))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(term)
                        .child(add_term),
                )
                .child(list_of(
                    p,
                    "terms",
                    &d.draft.privacy.personal_terms,
                    "No personal terms yet.",
                    |s, i| {
                        let t = &mut s.setup.draft.privacy.personal_terms;
                        if i < t.len() {
                            t.remove(i);
                        }
                    },
                )),
        )
        .child(
            p.section("excluded", "Never archive", None)
                .child(p.note(
                    "Sessions that worked inside one of these folders are skipped before \
                     anything is copied.",
                ))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(exclude)
                        .child(add_exclude),
                )
                .child(list_of(
                    p,
                    "excludes",
                    &d.draft.privacy.exclude_paths,
                    "Nothing excluded.",
                    |s, i| {
                        let t = &mut s.setup.draft.privacy.exclude_paths;
                        if i < t.len() {
                            t.remove(i);
                        }
                    },
                )),
        )
}

fn timer(s: &State, p: &Paint) -> El {
    let c = &s.setup.draft.schedule;
    let labels: Vec<&str> = SWEEPS.iter().map(|(_, l)| *l).collect();
    let sweep = SegmentedControl::new(
        &labels,
        SWEEPS.iter().position(|(m, _)| *m == c.sweep_every_minutes),
    )
    .label("Sweep every")
    .width(Some(340.0))
    .on_pick(|s: &mut State, i, _: &mut Tx| {
        s.setup.draft.schedule.sweep_every_minutes = SWEEPS[i].0
    })
    .build(&p.k);
    let installed = match s.setup.timer_installed {
        Some(true) => "The timer is installed. Finish installs it again with these settings.",
        Some(false) => "The timer is not installed yet. Finish installs it.",
        None => "Checking the timer…",
    };
    col()
        .gap(12.0)
        .child(p.card().key("schedule").gap(0.0).py(4.0).child(setting(
            p,
            "sweep",
            "Sweep every",
            "How often the timer runs a tick: sweep the harnesses, then parse, scan, index \
                 and organize what is new.",
            sweep,
        )))
        .child(p.note(installed).key("timer"))
        .child(p.note(format!(
            "Finish writes {} and installs the timer ({}), which runs `distill tick` every {}.",
            tilde(&s.home.config_path()),
            schedule::LABEL,
            SWEEPS
                .iter()
                .find(|(m, _)| *m == c.sweep_every_minutes)
                .map_or(format!("{} minutes", c.sweep_every_minutes), |(_, l)| {
                    l.replace("min", "minutes")
                })
        )))
}
