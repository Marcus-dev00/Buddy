//! Runtime-editable Markdown prompt architecture:
//! - identity.md     (Who Buddy is)
//! - personality.md  (Selectable personality presets via `## id | Label`)
//! - rules.md        (Language mirroring, honesty, and formatting rules)

use serde::Serialize;
use std::path::{Path, PathBuf};

const DEFAULT_IDENTITY_MD: &str = include_str!("../../prompts/identity.md");
const DEFAULT_PERSONALITY_MD: &str = include_str!("../../prompts/personality.md");
const DEFAULT_RULES_MD: &str = include_str!("../../prompts/rules.md");

#[derive(Clone, Debug, Serialize)]
pub struct PersonalityOption {
    pub id: String,
    pub label: String,
}

pub struct PromptsManager {
    pub prompts_dir: PathBuf,
}

impl PromptsManager {
    pub fn init(data_dir: &Path) -> Self {
        let prompts_dir = data_dir.join("prompts");
        let _ = std::fs::create_dir_all(&prompts_dir);

        let files = [
            ("identity.md", DEFAULT_IDENTITY_MD),
            ("personality.md", DEFAULT_PERSONALITY_MD),
            ("rules.md", DEFAULT_RULES_MD),
        ];

        for (name, default_content) in files {
            let path = prompts_dir.join(name);
            let should_write = if !path.exists() {
                true
            } else if let Ok(existing) = std::fs::read_to_string(&path) {
                // Upgrade unmodified v1 templates automatically while keeping custom user edits
                match name {
                    "identity.md" => !existing.contains("buddy-prompts-v4") && existing.contains("Dynamic Island capsule"),
                    "personality.md" => {
                        !existing.contains("buddy-prompts-v3")
                            || existing.contains("Plan` / `Work` / `Auto")
                    }
                    "rules.md" => !existing.contains("buddy-prompts-v4") && !existing.contains("Anti-Sycophancy"),
                    _ => false,
                }
            } else {
                false
            };

            if should_write {
                let _ = std::fs::write(&path, default_content);
            }
        }

        Self { prompts_dir }
    }

    fn read_or_default(&self, filename: &str, fallback: &str) -> String {
        std::fs::read_to_string(self.prompts_dir.join(filename))
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string())
    }

    fn parse_personality_sections(raw: &str) -> Vec<(String, String, String)> {
        let mut sections: Vec<(String, String, Vec<String>)> = Vec::new();

        for line in raw.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("## ") {
                let mut parts = rest.splitn(2, '|');
                let id = parts.next().unwrap_or("").trim().to_string();
                let label = parts
                    .next()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| id.clone());
                if !id.is_empty() {
                    sections.push((id, label, Vec::new()));
                }
            } else if let Some((_, _, body)) = sections.last_mut() {
                body.push(line.to_string());
            }
        }

        sections
            .into_iter()
            .map(|(id, label, lines)| (id, label, lines.join("\n").trim().to_string()))
            .collect()
    }

    pub fn list_personalities(&self) -> Vec<PersonalityOption> {
        let raw = self.read_or_default("personality.md", DEFAULT_PERSONALITY_MD);
        let parsed = Self::parse_personality_sections(&raw);
        if parsed.is_empty() {
            return vec![PersonalityOption {
                id: "warm_companion".to_string(),
                label: "Desktop Companion".to_string(),
            }];
        }
        parsed
            .into_iter()
            .map(|(id, label, _)| PersonalityOption { id, label })
            .collect()
    }

    pub fn build_system_prompt(&self, persona_id: &str) -> String {
        let identity = self.read_or_default("identity.md", DEFAULT_IDENTITY_MD);
        let personality_raw = self.read_or_default("personality.md", DEFAULT_PERSONALITY_MD);
        let rules = self.read_or_default("rules.md", DEFAULT_RULES_MD);

        let sections = Self::parse_personality_sections(&personality_raw);
        let selected = sections
            .iter()
            .find(|(id, _, _)| id == persona_id)
            .or_else(|| sections.first());

        let personality_block = if let Some((_, label, body)) = selected {
            format!("## Active Personality: {label}\n{body}")
        } else {
            "## Active Personality\nBe concise, clear, and helpful.".to_string()
        };

        format!(
            "{}\n\n---\n\n{}\n\n---\n\n{}",
            identity.trim(),
            personality_block.trim(),
            rules.trim()
        )
    }

    pub fn open_folder(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            std::process::Command::new("explorer.exe")
                .arg(&self.prompts_dir)
                .spawn()
                .map_err(|e| format!("failed to open prompts folder: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_companion_personality_and_rules_hierarchy() {
        let dir = std::env::temp_dir().join("buddy_prompts_test");
        let _ = std::fs::remove_dir_all(&dir);
        let mgr = PromptsManager::init(&dir);

        let list = mgr.list_personalities();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "warm_companion");

        let prompt = mgr.build_system_prompt("warm_companion");
        assert!(prompt.contains("Desktop Companion"));
        assert!(prompt.contains("Strict Priority Hierarchy"));
        assert!(prompt.contains("Strict Action Truthfulness"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
