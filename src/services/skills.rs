use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: Uuid,
    pub user_id: Option<i32>,
    pub service_type: Option<String>,
    pub campaign_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub trigger_conditions: serde_json::Value,
    pub tool_sequence: serde_json::Value,
    pub source: String,
    pub correction: Option<serde_json::Value>,
    pub success_count: i32,
    pub scope: String,
    pub restricted_to_user_id: Option<i32>,
    pub qdrant_point_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Store a detected skill in PostgreSQL (and optionally Qdrant).
pub async fn store_skill(
    pool: &sqlx::PgPool,
    qdrant: Option<&crate::qdrant_client::QdrantClient>,
    gemini: Option<&crate::gemini_client::GeminiClient>,
    user_id: Option<i32>,
    service_type: Option<&str>,
    campaign_id: Option<Uuid>,
    name: &str,
    description: &str,
    trigger_conditions: serde_json::Value,
    tool_sequence: serde_json::Value,
    source: &str,
    correction: Option<serde_json::Value>,
    scope: &str,
) -> Result<Uuid, String> {
    let id = Uuid::new_v4();

    let qdrant_point_id = if let (Some(q), Some(g)) = (qdrant, gemini) {
        let embed_text = format!("{}: {}", name, description);
        match q
            .store_chat_memory_with_gemini2(
                &id.to_string(),
                user_id.map(|u| u.to_string()).as_deref(),
                &embed_text,
                &serde_json::to_string(&tool_sequence).unwrap_or_default(),
                vec![],
                {
                    let mut ctx = HashMap::new();
                    if let Some(st) = service_type {
                        ctx.insert("service_type".to_string(), json!(st));
                    }
                    if let Some(cid) = campaign_id {
                        ctx.insert("campaign_id".to_string(), json!(cid.to_string()));
                    }
                    ctx.insert("source".to_string(), json!(source));
                    ctx
                },
                g,
                Some("skill"),
            )
            .await
        {
            Ok(pid) => Some(pid),
            Err(e) => {
                tracing::warn!("Failed to store skill embedding: {}", e);
                None
            }
        }
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO skills (id, user_id, service_type, campaign_id, name, description, \
                trigger_conditions, tool_sequence, source, correction, scope, qdrant_point_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(user_id)
    .bind(service_type)
    .bind(campaign_id)
    .bind(name)
    .bind(description)
    .bind(&trigger_conditions)
    .bind(&tool_sequence)
    .bind(source)
    .bind(&correction)
    .bind(scope)
    .bind(&qdrant_point_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to insert skill: {}", e))?;

    tracing::info!("Stored skill '{}' (source={}, id={})", name, source, id);
    Ok(id)
}

/// Query relevant skills for a given campaign + service_type.
/// Returns skills scoped to the campaign, service, or global.
pub async fn get_relevant_skills(
    pool: &sqlx::PgPool,
    service_type: Option<&str>,
    campaign_id: Option<Uuid>,
    user_id: Option<i32>,
    limit: usize,
) -> Result<Vec<Skill>, String> {
    let rows = sqlx::query_as::<_, (
        Uuid, Option<i32>, Option<String>, Option<Uuid>, String, Option<String>,
        serde_json::Value, serde_json::Value, String, Option<serde_json::Value>,
        i32, String, Option<i32>, Option<String>,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>,
    )>(
        "SELECT id, user_id, service_type, campaign_id, name, description, \
                trigger_conditions, tool_sequence, source, correction, \
                success_count, scope, restricted_to_user_id, qdrant_point_id, \
                created_at, updated_at \
         FROM skills \
         WHERE scope = 'global' \
            OR (scope = 'service' AND ($1::text IS NULL OR service_type = $1)) \
            OR (scope = 'campaign' AND ($2::uuid IS NULL OR campaign_id = $2)) \
         ORDER BY success_count DESC, created_at DESC",
    )
    .bind(service_type)
    .bind(campaign_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to query skills: {}", e))?;

    let skills: Vec<Skill> = rows
        .into_iter()
        .map(|(
            id, uid, st, cid, name, desc, tc, ts, src, corr, count, scope, rest, qpid, ca, ua,
        )| Skill {
            id,
            user_id: uid,
            service_type: st,
            campaign_id: cid,
            name,
            description: desc,
            trigger_conditions: tc,
            tool_sequence: ts,
            source: src,
            correction: corr,
            success_count: count,
            scope,
            restricted_to_user_id: rest,
            qdrant_point_id: qpid,
            created_at: ca,
            updated_at: ua,
        })
        .filter(|s| {
            if let Some(ref uid) = user_id {
                if let Some(rest) = s.restricted_to_user_id {
                    if rest != *uid {
                        return false;
                    }
                }
            }
            true
        })
        .take(limit)
        .collect();

    Ok(skills)
}

/// Format skills into an injectable context string for the agent prompt.
pub fn format_skills_context(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from("## LESSONS LEARNED\n");
    out.push_str("The following patterns have been learned from past work and user corrections. Apply them unless the user explicitly overrides:\n\n");
    for s in skills {
        out.push_str(&format!("- **{}**", s.name));
        if let Some(ref desc) = s.description {
            out.push_str(&format!(": {}", desc));
        }
        out.push_str(&format!(" (source: {}, used {} times)\n", s.source, s.success_count));
    }
    out
}

/// Given a user message and agent response, determine if this was a correction.
/// If so, extract the correction pattern and create a skill.
/// Takes owned values for use with tokio::spawn.
pub async fn detect_and_store_correction(
    pool: sqlx::PgPool,
    qdrant: Option<crate::qdrant_client::QdrantClient>,
    gemini: std::sync::Arc<crate::gemini_client::GeminiClient>,
    user_id: Option<i32>,
    service_type: Option<String>,
    campaign_id: Option<Uuid>,
    user_message: String,
    agent_response: String,
) {
    let correction_keywords = [
        "don't", "dont", "stop", "instead", "wrong", "no,", "no ", "not that",
        "change", "different", "actually", "rather", "fix", "bad", "try again",
        "revert", "undo", "that's not", "thats not", "i meant", "i mean",
    ];
    let lower = user_message.to_lowercase();
    let has_correction_signal = correction_keywords.iter().any(|kw| lower.contains(kw));

    if !has_correction_signal {
        return;
    }

    let prompt = format!(
        "You are a skill extraction system. Given a user message and the agent's response, \
         determine if the user is correcting the agent's behavior or output.\n\n\
         If this IS a correction, extract:\n\
         1. skill_name: A short, specific name for the lesson (e.g., \"Use fast intro for tutorials\")\n\
         2. description: What the agent should do differently next time\n\
         3. trigger_conditions: When this skill applies (e.g., {{\"brief_contains\": \"tutorial\"}})\n\n\
         If this is NOT a correction, respond with {{\"is_correction\": false}}\n\n\
         User message: {user}\n\
         Agent response: {agent}\n\n\
         JSON response:",
        user = user_message,
        agent = agent_response,
    );

    match gemini.generate_text(prompt.as_str()).await {
        Ok(response) => {
            let trimmed = response.trim();
            if trimmed.starts_with('{') {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if parsed.get("is_correction").and_then(|v| v.as_bool()).unwrap_or(true) {
                        let skill_name = parsed
                            .get("skill_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Learned pattern")
                            .to_string();
                        let description = parsed
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let trigger_conditions = parsed
                            .get("trigger_conditions")
                            .cloned()
                            .unwrap_or(json!({}));

                        let correction_data = json!({
                            "user_message": user_message,
                            "agent_response": agent_response,
                        });

                        match store_skill(
                            &pool,
                            qdrant.as_ref(),
                            Some(gemini.as_ref()),
                            user_id,
                            service_type.as_deref(),
                            campaign_id,
                            &skill_name,
                            &description,
                            trigger_conditions,
                            json!([]),
                            "user_correction",
                            Some(correction_data),
                            if campaign_id.is_some() { "campaign" } else { "service" },
                        )
                        .await
                        {
                            Ok(id) => tracing::info!("Created skill '{}' from correction (id={})", skill_name, id),
                            Err(e) => tracing::warn!("Failed to store correction skill: {}", e),
                        }
                    }
                }
            }
        }
        Err(e) => tracing::warn!("Correction detection LLM call failed: {}", e),
    }
}
