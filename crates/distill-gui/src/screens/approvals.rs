//! Approvals (#87): the reveal requests waiting for the owner, and the one place a request is
//! approved or denied (ADR 0004). Each request shows exactly what it asks for: the reader, each
//! secret by kind and fingerprint, the session, the reason in the reader's words, and the time
//! left before it expires. A request names secrets by fingerprint and holds no value, so no value
//! is ever on this screen.
//!
//! Approving takes two presses: the first names what will be released and to whom, the second
//! approves. Denying takes one. Both call core (`reveal::approve`, `reveal::deny`) on one request,
//! then reload the list; a request answered elsewhere, or expired, says so.

use distill_core::reveal::{self, Request};
use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::State;
use crate::cover::chip;
use crate::jobs::{self, Job};
use crate::nav::Screen;
use crate::screens::sessions;
use crate::ui::{Paint, ago, count, left, pill};

/// How often the list reloads while it shows: requests arrive while the window is open.
const REFRESH_SECS: f64 = 30.0;

#[derive(Default)]
pub struct ApprovalsState {
    /// The requests waiting, newest first; None until the first list lands.
    pub requests: Option<Vec<Request>>,
    pub loading: bool,
    /// Why the list could not be read.
    pub list_error: Option<String>,
    /// Why the last answer, or opening a session, failed.
    pub error: Option<String>,
    /// The request whose approval waits for its second press.
    pub confirming: Option<String>,
    /// The request being answered, so a second press does nothing.
    pub answering: Option<String>,
    /// What the last answer did.
    pub notice: Option<String>,
    pub next_refresh: f64,
}

pub fn open(s: &mut State) {
    s.approvals.confirming = None;
    s.approvals.error = None;
    load(s);
}

/// Ask core for the requests waiting.
pub fn load(s: &mut State) {
    s.approvals.loading = true;
    s.ask(Job::new("reveals", |core| {
        let r = reveal::pending(&core.home).map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let st = &mut s.approvals;
            st.loading = false;
            match r {
                Ok(list) => {
                    if st
                        .confirming
                        .as_ref()
                        .is_some_and(|id| !list.iter().any(|r| &r.id == id))
                    {
                        st.confirming = None;
                    }
                    // The navigation's count follows the list as soon as it changes.
                    if let Some(status) = s.overview.status.as_mut() {
                        status.pending_reveals = list.len() as u64;
                    }
                    s.approvals.requests = Some(list);
                    s.approvals.list_error = None;
                }
                Err(e) => st.list_error = Some(format!("Could not list the requests: {e}")),
            }
        })
    }));
}

/// Reload every [`REFRESH_SECS`] while the screen shows; returns how long to sleep.
pub fn refresh(s: &mut State, clock: f64) -> Option<f64> {
    if s.screen != Screen::Approvals {
        return None;
    }
    if clock >= s.approvals.next_refresh {
        if s.approvals.next_refresh > 0.0 && !s.approvals.loading {
            load(s);
        }
        s.approvals.next_refresh = clock + REFRESH_SECS;
    }
    Some((s.approvals.next_refresh - clock).max(0.5))
}

/// The first press on Approve: name what will be released, and wait for the second.
fn ask_to_approve(s: &mut State, id: &str) {
    s.approvals.confirming = Some(id.to_string());
    s.approvals.notice = None;
}

fn cancel(s: &mut State) {
    s.approvals.confirming = None;
}

/// Approve or deny request `id`, then reload the list.
fn answer(s: &mut State, id: &str, approve: bool) {
    let st = &mut s.approvals;
    if st.answering.is_some() {
        return;
    }
    st.answering = Some(id.to_string());
    st.confirming = None;
    let id = id.to_string();
    let what = if approve {
        "approve reveal"
    } else {
        "deny reveal"
    };
    s.ask(Job::new(what, move |core| {
        let r = if approve {
            reveal::approve(&core.home, &id)
        } else {
            reveal::deny(&core.home, &id)
        }
        .map_err(jobs::line);
        Box::new(move |s: &mut State| {
            let st = &mut s.approvals;
            st.answering = None;
            match r {
                Ok(r) if approve => {
                    st.notice = Some(format!(
                        "Approved. {} may collect {} from {} once, until {}.",
                        r.reader,
                        count(r.secrets.len() as u64, "secret"),
                        r.session,
                        when(&r.expires_at)
                    ));
                    st.error = None;
                }
                Ok(r) => {
                    st.notice = Some(format!(
                        "Denied. {} gets nothing from request {}.",
                        r.reader, r.id
                    ));
                    st.error = None;
                }
                Err(e) => {
                    st.notice = None;
                    st.error = Some(format!(
                        "Could not {} it: {e}",
                        if approve { "approve" } else { "deny" }
                    ));
                }
            }
            load(s);
        })
    }));
}

/// `Nov 16, 14:05 UTC` from an RFC 3339 time.
fn when(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .format("%b %-d, %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.to_string())
}

pub fn view(s: &State, p: &Paint, _: &mut Vec<El>) -> El {
    let st = &s.approvals;
    let mut head = col()
        .key("header")
        .w_full()
        .px(28.0)
        .pt(20.0)
        .pb(16.0)
        .gap(4.0)
        .child(
            text("Approvals")
                .key("heading")
                .font(p.title())
                .color(p.ink()),
        )
        .child(p.note(
            "A model at a trusted provider asks for the real value of specific secrets. \
             Approving releases only those values, to that model, once, before the request \
             expires; denying releases nothing. No value is shown here.",
        ));
    if let Some(n) = &st.notice {
        head = head.child(
            text(n.clone())
                .key("notice")
                .font(p.text())
                .color(p.good())
                .wrap(),
        );
    }
    for (key, e) in [("list-error", &st.list_error), ("error", &st.error)] {
        let Some(e) = e else { continue };
        head = head.child(
            text(e.clone())
                .key(key)
                .font(p.text())
                .color(p.bad())
                .wrap(),
        );
    }
    let mut list = col().key("requests").w_full().px(28.0).py(16.0).gap(12.0);
    match &st.requests {
        None => list = list.child(p.empty("loading", "Reading the requests…")),
        Some(rs) if rs.is_empty() => {
            list = list.child(p.empty("none", "No reveal requests are waiting."))
        }
        Some(rs) => {
            for r in rs {
                list = list.child(request(s, r, p));
            }
        }
    }
    col()
        .w_full()
        .h_full()
        .child(head)
        .child(p.separator())
        .child(
            col()
                .key("body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

/// One field of a request: a label on the left, what it says on the right.
fn field(p: &Paint, key: &str, label: &str, value: El) -> El {
    row()
        .key(key)
        .w_full()
        .gap(12.0)
        .child(
            text(label)
                .font(p.caption())
                .color(p.muted())
                .w(80.0)
                .a11y_hidden(),
        )
        .child(value.grow(1.0).min_w(0.0))
}

fn request(s: &State, r: &Request, p: &Paint) -> El {
    let st = &s.approvals;
    let n = r.secrets.len() as u64;
    let head = row()
        .key("head")
        .w_full()
        .gap(10.0)
        .center_y()
        .justify_between()
        .child(
            text(format!("{} asks for {}", r.reader, count(n, "secret")))
                .key("asks")
                .font(p.heading())
                .color(p.ink())
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(pill(p, &left(&r.expires_at, s.now()), p.k.c.warning).key("left"));

    let mut secrets = col().gap(4.0);
    for (i, x) in r.secrets.iter().enumerate() {
        secrets = secrets.child(
            text(format!("{}  #{}", chip(x.kind), x.fingerprint.short()))
                .key(format!("secret-{i}"))
                .font(p.mono())
                .color(p.ink()),
        );
    }
    let id = r.id.clone();
    let key = r.session.clone();
    let session = row()
        .gap(10.0)
        .center_y()
        .child(
            text(r.session.clone())
                .key("session")
                .font(p.mono())
                .color(p.ink())
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(
            Button::new("Open session")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| {
                    sessions::open_key(s, &key, Screen::Approvals, |s, e| {
                        s.approvals.error = Some(e)
                    })
                })
                .build(&p.k)
                .key(format!("open-session-{id}")),
        );
    let reason = text(format!("“{}”", r.reason))
        .key("reason")
        .font(p.text())
        .color(p.ink())
        .wrap()
        .selectable(true);
    let made = p
        .note(format!(
            "Asked {} · request {} · expires {}",
            ago(&r.made_at, s.now()),
            r.id,
            when(&r.expires_at)
        ))
        .key("made");

    let busy = st.answering.as_deref() == Some(r.id.as_str());
    let actions = if st.confirming.as_deref() == Some(r.id.as_str()) {
        let (yes, no) = (r.id.clone(), r.id.clone());
        row()
            .key("confirm")
            .w_full()
            .gap(8.0)
            .center_y()
            .padding(10.0)
            .radius(8.0)
            .fill(p.k.c.warning.bg)
            .border(p.hair, p.k.c.warning.border)
            .child(
                text(format!(
                    "Release {} to {} once? Only the value{} named above, nothing else.",
                    count(n, "secret"),
                    r.reader,
                    if n == 1 { "" } else { "s" }
                ))
                .key("confirm-text")
                .font(p.text())
                .color(p.k.c.warning.fg)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
            )
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_press(cancel)
                    .build(&p.k)
                    .key(format!("cancel-{}", r.id)),
            )
            .child(
                Button::new("Release once")
                    .variant(ButtonVariant::Warning)
                    .on_press(move |s: &mut State| answer(s, &yes, true))
                    .build(&p.k)
                    .key(format!("confirm-{no}")),
            )
    } else {
        let (a, d) = (r.id.clone(), r.id.clone());
        row()
            .key("actions")
            .w_full()
            .gap(8.0)
            .center_y()
            .justify_end()
            .child(
                Button::new("Deny")
                    .disabled(busy)
                    .on_press(move |s: &mut State| answer(s, &d, false))
                    .build(&p.k)
                    .key(format!("deny-{}", r.id)),
            )
            .child(
                Button::new(if busy { "Answering…" } else { "Approve…" })
                    .variant(ButtonVariant::Submit)
                    .disabled(busy)
                    .on_press(move |s: &mut State| ask_to_approve(s, &a))
                    .build(&p.k)
                    .key(format!("approve-{}", r.id)),
            )
    };

    p.card()
        .key(format!("request-{}", r.id))
        .role("group")
        .label(format!("Reveal request {} from {}", r.id, r.reader))
        .w_full()
        .child(head)
        .child(field(p, "secrets", "Secrets", secrets))
        .child(field(p, "where", "Session", session))
        .child(field(p, "why", "Reason", reason))
        .child(made)
        .child(p.separator())
        .child(actions)
}
