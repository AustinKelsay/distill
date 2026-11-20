//! Setup: six steps over a draft `Config`, written on Finish together with the timer.
//!
//! Harnesses (detected, untick to skip), the home folder, trusted AI (the providers, via
//! `trust::add` and `trust::set_trusted`, shown as `distill providers` lists them, with the local
//! runtime's models and common services ready to trust), the steward's models (run on this
//! machine's Ollama, or at a trusted service through `[steward.generation]`, and chosen from the
//! models it lists; embeddings always run in Ollama), privacy (what is found automatically, personal terms and
//! excluded paths), and the schedule. Finish calls `setup::finish`, which writes
//! `config.toml` and installs the timer. Nothing reaches disk before Finish.

use distill_core::config::{GenerationConfig, ProviderKind};
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
    ("Trusted AI", "Who may read your sessions"),
    ("Models", "The models that organize the archive"),
    ("Privacy", "What DISTILL finds by itself, and what you add"),
    ("Schedule", "How often DISTILL runs"),
];

/// Common AI services, as (the provider name an agent passes with `--provider`, how it is shown).
/// Each is offered ready to trust; trusting one adds it to the config's `[[providers]]`.
pub const SERVICES: [(&str, &str); 6] = [
    ("anthropic", "Anthropic"),
    ("openai", "OpenAI"),
    ("google", "Google"),
    ("xai", "xAI"),
    ("openrouter", "OpenRouter"),
    (OLLAMA_CLOUD, "Ollama cloud"),
];

pub const SWEEPS: [(u32, &str); 4] = [
    (15, "15 min"),
    (30, "30 min"),
    (60, "1 hour"),
    (180, "3 hours"),
];

/// The "run on" choice for running models on this machine.
const HERE: &str = "This machine (Ollama)";

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
    /// The service's address and key file, while models run at a trusted service.
    pub base_url: TextEdit,
    pub key_file: TextEdit,
    /// What that service listed when last asked: its models, or why it couldn't.
    pub remote_models: Option<Result<Vec<String>, String>>,
    pub checking: bool,
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
        let generation = draft.steward.generation.clone();
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
            base_url: TextEdit::new(generation.as_ref().map_or("", |g| g.base_url.as_str())),
            key_file: TextEdit::new(
                generation
                    .as_ref()
                    .map_or(String::new(), |g| g.api_key_file.display().to_string()),
            ),
            remote_models: None,
            checking: false,
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

    /// Where the card and detail models run: a trusted service's name, or None for this machine.
    pub fn runs_at(&self) -> Option<&str> {
        self.draft
            .steward
            .generation
            .as_ref()
            .map(|g| g.provider.as_str())
    }

    /// Run the card and detail models at `provider`, or on this machine. Their models are
    /// cleared, since another runtime serves other models.
    fn run_at(&mut self, provider: Option<&str>) {
        if provider == self.runs_at() {
            return;
        }
        let st = &mut self.draft.steward;
        st.card_model = None;
        st.detail_model = None;
        st.generation = provider.map(|p| GenerationConfig {
            provider: p.to_string(),
            base_url: String::new(),
            api_key_file: Default::default(),
        });
        self.remote_models = None;
        self.sync_generation();
    }

    /// Copy the address and key file fields into the draft's `[steward.generation]`.
    fn sync_generation(&mut self) {
        if let Some(g) = &mut self.draft.steward.generation {
            g.base_url = self.base_url.text.trim().to_string();
            g.api_key_file = self.key_file.text.trim().into();
        }
    }

    /// Why the draft can't be finished as it is, if it can't.
    fn unfinished(&self) -> Option<String> {
        let g = self.draft.steward.generation.as_ref()?;
        if g.base_url.is_empty() || g.api_key_file.as_os_str().is_empty() {
            return Some(format!(
                "Models: give {}'s address and key file, or run models on this machine",
                g.provider
            ));
        }
        None
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

    /// Trust a common service the config doesn't list yet.
    fn trust_service(&mut self, name: &str) {
        self.provider_error = trust::add(&mut self.draft, name, true)
            .err()
            .map(jobs::line);
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
    check_service(s);
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

/// Ask the service the models run at for its models, when they run at one.
fn check_service(s: &mut State) {
    s.setup.sync_generation();
    let Some(g) = s.setup.draft.steward.generation.clone() else {
        return;
    };
    if g.base_url.is_empty() || g.api_key_file.as_os_str().is_empty() {
        return;
    }
    s.setup.checking = true;
    let config = s.setup.draft.clone();
    s.ask(Job::new("service models", move |_| {
        let listed = distill_core::tick::generation_models(&config, &g).map_err(jobs::line);
        Box::new(move |s: &mut State| {
            s.setup.checking = false;
            s.setup.remote_models = Some(listed);
        })
    }));
}

/// Finish: write the draft and install the timer, then show the overview.
fn finish(s: &mut State) {
    if s.setup.finishing {
        return;
    }
    s.setup.sync_generation();
    if let Some(e) = s.setup.unfinished() {
        s.setup.error = Some(e);
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
                     else, start DISTILL with DISTILL_HOME set to that folder; the timer uses \
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

/// How a service is shown: its common name when it has one.
fn service_name(name: &str) -> &str {
    SERVICES
        .iter()
        .find(|(id, _)| *id == name)
        .map_or(name, |(_, shown)| shown)
}

/// A service's trust switch, with its state in words.
fn trust_switch(p: &Paint, name: &str, trusted: bool, toggle: impl Fn(&mut State) + 'static) -> El {
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
                .label(format!("Trust {name}"))
                .hide_label()
                .control_size(ControlSize::Small)
                .on_toggle(toggle)
                .build(&p.k)
                .key("switch"),
        )
}

/// A service's row: its shown name, the provider name agents pass, and `control`.
fn service_row(p: &Paint, key: String, label: String, name: &str, control: El) -> El {
    let shown = service_name(name);
    let mut title = row().gap(8.0).center_y().child(
        text(shown.to_string())
            .font(p.k.ty.ui_medium)
            .color(p.ink()),
    );
    if shown != name {
        title = title.child(text(name.to_string()).font(p.mono()).color(p.faint()));
    }
    row()
        .key(key)
        .label(label)
        .w_full()
        .py(10.0)
        .px(4.0)
        .gap(16.0)
        .center_y()
        .child(col().grow(1.0).shrink().child(title))
        .child(control)
}

fn providers(d: &SetupState, p: &Paint) -> El {
    let trust = Trust::new(&d.draft);
    let mut here = p.section("this-machine", "This machine", None);
    let mut services = p.section("services", "AI services", None).gap(0.0);
    let mut listed = 0;
    for pr in trust.providers() {
        match pr.kind {
            ProviderKind::Local => {
                let mut about = col().gap(2.0).grow(1.0).shrink().child(
                    text(if pr.name == OLLAMA {
                        "Ollama"
                    } else {
                        &pr.name
                    })
                    .font(p.k.ty.ui_medium)
                    .color(p.ink()),
                );
                for (name, where_) in runtime_lines(d.runtime.as_ref()) {
                    about = about.child(
                        row()
                            .gap(10.0)
                            .child(text(name).font(p.mono()).color(p.muted()))
                            .child(text(where_).font(p.caption()).color(p.faint())),
                    );
                }
                here = here.child(
                    row()
                        .key(format!("provider-{}", pr.name))
                        .label(provider_label(pr))
                        .w_full()
                        .px(4.0)
                        .gap(16.0)
                        .center_y()
                        .child(about)
                        .child(pill(p, "Always trusted", p.k.c.success).key("trust")),
                );
            }
            ProviderKind::Remote => {
                let name = pr.name.clone();
                let trusted = pr.trusted;
                let control = trust_switch(p, service_name(&pr.name), trusted, move |s| {
                    s.setup.set_trusted(&name, !trusted)
                });
                services = services.child(if listed == 0 {
                    el_spacer()
                } else {
                    p.separator()
                });
                listed += 1;
                services = services.child(service_row(
                    p,
                    format!("provider-{}", pr.name),
                    provider_label(pr),
                    &pr.name,
                    control,
                ));
            }
        }
    }
    // Common services the config doesn't list yet, ready to trust.
    for (id, shown) in SERVICES {
        if trust.provider(id).is_some() {
            continue;
        }
        let control = trust_switch(p, shown, false, move |s| s.setup.trust_service(id));
        services = services.child(p.separator()).child(service_row(
            p,
            format!("service-{id}"),
            format!("{id} remote not trusted"),
            id,
            control,
        ));
    }
    let field = TextInput::new(&d.provider)
        .label("Another service")
        .placeholder("another service, by the name its agents pass with --provider")
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
    services = services.child(
        row()
            .w_full()
            .pt(12.0)
            .gap(8.0)
            .center_y()
            .child(field)
            .child(add),
    );
    let mut c = col()
        .gap(16.0)
        .child(p.note(
            "Trusting a service lets two things read your sessions: your agents that run at it, \
             when they search or read through DISTILL, and DISTILL's own models when they run \
             there (the next step). Each gets every secret replaced by a placeholder, and \
             personal details as written. Anything not trusted is refused.",
        ))
        .child(here)
        .child(services);
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
        "An agent names its service itself (`--provider`), and DISTILL cannot check it. Name a \
         service for an agent only if every service that agent can use is trusted.",
    ))
}

/// Space before the first row of a list, where later rows have a separator.
fn el_spacer() -> El {
    rune::el().h(4.0)
}

/// Where the card and detail models run: this machine, or a trusted service with its address,
/// key file and what it listed.
fn runner(d: &SetupState, p: &Paint, layer: &mut Vec<El>) -> El {
    let trust = Trust::new(&d.draft);
    let mut options = vec![HERE.to_string()];
    options.extend(
        trust
            .providers()
            .iter()
            .filter(|pr| pr.kind == ProviderKind::Remote && pr.trusted)
            .map(|pr| pr.name.clone()),
    );
    // A service set before but no longer trusted: shown, and refused at the next tick.
    if let Some(at) = d.runs_at()
        && !options.iter().any(|o| o == at)
    {
        options.push(at.to_string());
    }
    let shown = d.runs_at().unwrap_or(HERE).to_string();
    let select = Select::new("runs-at", &options)
        .label("Run models on")
        .appearance(SelectAppearance::Field)
        .width(240.0)
        .current(Some(&shown))
        .open(d.open == Some("runs-at"))
        .on_toggle(|s: &mut State| {
            s.setup.open = if s.setup.open == Some("runs-at") {
                None
            } else {
                Some("runs-at")
            }
        })
        .on_close(|s: &mut State| s.setup.open = None)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
            s.setup.run_at((v != HERE).then_some(v));
            s.setup.open = None;
        })
        .build(&p.k, layer);
    let mut card = p.card().key("runner").gap(0.0).py(4.0).child(setting(
        p,
        "runs-at",
        "Run models on",
        "Where the card and detection models run. A service must be trusted (the Trusted AI \
         step) and OpenAI-compatible.",
        select,
    ));
    let Some(at) = d.runs_at() else {
        return card;
    };
    let field =
        |edit: &TextEdit, label: &str, hint: &str, pick: fn(&mut State) -> &mut TextEdit| {
            TextInput::new(edit)
                .label(label)
                .placeholder(hint)
                .width(Some(360.0))
                .on_edit(pick)
                .build(&p.k)
        };
    card = card
        .child(p.separator())
        .child(setting(
            p,
            "base-url",
            "Address",
            "The service's HTTPS API address, with its version path.",
            field(&d.base_url, "Address", "https://…/v1", |s| {
                &mut s.setup.base_url
            }),
        ))
        .child(p.separator())
        .child(setting(
            p,
            "key-file",
            "API key file",
            "A file holding the key on its first line, readable by you alone.",
            field(&d.key_file, "API key file", "/path/to/key", |s| {
                &mut s.setup.key_file
            }),
        ));
    let status = match (&d.remote_models, d.checking) {
        (_, true) => text(format!("Asking {at}…")).color(p.muted()),
        (Some(Ok(m)), _) if m.is_empty() => text(format!("{at} lists no models")).color(p.warn()),
        (Some(Ok(m)), _) => {
            text(format!("{at} serves {}", count(m.len() as u64, "model"))).color(p.good())
        }
        (Some(Err(e)), _) => text(e.clone()).color(p.bad()),
        (None, _) => text("Check lists the models it serves.").color(p.muted()),
    };
    let check = Button::new("Check")
        .disabled(
            d.checking || d.base_url.text.trim().is_empty() || d.key_file.text.trim().is_empty(),
        )
        .on_press(check_service)
        .build(&p.k)
        .key("check");
    card.child(
        row()
            .key("service-status")
            .w_full()
            .py(12.0)
            .gap(12.0)
            .center_y()
            .child(check)
            .child(status.font(p.caption()).wrap().shrink()),
    )
}

fn models(d: &SetupState, p: &Paint, layer: &mut Vec<El>) -> El {
    let installed = d.local_models();
    let served = match (d.runs_at(), &d.remote_models) {
        (None, _) => installed.clone(),
        (Some(_), Some(Ok(m))) => m.clone(),
        (Some(_), _) => Vec::new(),
    };
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
        // Embeddings always run in Ollama on this machine.
        options.extend(
            (if key == "embedding-model" {
                &installed
            } else {
                &served
            })
            .iter()
            .cloned(),
        );
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
        .child(runner(d, p, layer))
        .child(card)
        .child(p.note(note).key("models-note"))
        .child(p.note(
            "A model that is not set means its step is skipped; the detection pass falls back to \
             the card model. Each takes the newest sessions first, a few per run. Embeddings \
             always run in Ollama on this machine.",
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
        .label("Always personal")
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
    let st = &d.draft.steward;
    let detection = match st.detail_model.as_ref().or(st.card_model.as_ref()) {
        Some(m) => pill(p, &format!("Patterns and {m}"), p.k.c.success),
        None => pill(p, "Patterns only", p.k.c.warning),
    };
    let automatic = p
        .section("automatic", "Found automatically", None)
        .gap(0.0)
        .child(setting(
            p,
            "secrets",
            "Secrets",
            "API keys, tokens and passwords, by pattern. Always on.",
            pill(p, "Always on", p.k.c.success),
        ))
        .child(p.separator())
        .child(setting(
            p,
            "details",
            "Personal details",
            "Email addresses, hosts, your username and common name and address shapes, by \
             pattern. The detection pass's model (Models step) also finds names, addresses, \
             phone numbers and companies that no pattern can.",
            detection.key("detection"),
        ))
        .child(p.note(
            "In this window both are blurred until you click them. A trusted AI gets each \
             secret as a placeholder and personal details as written.",
        ));
    col()
        .gap(16.0)
        .child(automatic)
        .child(
            p.section("personal-terms", "Always personal", None)
                .child(p.note(
                    "Names and words to treat as personal details even when nothing above \
                     catches them: clients, people, places. Matched as whole words, whatever \
                     their case.",
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
