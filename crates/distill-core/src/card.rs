//! Agent-written session cards: what the session was for and how it went.

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::util::write_json_pretty;

pub const PROMPT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Partial,
    Failed,
    Abandoned,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    /// One sentence: what the user wanted.
    pub task: String,
    pub outcome: Outcome,
    /// 1 (useless as training data) to 5 (exemplary).
    pub quality: u8,
    /// Places the user had to correct or redirect the agent.
    #[serde(default)]
    pub corrections: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub summary: String,
    pub produced_by: CardProducer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardProducer {
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub prompt_version: u32,
    pub written_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// The raw capture the card was written from; a different capture makes the card stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_sha256: Option<String>,
}

/// The part of a card the agent writes. Distill adds `produced_by` itself.
#[derive(Debug, Clone, Deserialize)]
pub struct CardDraft {
    pub task: String,
    pub outcome: Outcome,
    pub quality: u8,
    #[serde(default)]
    pub corrections: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub summary: String,
}

impl CardDraft {
    /// Pull the card out of an agent's answer, tolerating prose, code fences and reasoning
    /// around it (open models often think aloud in `<think>…</think>` first, braces and all).
    /// The answer is scanned once for top-level JSON objects outside reasoning blocks; the
    /// last one that is a valid card wins. A card's own nested objects are never candidates,
    /// and text inside its strings is never touched.
    pub fn from_answer(answer: &str) -> Result<Self> {
        let objects = top_level_objects(answer);
        if objects.is_empty() {
            bail!("answer has no JSON object");
        }
        let mut last_err = None;
        for (start, end) in objects.into_iter().rev() {
            match serde_json::from_str::<CardDraft>(&answer[start..end]) {
                Ok(draft) => match draft.check() {
                    Ok(()) => return Ok(draft),
                    Err(e) => last_err = Some(e),
                },
                Err(e) => {
                    last_err.get_or_insert_with(|| {
                        anyhow::Error::new(e).context("answer is not a valid card")
                    });
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("answer is not a valid card")))
    }

    fn check(&self) -> Result<()> {
        if !(1..=5).contains(&self.quality) {
            bail!("quality {} is outside 1..=5", self.quality);
        }
        if self.task.trim().is_empty() {
            bail!("card has an empty task");
        }
        Ok(())
    }

    pub fn into_card(self, produced_by: CardProducer) -> Card {
        Card {
            task: self.task,
            outcome: self.outcome,
            quality: self.quality,
            corrections: self.corrections,
            tags: self.tags,
            summary: self.summary,
            produced_by,
        }
    }
}

/// Byte ranges of the JSON objects in `answer` that aren't inside another one, in order.
/// Each `{` is tried with a real JSON parser; an object it accepts is jumped over whole, so
/// nothing nested in it is a candidate and nothing in its strings is read as prose. Between
/// objects, `<think>…</think>` is skipped (an unclosed one runs to the end), and stray braces
/// and quotes in prose just fail to parse. The work is bounded: only the last 64 KB of an answer
/// is scanned, each try reads at most 16 KB (a card is a few hundred bytes), and tries are capped.
fn top_level_objects(answer: &str) -> Vec<(usize, usize)> {
    const WINDOW: usize = 64 * 1024;
    const MAX_OBJECT: usize = 16 * 1024;
    const MAX_TRIES: usize = 4096;
    let mut i = answer.len().saturating_sub(WINDOW);
    while !answer.is_char_boundary(i) {
        i += 1;
    }
    let mut found = Vec::new();
    let mut tries = 0;
    while i < answer.len() {
        let rest = &answer[i..];
        if rest.starts_with("<think>") {
            match rest.find("</think>") {
                Some(end) => {
                    i += end + "</think>".len();
                    continue;
                }
                None => break,
            }
        }
        if rest.starts_with('{') && tries < MAX_TRIES {
            tries += 1;
            let mut cut = rest.len().min(MAX_OBJECT);
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            let mut values = serde_json::Deserializer::from_str(&rest[..cut])
                .into_iter::<serde::de::IgnoredAny>();
            if let Some(Ok(_)) = values.next() {
                let end = i + values.byte_offset();
                found.push((i, end));
                i = end;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    found
}

/// Whether a card still describes its session: written with the current prompt from the
/// capture the session was last normalized from.
pub fn is_current(card: &Card, raw_sha256: &str) -> bool {
    card.produced_by.prompt_version >= PROMPT_VERSION
        && card.produced_by.input_sha256.as_deref() == Some(raw_sha256)
}

pub fn load(session_dir: &Path) -> Option<Card> {
    let text = std::fs::read_to_string(session_dir.join("card.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(session_dir: &Path, card: &Card) -> Result<()> {
    write_json_pretty(&session_dir.join("card.json"), card)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_extraction_respects_json_nesting_and_strings() {
        let card = |q: u8, summary: &str| {
            serde_json::json!({"task": "Fix the parser", "outcome": "success", "quality": q,
                "corrections": [], "tags": [], "summary": summary})
            .to_string()
        };
        // A nested example card never wins over the card that holds it.
        let outer = card(1, "x").replacen('{', &format!("{{\"example\":{},", card(5, "y")), 1);
        assert_eq!(CardDraft::from_answer(&outer).unwrap().quality, 1);
        // Tags and braces inside a string are left alone.
        let literal = card(3, "Kept <think>this</think> and {that} literally");
        assert_eq!(
            CardDraft::from_answer(&literal).unwrap().summary,
            "Kept <think>this</think> and {that} literally"
        );
        let open_tag = card(3, "An unmatched <think> tag");
        assert!(CardDraft::from_answer(&open_tag).is_ok());
        // A stray brace in prose before the card doesn't hide it.
        let stray = format!(
            "Using the schema {{task, outcome... here:\n{}",
            card(4, "z")
        );
        assert_eq!(CardDraft::from_answer(&stray).unwrap().quality, 4);
        // A stray brace and an unmatched quote in prose don't hide the card...
        let quoted = format!(
            "Schema {{ ... I'll use the \"task field here:\n{}",
            card(4, "q")
        );
        assert_eq!(CardDraft::from_answer(&quoted).unwrap().quality, 4);
        // ...and don't expose an example card inside reasoning after the real one.
        let after = format!(
            "{}\nNote: {{\n<think>{}</think>",
            card(2, "real"),
            card(5, "thought")
        );
        assert_eq!(CardDraft::from_answer(&after).unwrap().summary, "real");
        // Deeply nested junk is scanned once, not once per brace.
        let junk = format!("{}{}", "{\"x\":".repeat(2_000), card(2, "w"));
        assert_eq!(CardDraft::from_answer(&junk).unwrap().quality, 2);
    }

    #[test]
    fn a_card_is_found_after_an_open_models_reasoning() {
        let card = r#"{"task":"Fix the parser","outcome":"success","quality":4,"corrections":[],"tags":["rust"],"summary":"Done."}"#;
        // Braces in the reasoning, prose and a fence around the card, a stray brace after it.
        let answer = format!(
            "<think>The schema is {{task, outcome}}; maybe {{\"quality\": 9}}?</think>\nHere it is:\n```json\n{card}\n```\nNote: {{ends here"
        );
        let draft = CardDraft::from_answer(&answer).unwrap();
        assert_eq!((draft.task.as_str(), draft.quality), ("Fix the parser", 4));
        // Without a closing tag the reasoning runs to the end, so there is no card.
        assert!(CardDraft::from_answer(&format!("<think>{card}")).is_err());
        // An invalid card is still rejected.
        assert!(CardDraft::from_answer(&card.replace("\"quality\":4", "\"quality\":9")).is_err());
    }

    #[test]
    fn draft_is_found_inside_prose_and_fences() {
        let answer = "Here you go:\n```json\n{\"task\":\"Fix the build\",\"outcome\":\"success\",\"quality\":4,\"summary\":\"Fixed it.\"}\n```";
        let d = CardDraft::from_answer(answer).unwrap();
        assert_eq!(d.outcome, Outcome::Success);
        assert_eq!(d.quality, 4);
    }

    #[test]
    fn out_of_range_quality_is_rejected() {
        let answer = "{\"task\":\"x\",\"outcome\":\"failed\",\"quality\":9,\"summary\":\"\"}";
        assert!(CardDraft::from_answer(answer).is_err());
    }
}
