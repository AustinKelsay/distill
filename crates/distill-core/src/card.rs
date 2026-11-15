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
    /// around it (open models often think aloud in `<think>…</think>` first, braces and all):
    /// the last JSON object in the answer that is a valid card wins.
    pub fn from_answer(answer: &str) -> Result<Self> {
        let answer = strip_thinking(answer);
        if !answer.contains('{') {
            bail!("answer has no JSON object");
        }
        let mut last_err = None;
        for (i, _) in answer
            .match_indices('{')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let mut objects =
                serde_json::Deserializer::from_str(&answer[i..]).into_iter::<CardDraft>();
            match objects.next() {
                Some(Ok(draft)) => match draft.check() {
                    Ok(()) => return Ok(draft),
                    Err(e) => last_err = Some(e),
                },
                Some(Err(e)) => {
                    last_err.get_or_insert_with(|| {
                        anyhow::Error::new(e).context("answer is not a valid card")
                    });
                }
                None => {}
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

/// The answer without `<think>…</think>` blocks (an unclosed one runs to the end).
fn strip_thinking(answer: &str) -> String {
    let mut out = String::with_capacity(answer.len());
    let mut rest = answer;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
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
