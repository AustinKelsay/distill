//! Agent-written session cards: what the session was for and how it went.

use std::path::Path;

use anyhow::{Context, Result, bail};
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
    /// Pull the first JSON object out of an agent's answer, tolerating prose or code fences
    /// around it, and check it.
    pub fn from_answer(answer: &str) -> Result<Self> {
        let start = answer.find('{').context("answer has no JSON object")?;
        let end = answer.rfind('}').context("answer has no JSON object")?;
        if end < start {
            bail!("answer has no JSON object");
        }
        let draft: CardDraft =
            serde_json::from_str(&answer[start..=end]).context("answer is not a valid card")?;
        if !(1..=5).contains(&draft.quality) {
            bail!("quality {} is outside 1..=5", draft.quality);
        }
        if draft.task.trim().is_empty() {
            bail!("card has an empty task");
        }
        Ok(draft)
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
