//! GitHub & Local Agent Skills Manager (%LOCALAPPDATA%/Buddy/skills/):
//! - Standard `SKILL.md` (YAML frontmatter `name` & `description` + optional `scripts/`, `references/`, etc.)
//! - Import from local folder / `.zip`, or download from GitHub (`owner/repo`, `tree/<branch>/<subpath>`, `SKILL.md` links)
//! - GitHub skill search & multi-skill monorepo discovery
//! - Progressive prompt indexing (concise metadata in system prompt; full `SKILL.md` loaded on demand via `read_skill`)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub source: String,
    pub dir_path: String,
    pub has_scripts: bool,
    pub file_count: usize,
    pub installed_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillDetail {
    pub info: SkillInfo,
    pub skill_md: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GithubSkillCandidate {
    pub title: String,
    pub description: String,
    pub install_spec: String,
    pub html_url: String,
    pub stars: u64,
    pub is_repo: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedSkillEntry {
    enabled: bool,
    source: String,
    installed_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedSkillStore {
    #[serde(default)]
    entries: HashMap<String, PersistedSkillEntry>,
    #[serde(default)]
    seeded_builtins: Vec<String>,
}

pub struct SkillsManager {
    pub skills_dir: PathBuf,
    state_path: PathBuf,
    store: Mutex<PersistedSkillStore>,
}

fn now_sec() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        let low = ch.to_ascii_lowercase();
        if low.is_ascii_alphanumeric() {
            out.push(low);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "custom-skill".to_string()
    } else {
        out.chars().take(48).collect()
    }
}

/// Parses `SKILL.md` YAML frontmatter (`name`, `description`) cleanly, falling back to Markdown heading/body.
pub fn parse_skill_md(raw: &str, fallback_id: &str) -> (String, String, String) {
    let norm = raw.replace("\r\n", "\n");
    let mut name = String::new();
    let mut desc = String::new();
    let mut body = norm.as_str();

    if let Some(rest) = norm.strip_prefix("---\n") {
        if let Some(end_idx) = rest.find("\n---") {
            let fm = &rest[..end_idx];
            body = rest[end_idx + 4..].trim_start_matches('\n');
            let lines: Vec<&str> = fm.lines().collect();
            let mut i = 0;
            while i < lines.len() {
                let line = lines[i];
                let trimmed = line.trim();
                if let Some((k, v)) = trimmed.split_once(':') {
                    let key = k.trim().to_ascii_lowercase();
                    let val_raw = v.trim();
                    let parsed_val = if val_raw == "|" || val_raw == ">" || val_raw == "|-" || val_raw == ">-" {
                        let mut block = Vec::new();
                        let mut j = i + 1;
                        while j < lines.len() && (lines[j].starts_with(' ') || lines[j].starts_with('\t') || lines[j].trim().is_empty()) {
                            if !lines[j].trim().is_empty() {
                                block.push(lines[j].trim());
                            }
                            j += 1;
                        }
                        i = j - 1;
                        block.join(" ")
                    } else {
                        val_raw
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string()
                    };
                    if key == "name" && !parsed_val.is_empty() {
                        name = parsed_val;
                    } else if key == "description" && !parsed_val.is_empty() {
                        desc = parsed_val;
                    }
                }
                i += 1;
            }
        }
    }

    if name.is_empty() {
        for line in body.lines() {
            let t = line.trim();
            if let Some(h) = t.strip_prefix("# ") {
                if !h.trim().is_empty() {
                    name = h.trim().to_string();
                    break;
                }
            }
        }
    }
    if name.is_empty() {
        name = fallback_id.to_string();
    }

    if desc.is_empty() {
        for line in body.lines() {
            let t = line.trim();
            if !t.is_empty() && !t.starts_with('#') && !t.starts_with("```") {
                desc = t.chars().take(180).collect();
                break;
            }
        }
    }
    if desc.is_empty() {
        desc = "Custom Agent Skill".to_string();
    }

    (name, desc, norm)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubSpec {
    pub owner: String,
    pub repo: String,
    pub branch: Option<String>,
    pub subpath: Option<String>,
}

/// Parses `owner/repo`, `https://github.com/owner/repo`, or `/tree/<branch>/<subpath>` URLs.
pub fn parse_github_spec(raw: &str) -> Result<GithubSpec, String> {
    let mut s = raw.trim().trim_end_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped.to_string();
    }

    if let Some(rest) = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))
        .or_else(|| s.strip_prefix("github.com/"))
    {
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            return Err("Invalid GitHub URL (expected owner/repo)".into());
        }
        let owner = parts[0].to_string();
        let repo = parts[1].to_string();
        if parts.len() >= 4 && (parts[2] == "tree" || parts[2] == "blob") {
            let branch = parts[3].to_string();
            let mut sub_parts = parts[4..].to_vec();
            if parts[2] == "blob"
                && sub_parts
                    .last()
                    .map(|f| f.eq_ignore_ascii_case("SKILL.md"))
                    .unwrap_or(false)
            {
                sub_parts.pop();
            }
            let subpath = if sub_parts.is_empty() {
                None
            } else {
                Some(sub_parts.join("/"))
            };
            return Ok(GithubSpec {
                owner,
                repo,
                branch: Some(branch),
                subpath,
            });
        }
        return Ok(GithubSpec {
            owner,
            repo,
            branch: None,
            subpath: None,
        });
    }

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() == 2 && !s.contains(':') && !s.contains('\\') {
        return Ok(GithubSpec {
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            branch: None,
            subpath: None,
        });
    }

    Err("Expected `owner/repo` or `https://github.com/owner/repo` URL".into())
}

fn apply_mirror(url: &str, mirror_prefix: &str) -> String {
    let m = mirror_prefix.trim();
    if m.is_empty() || !url.starts_with("https://github.com/") {
        return url.to_string();
    }
    if m.ends_with('/') {
        format!("{m}{url}")
    } else {
        format!("{m}/{url}")
    }
}

fn collect_skill_files(root: &Path, cur: &Path, out: &mut Vec<String>) {
    if out.len() >= 200 {
        return;
    }
    if let Ok(rd) = fs::read_dir(cur) {
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                collect_skill_files(root, &p, out);
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" || name == "__pycache__" {
            continue;
        }
        let target = dst.join(&name);
        if p.is_dir() {
            copy_dir_recursive(&p, &target)?;
        } else {
            fs::copy(&p, &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

const BUILTIN_OCR_SKILL_MD: &str = r#"---
name: ocr
description: Extract, clean, translate, or summarize text from screenshots, images, or 7-Day Memory captures using fast Windows offline OCR (zero extra vision tokens needed when text extraction is the goal).
---

# Offline Screenshot & Image OCR

Use Buddy's native `offline_ocr` tool whenever the user invokes `/ocr` or asks to extract/read/copy/translate text from an image, attached screenshot, or 7-Day Memory screenshot.

## Workflow
1. Identify the target image path from the current message attachments (`file_path`).
   - If no image is attached and the user asks about a recent screenshot, ask them to attach one from **7-Day Memory** (or paste/drop an image), or locate the image path if provided.
2. Call `offline_ocr` with `{ "source": "<image_path>" }`.
3. Present the extracted text clearly in a fenced block or cleanly formatted Markdown, fixing obvious line breaks only if requested.
4. If the user asked for translation, JSON extraction, code cleanup, or explanation after `/ocr`, perform that transformation directly on the OCR text.
"#;

impl SkillsManager {
    pub fn init(data_dir: &Path) -> Self {
        let skills_dir = data_dir.join("skills");
        let _ = fs::create_dir_all(&skills_dir);
        let state_path = skills_dir.join("state.json");

        let mut store: PersistedSkillStore = fs::read_to_string(&state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // Seed built-in `ocr` starter skill once so `/ocr` works out of the box
        if !store.seeded_builtins.iter().any(|s| s == "ocr") {
            let ocr_dir = skills_dir.join("ocr");
            let ocr_md = ocr_dir.join("SKILL.md");
            if !ocr_md.exists() {
                let _ = fs::create_dir_all(&ocr_dir);
                let _ = fs::write(&ocr_md, BUILTIN_OCR_SKILL_MD);
            }
            store.seeded_builtins.push("ocr".to_string());
            store.entries.entry("ocr".to_string()).or_insert(PersistedSkillEntry {
                enabled: true,
                source: "builtin".to_string(),
                installed_at: now_sec(),
            });
            if let Ok(json) = serde_json::to_string_pretty(&store) {
                let _ = fs::write(&state_path, json);
            }
        }

        Self {
            skills_dir,
            state_path,
            store: Mutex::new(store),
        }
    }

    fn save_store(&self, guard: &PersistedSkillStore) {
        if let Ok(json) = serde_json::to_string_pretty(guard) {
            let _ = fs::write(&self.state_path, json);
        }
    }

    fn record_skill(&self, id: &str, source: &str) {
        if let Ok(mut g) = self.store.lock() {
            g.entries.insert(
                id.to_string(),
                PersistedSkillEntry {
                    enabled: true,
                    source: source.to_string(),
                    installed_at: now_sec(),
                },
            );
            self.save_store(&g);
        }
    }

    pub fn list(&self) -> Vec<SkillInfo> {
        let store = self.store.lock().map(|g| g.clone()).unwrap_or_default();
        let mut out = Vec::new();

        if let Ok(rd) = fs::read_dir(&self.skills_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let md_path = p.join("SKILL.md");
                if !md_path.exists() {
                    continue;
                }
                let id = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let raw_md = fs::read_to_string(&md_path).unwrap_or_default();
                let (name, description, _) = parse_skill_md(&raw_md, &id);
                let mut files = Vec::new();
                collect_skill_files(&p, &p, &mut files);
                let has_scripts = p.join("scripts").is_dir()
                    || files.iter().any(|f| {
                        f.ends_with(".py") || f.ends_with(".ps1") || f.ends_with(".js") || f.ends_with(".sh")
                    });
                let meta = store.entries.get(&id);
                out.push(SkillInfo {
                    id,
                    name,
                    description,
                    enabled: meta.map(|m| m.enabled).unwrap_or(true),
                    source: meta.map(|m| m.source.clone()).unwrap_or_else(|| "local".into()),
                    dir_path: p.to_string_lossy().to_string(),
                    has_scripts,
                    file_count: files.len(),
                    installed_at: meta.map(|m| m.installed_at).unwrap_or(0),
                });
            }
        }

        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    pub fn get_detail(&self, id_or_name: &str) -> Result<SkillDetail, String> {
        let q = id_or_name.trim().to_ascii_lowercase();
        let list = self.list();
        let info = list
            .into_iter()
            .find(|s| s.id.to_ascii_lowercase() == q || s.name.to_ascii_lowercase() == q)
            .ok_or_else(|| format!("Skill not found: {id_or_name}"))?;

        let dir = Path::new(&info.dir_path);
        let skill_md = fs::read_to_string(dir.join("SKILL.md")).map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        collect_skill_files(dir, dir, &mut files);
        Ok(SkillDetail {
            info,
            skill_md,
            files,
        })
    }

    pub fn toggle(&self, id: &str, enabled: bool) -> Vec<SkillInfo> {
        if let Ok(mut g) = self.store.lock() {
            let e = g.entries.entry(id.to_string()).or_insert_with(|| PersistedSkillEntry {
                enabled,
                source: "local".into(),
                installed_at: now_sec(),
            });
            e.enabled = enabled;
            self.save_store(&g);
        }
        self.list()
    }

    pub fn uninstall(&self, id: &str) -> Result<Vec<SkillInfo>, String> {
        let safe = slugify(id);
        let dir = self.skills_dir.join(&safe);
        if dir.exists() && dir.starts_with(&self.skills_dir) {
            fs::remove_dir_all(&dir).map_err(|e| format!("Failed removing skill: {e}"))?;
        }
        if let Ok(mut g) = self.store.lock() {
            g.entries.remove(id);
            g.entries.remove(&safe);
            self.save_store(&g);
        }
        Ok(self.list())
    }

    pub fn open_folder(&self, id: Option<&str>) {
        let target = match id {
            Some(i) if !i.trim().is_empty() => {
                let d = self.skills_dir.join(slugify(i));
                if d.exists() { d } else { self.skills_dir.clone() }
            }
            _ => self.skills_dir.clone(),
        };
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer.exe").arg(target).spawn();
        }
        #[cfg(not(windows))]
        let _ = target;
    }

    /// Compact progressive skill directory injected into system prompt (zero context bloat).
    pub fn build_prompt_appendix(&self) -> String {
        let enabled: Vec<SkillInfo> = self.list().into_iter().filter(|s| s.enabled).collect();
        if enabled.is_empty() {
            return String::new();
        }
        let mut lines = vec![
            "\n\n---\n\n## Installed Agent Skills".to_string(),
            "The user has enabled the following modular skills under `%LOCALAPPDATA%/Buddy/skills/`. When a request matches a skill's scope (or user mentions `/skill-id`), call `read_skill` with its `id` FIRST to load its full `SKILL.md` instructions before acting. Scripts inside a skill directory require `run_shell` (with user approval).".to_string(),
        ];
        for s in enabled {
            lines.push(format!(
                "- `{}` (**{}**): {} *(dir: `{}`)*",
                s.id,
                s.name,
                s.description.replace('\n', " "),
                s.dir_path
            ));
        }
        lines.join("\n")
    }

    /// Detects explicit `/skill-id` invocations in a chat message and pre-loads their full `SKILL.md`.
    pub fn build_explicit_slash_appendix(&self, prompt: &str) -> String {
        let enabled: Vec<SkillInfo> = self.list().into_iter().filter(|s| s.enabled).collect();
        if enabled.is_empty() {
            return String::new();
        }
        let mut matched_ids: Vec<String> = Vec::new();
        for tok in prompt.split_whitespace() {
            let trimmed = tok.trim_matches(|c: char| matches!(c, ',' | '.' | ':' | ';' | '!' | '?' | '，' | '。'));
            if let Some(raw_cmd) = trimmed.strip_prefix('/') {
                let q = raw_cmd.trim().to_ascii_lowercase();
                if q.is_empty() {
                    continue;
                }
                if let Some(found) = enabled
                    .iter()
                    .find(|s| s.id.to_ascii_lowercase() == q || s.name.to_ascii_lowercase() == q)
                {
                    if !matched_ids.contains(&found.id) {
                        matched_ids.push(found.id.clone());
                    }
                }
            }
        }
        if matched_ids.is_empty() {
            return String::new();
        }
        let mut out = vec![
            "\n\n---\n\n## Explicitly Invoked Skill(s) via `/command`".to_string(),
            "The user explicitly invoked the following Skill(s) in this message. Follow their `SKILL.md` instructions immediately:".to_string(),
        ];
        for id in matched_ids {
            if let Ok(det) = self.get_detail(&id) {
                out.push(format!(
                    "\n### Skill `/{}` (**{}**)\nDir: `{}`\nFiles: [{}]\n\n```markdown\n{}\n```",
                    det.info.id,
                    det.info.name,
                    det.info.dir_path,
                    det.files.join(", "),
                    det.skill_md
                ));
            }
        }
        out.join("\n")
    }

    /// Imports a local Skill directory (or `.zip` containing `SKILL.md`) into `%LOCALAPPDATA%/Buddy/skills/`.
    pub fn import_local(&self, raw_path: &str) -> Result<SkillInfo, String> {
        let src = Path::new(raw_path.trim().trim_matches('"'));
        if !src.exists() {
            return Err(format!("Path does not exist: {}", src.display()));
        }

        if src.is_file()
            && src
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("zip"))
                .unwrap_or(false)
        {
            let installed = install_skill_from_zip(
                src,
                None,
                &self.skills_dir,
                &format!("local:{}", src.file_name().unwrap_or_default().to_string_lossy()),
            )?;
            if installed.is_empty() {
                return Err("No SKILL.md found in zip archive".into());
            }
            for item in &installed {
                self.record_skill(&item.id, &item.source);
            }
            return Ok(installed[0].clone());
        }

        let skill_dir = if src.is_file()
            && src
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("SKILL.md"))
                .unwrap_or(false)
        {
            src.parent().ok_or("Invalid SKILL.md parent")?.to_path_buf()
        } else if src.is_dir() && src.join("SKILL.md").exists() {
            src.to_path_buf()
        } else {
            return Err("Selected folder or zip does not contain a `SKILL.md` file".into());
        };

        let raw_md = fs::read_to_string(skill_dir.join("SKILL.md")).map_err(|e| e.to_string())?;
        let folder_name = skill_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("skill");
        let (name, description, _) = parse_skill_md(&raw_md, folder_name);
        let id = slugify(if name != folder_name { &name } else { folder_name });
        let dest = self.skills_dir.join(&id);
        if dest.exists() {
            let _ = fs::remove_dir_all(&dest);
        }
        copy_dir_recursive(&skill_dir, &dest)?;

        let src_label = format!("local:{}", skill_dir.display());
        self.record_skill(&id, &src_label);

        self.list()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("Installed `{id}` cleanly but failed reloading"))
            .or_else(|_| {
                Ok(SkillInfo {
                    id,
                    name,
                    description,
                    enabled: true,
                    source: src_label,
                    dir_path: dest.to_string_lossy().to_string(),
                    has_scripts: dest.join("scripts").is_dir(),
                    file_count: 1,
                    installed_at: now_sec(),
                })
            })
    }

    /// Downloads & installs one or more skills from GitHub (`owner/repo`, `tree/...` URL, etc.).
    pub async fn install_from_github(
        &self,
        spec_raw: &str,
        mirror_prefix: &str,
    ) -> Result<Vec<SkillInfo>, String> {
        let spec = parse_github_spec(spec_raw)?;
        let client = reqwest::Client::builder()
            .user_agent("Buddy-Skills-Manager/1.0")
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| e.to_string())?;

        let branch = match spec.branch {
            Some(b) => b,
            None => resolve_default_branch(&client, &spec.owner, &spec.repo).await,
        };

        let raw_zip_url = format!(
            "https://github.com/{}/{}/archive/refs/heads/{}.zip",
            spec.owner, spec.repo, branch
        );
        let zip_url = apply_mirror(&raw_zip_url, mirror_prefix);
        let resp = client
            .get(&zip_url)
            .send()
            .await
            .map_err(|e| format!("Failed downloading GitHub archive: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {} downloading {}", resp.status(), raw_zip_url));
        }

        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let tmp_zip = self
            .skills_dir
            .join(format!("gh_{}_{}.zip.tmp", spec.owner, spec.repo));
        fs::write(&tmp_zip, &bytes).map_err(|e| e.to_string())?;

        let source_url = if let Some(ref sub) = spec.subpath {
            format!("https://github.com/{}/{}/tree/{}/{}", spec.owner, spec.repo, branch, sub)
        } else {
            format!("https://github.com/{}/{}", spec.owner, spec.repo)
        };

        let skills_dir_clone = self.skills_dir.clone();
        let sub_clone = spec.subpath.clone();
        let tmp_clone = tmp_zip.clone();
        let res = tokio::task::spawn_blocking(move || {
            install_skill_from_zip(&tmp_clone, sub_clone.as_deref(), &skills_dir_clone, &source_url)
        })
        .await
        .map_err(|e| e.to_string())?;

        let _ = fs::remove_file(&tmp_zip);
        let installed = res?;
        for s in &installed {
            self.record_skill(&s.id, &s.source);
        }
        Ok(installed)
    }
}

async fn resolve_default_branch(client: &reqwest::Client, owner: &str, repo: &str) -> String {
    let api_url = format!("https://api.github.com/repos/{owner}/{repo}");
    if let Ok(resp) = client.get(&api_url).send().await {
        if let Ok(val) = resp.json::<serde_json::Value>().await {
            if let Some(b) = val["default_branch"].as_str() {
                if !b.is_empty() {
                    return b.to_string();
                }
            }
        }
    }
    "main".to_string()
}

fn install_skill_from_zip(
    zip_path: &Path,
    target_subpath: Option<&str>,
    skills_root: &Path,
    source_label: &str,
) -> Result<Vec<SkillInfo>, String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {e}"))?;

    // Determine common top-level strip prefix (GitHub zipballs put `<repo>-<branch>/` at root)
    let mut top_prefixes = Vec::new();
    for i in 0..archive.len() {
        if let Ok(e) = archive.by_index(i) {
            let n = e.name().replace('\\', "/");
            if let Some((first, _)) = n.split_once('/') {
                if top_prefixes.len() < 3 && !top_prefixes.contains(&first.to_string()) {
                    top_prefixes.push(first.to_string());
                }
            } else if !n.is_empty() {
                top_prefixes.clear();
                break;
            }
        }
    }
    let strip_prefix = if top_prefixes.len() == 1 {
        format!("{}/", top_prefixes[0])
    } else {
        String::new()
    };

    // Find every SKILL.md inside the archive
    let mut skill_dirs: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let norm = entry.name().replace('\\', "/");
        let rel = norm.strip_prefix(&strip_prefix).unwrap_or(&norm);
        if rel.eq_ignore_ascii_case("SKILL.md") {
            skill_dirs.push(String::new());
        } else if let Some(dir_part) = rel.strip_suffix("/SKILL.md").or_else(|| rel.strip_suffix("/skill.md")) {
            skill_dirs.push(dir_part.trim_matches('/').to_string());
        }
    }

    if skill_dirs.is_empty() {
        return Err("No `SKILL.md` found anywhere inside archive".into());
    }

    let selected_dirs: Vec<String> = if let Some(sub) = target_subpath {
        let clean_sub = sub.trim_matches('/').to_string();
        if skill_dirs.iter().any(|d| d == &clean_sub) {
            vec![clean_sub]
        } else {
            let matches: Vec<String> = skill_dirs
                .iter()
                .filter(|d| d.starts_with(&format!("{clean_sub}/")))
                .cloned()
                .collect();
            if matches.is_empty() {
                return Err(format!("Subfolder `{clean_sub}` does not contain a `SKILL.md`"));
            }
            matches
        }
    } else if skill_dirs.len() == 1 {
        skill_dirs
    } else if skill_dirs.len() <= 8 {
        skill_dirs
    } else {
        let preview = skill_dirs
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Repository contains {} skills (e.g. {}). Install a specific skill via its subfolder URL (`/tree/<branch>/<skill_dir>`).",
            skill_dirs.len(),
            preview
        ));
    };

    let mut installed = Vec::new();

    for sdir in selected_dirs {
        // Read SKILL.md first to resolve clean id/name
        let md_rel = if sdir.is_empty() {
            "SKILL.md".to_string()
        } else {
            format!("{sdir}/SKILL.md")
        };
        let md_full = format!("{strip_prefix}{md_rel}");
        let mut raw_md = String::new();
        if let Ok(mut f) = archive.by_name(&md_full) {
            let _ = f.read_to_string(&mut raw_md);
        }
        let fallback_name = sdir.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("skill");
        let (name, desc, _) = parse_skill_md(&raw_md, fallback_name);
        let skill_id = slugify(if sdir.is_empty() { &name } else { fallback_name });
        let dest = skills_root.join(&skill_id);
        if dest.exists() {
            let _ = fs::remove_dir_all(&dest);
        }
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

        let prefix_match = if sdir.is_empty() {
            strip_prefix.clone()
        } else {
            format!("{strip_prefix}{sdir}/")
        };

        let mut file_count = 0usize;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            if entry.is_dir() {
                continue;
            }
            let norm = entry.name().replace('\\', "/");
            let Some(rel_in_skill) = norm.strip_prefix(&prefix_match) else {
                continue;
            };
            if rel_in_skill.is_empty() || rel_in_skill.starts_with(".git/") {
                continue;
            }
            let rel_path = Path::new(rel_in_skill);
            if rel_path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
            {
                continue;
            }
            let out_path = dest.join(rel_path);
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let mut out_file = File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
            file_count += 1;
        }

        installed.push(SkillInfo {
            id: skill_id,
            name,
            description: desc,
            enabled: true,
            source: source_label.to_string(),
            dir_path: dest.to_string_lossy().to_string(),
            has_scripts: dest.join("scripts").is_dir(),
            file_count,
            installed_at: now_sec(),
        });
    }

    Ok(installed)
}

/// Searches GitHub repositories or inspects all `SKILL.md` skills inside a specific repo (`owner/repo`).
pub async fn search_github_skills(query_or_repo: &str) -> Result<Vec<GithubSkillCandidate>, String> {
    let q = query_or_repo.trim();
    if q.is_empty() {
        return Err("Search query cannot be empty".into());
    }

    let client = reqwest::Client::builder()
        .user_agent("Buddy-Skills-Manager/1.0")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    // If user passed `owner/repo` or full GitHub URL -> discover every SKILL.md subfolder inside that repo!
    if let Ok(spec) = parse_github_spec(q) {
        let branch = match spec.branch {
            Some(b) => b,
            None => resolve_default_branch(&client, &spec.owner, &spec.repo).await,
        };
        let tree_url = format!(
            "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
            spec.owner, spec.repo, branch
        );
        let resp = client
            .get(&tree_url)
            .send()
            .await
            .map_err(|e| format!("Failed inspecting GitHub repo: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {} reading repository tree", resp.status()));
        }
        let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let mut candidates = Vec::new();
        if let Some(tree) = val["tree"].as_array() {
            for item in tree {
                let p = item["path"].as_str().unwrap_or("");
                if p.eq_ignore_ascii_case("SKILL.md") {
                    candidates.push(GithubSkillCandidate {
                        title: spec.repo.clone(),
                        description: format!("Root skill in {}/{}", spec.owner, spec.repo),
                        install_spec: format!("https://github.com/{}/{}", spec.owner, spec.repo),
                        html_url: format!("https://github.com/{}/{}", spec.owner, spec.repo),
                        stars: 0,
                        is_repo: false,
                    });
                } else if let Some(dir) = p.strip_suffix("/SKILL.md").or_else(|| p.strip_suffix("/skill.md")) {
                    let short = dir.rsplit('/').next().unwrap_or(dir);
                    candidates.push(GithubSkillCandidate {
                        title: short.to_string(),
                        description: format!("{}/{}: {}", spec.owner, spec.repo, dir),
                        install_spec: format!(
                            "https://github.com/{}/{}/tree/{}/{}",
                            spec.owner, spec.repo, branch, dir
                        ),
                        html_url: format!(
                            "https://github.com/{}/{}/tree/{}/{}",
                            spec.owner, spec.repo, branch, dir
                        ),
                        stars: 0,
                        is_repo: false,
                    });
                }
            }
        }
        return Ok(candidates);
    }

    // General GitHub repository search for Agent Skills
    let search_q = format!("{q} (skill OR skills OR SKILL.md)");
    let url = format!(
        "https://api.github.com/search/repositories?q={}&sort=stars&order=desc&per_page=8",
        url_encode(&search_q)
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GitHub search failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub Search HTTP {}", resp.status()));
    }
    let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(items) = val["items"].as_array() {
        for it in items {
            let full_name = it["full_name"].as_str().unwrap_or("").to_string();
            let html_url = it["html_url"].as_str().unwrap_or("").to_string();
            let desc = it["description"]
                .as_str()
                .unwrap_or("GitHub repository")
                .to_string();
            let stars = it["stargazers_count"].as_u64().unwrap_or(0);
            if !full_name.is_empty() {
                out.push(GithubSkillCandidate {
                    title: full_name.clone(),
                    description: desc,
                    install_spec: html_url.clone(),
                    html_url,
                    stars,
                    is_repo: true,
                });
            }
        }
    }
    Ok(out)
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml_frontmatter_skill_md() {
        let md = "---\nname: pdf-helper\ndescription: Convert and split PDFs easily.\n---\n# PDF Helper\nUse script.py.";
        let (name, desc, _) = parse_skill_md(md, "fallback");
        assert_eq!(name, "pdf-helper");
        assert_eq!(desc, "Convert and split PDFs easily.");
    }

    #[test]
    fn parses_github_monorepo_tree_urls() {
        let s1 = parse_github_spec("https://github.com/anthropics/skills/tree/main/skills/docx").unwrap();
        assert_eq!(s1.owner, "anthropics");
        assert_eq!(s1.repo, "skills");
        assert_eq!(s1.branch.as_deref(), Some("main"));
        assert_eq!(s1.subpath.as_deref(), Some("skills/docx"));

        let s2 = parse_github_spec("owner/my-skill").unwrap();
        assert_eq!(s2.owner, "owner");
        assert_eq!(s2.repo, "my-skill");
        assert_eq!(s2.subpath, None);
    }
}
