pub const ELECTRON_SCHEMA_SQL: &str = include_str!("../../../distill-electron/schema.sql");

pub const SCHEMA_MIGRATIONS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

pub const SOURCE_SEEDS: [(&str, &str); 3] = [
    ("codex", "Codex"),
    ("claude_code", "Claude Code"),
    ("opencode", "OpenCode"),
];

pub const LABEL_SEEDS: [&str; 5] = ["train", "holdout", "exclude", "sensitive", "favorite"];
