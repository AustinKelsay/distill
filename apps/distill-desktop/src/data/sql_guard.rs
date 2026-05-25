use anyhow::{Result, bail};

const FORBIDDEN_KEYWORDS: &[&str] = &[
    "insert", "update", "delete", "alter", "drop", "create", "replace", "attach",
    "detach", "vacuum", "reindex", "analyze",
];

pub fn guard_read_only_sql(sql: &str) -> Result<String> {
    let normalized = ensure_single_statement_sql(sql)?;
    let head = statement_head(&normalized);
    let allowed = matches!(head.as_str(), "select" | "with" | "pragma" | "explain");
    if !allowed {
        bail!("Only read-only SELECT, WITH, PRAGMA, and EXPLAIN statements are allowed.");
    }

    if contains_forbidden_sql(&normalized) {
        bail!("Mutating SQL is not allowed in the desktop shell.");
    }

    Ok(normalized)
}

fn contains_forbidden_sql(sql: &str) -> bool {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Mode {
        Normal,
        SingleQuote,
        DoubleQuote,
        Backtick,
        Bracket,
        LineComment,
        BlockComment,
    }

    let chars = sql.chars().collect::<Vec<_>>();
    let mut mode = Mode::Normal;
    let mut index = 0usize;
    let mut word_start = 0usize;

    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();

        match mode {
            Mode::LineComment => {
                if current == '\n' {
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::BlockComment => {
                if current == '*' && next == Some('/') {
                    mode = Mode::Normal;
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            Mode::SingleQuote => {
                if current == '\'' {
                    if next == Some('\'') {
                        index += 2;
                        continue;
                    }
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::DoubleQuote => {
                if current == '"' {
                    if next == Some('"') {
                        index += 2;
                        continue;
                    }
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::Backtick => {
                if current == '`' {
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::Bracket => {
                if current == ']' {
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::Normal => {}
        }

        if current == '-' && next == Some('-') {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 2;
            mode = Mode::LineComment;
            index += 2;
            continue;
        }
        if current == '/' && next == Some('*') {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 2;
            mode = Mode::BlockComment;
            index += 2;
            continue;
        }
        if current == '\'' {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 1;
            mode = Mode::SingleQuote;
            index += 1;
            continue;
        }
        if current == '"' {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 1;
            mode = Mode::DoubleQuote;
            index += 1;
            continue;
        }
        if current == '`' {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 1;
            mode = Mode::Backtick;
            index += 1;
            continue;
        }
        if current == '[' {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 1;
            mode = Mode::Bracket;
            index += 1;
            continue;
        }

        if current.is_whitespace()
            || matches!(current, ';' | '(' | ')' | ',' | '.' | '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/')
        {
            if index > word_start && is_forbidden_word(&chars, word_start, index) {
                return true;
            }
            word_start = index + 1;
        } else {
            // accumulate
        }
        index += 1;
    }
    if chars.len() > word_start && is_forbidden_word(&chars, word_start, chars.len()) {
        return true;
    }

    false
}

fn is_forbidden_word(chars: &[char], start: usize, end: usize) -> bool {
    let word: String = chars[start..end].iter().map(|c| c.to_ascii_lowercase()).collect();
    FORBIDDEN_KEYWORDS.contains(&word.as_str())
}

fn statement_head(sql: &str) -> String {
    sql.split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn ensure_single_statement_sql(sql: &str) -> Result<String> {
    if sql.trim().is_empty() {
        bail!("SQL query is required.");
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Mode {
        Normal,
        SingleQuote,
        DoubleQuote,
        Backtick,
        Bracket,
        LineComment,
        BlockComment,
    }

    let chars = sql.chars().collect::<Vec<_>>();
    let mut mode = Mode::Normal;
    let mut index = 0usize;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();

        match mode {
            Mode::LineComment => {
                if current == '\n' {
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::BlockComment => {
                if current == '*' && next == Some('/') {
                    mode = Mode::Normal;
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            Mode::SingleQuote => {
                if current == '\'' {
                    if next == Some('\'') {
                        index += 2;
                        continue;
                    }
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::DoubleQuote => {
                if current == '"' {
                    if next == Some('"') {
                        index += 2;
                        continue;
                    }
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::Backtick => {
                if current == '`' {
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::Bracket => {
                if current == ']' {
                    mode = Mode::Normal;
                }
                index += 1;
                continue;
            }
            Mode::Normal => {}
        }

        if current == '-' && next == Some('-') {
            mode = Mode::LineComment;
            index += 2;
            continue;
        }
        if current == '/' && next == Some('*') {
            mode = Mode::BlockComment;
            index += 2;
            continue;
        }
        if current == '\'' {
            mode = Mode::SingleQuote;
            index += 1;
            continue;
        }
        if current == '"' {
            mode = Mode::DoubleQuote;
            index += 1;
            continue;
        }
        if current == '`' {
            mode = Mode::Backtick;
            index += 1;
            continue;
        }
        if current == '[' {
            mode = Mode::Bracket;
            index += 1;
            continue;
        }

        if current == ';' {
            let remainder = chars[index + 1..].iter().collect::<String>();
            if remainder
                .chars()
                .any(|character| !character.is_whitespace())
            {
                bail!("Only one SQL statement is allowed per query.");
            }
        }

        index += 1;
    }

    Ok(sql.trim().trim_end_matches(';').trim().to_string())
}
