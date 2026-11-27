//! Skill registry for the production Rust agent.
//!
//! Mirrors the progressive-disclosure pattern used by Claude Code, Codex and
//! OpenCode: the model sees a compact catalogue (id + description) and loads a
//! skill's full body only when it decides the skill is relevant. Two backends:
//!   - vendored file skills: `.agents/skills/<id>/SKILL.md`
//!   - learned DB skills: the `skills` table populated from successful renders
//!     and user corrections.

use crate::services::skills::{self, Skill};
use sqlx::PgPool;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_CATALOG_CHARS: usize = 8_000;
const MAX_SKILL_BODY_CHARS: usize = 60_000;

#[derive(Clone, Debug)]
pub struct SkillDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: Option<PathBuf>,
    pub db_id: Option<Uuid>,
}

fn parse_skill_file(text: &str) -> (String, String, String) {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim();
    if first != "---" {
        let description = text.lines().next().unwrap_or("").trim().to_string();
        return (String::new(), description, text.to_string());
    }
    let mut name = String::new();
    let mut description = String::new();
    let mut body = String::new();
    let mut in_frontmatter = true;
    for line in lines {
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
            continue;
        }
        if in_frontmatter {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("name:") {
                name = value.trim().trim_matches('"').to_string();
            } else if let Some(value) = trimmed.strip_prefix("description:") {
                description = value.trim().trim_matches('"').to_string();
            }
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    (name, description, body)
}

fn walk_skills(dir: &Path, depth: usize, out: &mut Vec<SkillDescriptor>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with('.') || file_name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            let skill_file = path.join("SKILL.md");
            if skill_file.is_file() {
                if let Ok(text) = std::fs::read_to_string(&skill_file) {
                    let (name, description, _) = parse_skill_file(&text);
                    let id = file_name.to_string();
                    out.push(SkillDescriptor {
                        id: id.clone(),
                        name: if name.is_empty() { id } else { name },
                        description,
                        path: Some(skill_file),
                        db_id: None,
                    });
                }
            } else {
                walk_skills(&path, depth + 1, out);
            }
        } else if depth == 0 && path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let (name, description, _) = parse_skill_file(&text);
                let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("skill").to_string();
                out.push(SkillDescriptor {
                    id: id.clone(),
                    name: if name.is_empty() { id } else { name },
                    description,
                    path: Some(path),
                    db_id: None,
                });
            }
        }
    }
}

/// Discover vendored file skills without loading their bodies into context.
pub fn discover_file_skills() -> Vec<SkillDescriptor> {
    let mut out = Vec::new();
    for root in [".agents/skills", ".claude/skills", ".opencode/skills"] {
        let path = Path::new(root);
        if path.exists() {
            walk_skills(path, 0, &mut out);
        }
    }
    let mut seen = HashSet::new();
    out.retain(|s| seen.insert(s.id.clone()));
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn learned_descriptor(skill: &Skill) -> SkillDescriptor {
    SkillDescriptor {
        id: skill.id.to_string(),
        name: skill.name.clone(),
        description: skill.description.clone().unwrap_or_default(),
        path: None,
        db_id: Some(skill.id),
    }
}

/// Compact catalogue for the system prompt (Codex-style context budget).
pub async fn catalog_text(
    pool: &PgPool,
    service_type: Option<&str>,
    user_id: Option<i32>,
) -> String {
    let mut lines = Vec::new();
    for skill in discover_file_skills() {
        lines.push(format!("- {}: {}", skill.id, skill.description));
    }
    if let Ok(learned) = skills::get_relevant_skills(pool, service_type, None, user_id, 20).await {
        for skill in learned {
            let descriptor = learned_descriptor(&skill);
            lines.push(format!("- learned:{}: {}", descriptor.id, descriptor.description));
        }
    }
    let mut out = String::from("## AVAILABLE SKILLS\nCall load_skill(id) to read the full instructions. Only load what you need.\n");
    for line in lines {
        if out.len() + line.len() + 1 > MAX_CATALOG_CHARS {
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn render_learned_skill(skill: &Skill) -> String {
    let mut out = format!(
        "# {}\n\n{}\n\n- source: {}\n- scope: {}\n- service_type: {}\n- times used: {}\n\n## Trigger conditions\n{}\n\n## Tool sequence that worked\n{}\n",
        skill.name,
        skill.description.clone().unwrap_or_default(),
        skill.source,
        skill.scope,
        skill.service_type.clone().unwrap_or_else(|| "any".to_string()),
        skill.success_count,
        serde_json::to_string_pretty(&skill.trigger_conditions).unwrap_or_default(),
        serde_json::to_string_pretty(&skill.tool_sequence).unwrap_or_default(),
    );
    if out.len() > MAX_SKILL_BODY_CHARS {
        out.truncate(MAX_SKILL_BODY_CHARS);
    }
    out
}

/// Load the full body of a skill by id. File skills return SKILL.md (without
/// frontmatter) plus a sample of supporting paths; learned skills are rendered
/// from PostgreSQL into the same shape.
pub async fn load_skill(id: &str, pool: &PgPool) -> Result<String, String> {
    if let Ok(uuid) = Uuid::parse_str(id) {
        if let Some(skill) = skills::get_skill_by_id(pool, uuid).await? {
            return Ok(render_learned_skill(&skill));
        }
        return Err(format!("learned skill {id} not found"));
    }

    let descriptor = discover_file_skills()
        .into_iter()
        .find(|skill| skill.id == id)
        .ok_or_else(|| format!("unknown skill '{id}' — call list_skills for valid ids"))?;
    let path = descriptor
        .path
        .ok_or_else(|| format!("skill '{id}' has no file path"))?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read skill '{id}': {e}"))?;
    let (_, _, body) = parse_skill_file(&text);
    let mut out = body;
    if let Some(dir) = path.parent() {
        let mut supporting = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
                    supporting.push(p);
                }
            }
        }
        supporting.sort();
        if !supporting.is_empty() {
            out.push_str("\n\n## Supporting files\n");
            for path in supporting.into_iter().take(10) {
                out.push_str(&format!("- {}\n", path.display()));
            }
        }
    }
    if out.len() > MAX_SKILL_BODY_CHARS {
        out.truncate(MAX_SKILL_BODY_CHARS);
    }
    Ok(out)
}

/// Rank learned skills for the current brief. PostgreSQL supplies scope
/// filtering; when Voyage + Qdrant are configured, semantic similarity narrows
/// the set further. Falls back to textual matching otherwise.
pub async fn search_learned_skills(
    pool: &PgPool,
    qdrant: Option<&crate::qdrant_client::QdrantClient>,
    service_type: Option<&str>,
    user_id: Option<i32>,
    query: &str,
    limit: usize,
) -> Result<Vec<Skill>, String> {
    let mut skills_list =
        skills::get_relevant_skills(pool, service_type, None, user_id, limit.max(10)).await?;

    let lowered = query.to_ascii_lowercase();
    if let Ok(key) = std::env::var("VOYAGEAI_API_KEY") {
        if let Some(q) = qdrant {
            let voyage = crate::voyage_embeddings::VoyageEmbeddings::new(key);
            let session = user_id.map(|u| u.to_string()).unwrap_or_default();
            if let Ok(documents) = q
                .search_similar_conversations_with_voyage(query, &session, limit as u32, &voyage)
                .await
            {
                let ids: HashSet<String> = documents
                    .into_iter()
                    .filter(|doc| {
                        doc.context
                            .get("feature")
                            .and_then(|v| v.as_str())
                            .map(|v| v == "skill")
                            .unwrap_or(false)
                    })
                    .map(|doc| doc.id)
                    .collect();
                if !ids.is_empty() {
                    skills_list.retain(|skill| ids.contains(&skill.id.to_string()));
                }
            }
        }
    }
    if !lowered.is_empty() {
        skills_list.retain(|skill| {
            skill.name.to_ascii_lowercase().contains(&lowered)
                || skill
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&lowered)
        });
    }
    skills_list.truncate(limit);
    Ok(skills_list)
}
