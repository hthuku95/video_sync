// Content Management Agent — Gemini function-calling agent for post-publish YouTube management.
//
// Allows users to instruct the agent in natural language to:
//   - Fetch published clips
//   - Update video metadata
//   - Delete videos (with human confirmation)
//   - Re-post clips under new metadata
//
// Architecture mirrors GeminiClippingAgent (see clipping_agent.rs) but uses
// FunctionCallingMode::Auto so Gemini can ask clarifying questions.

use crate::gemini_client::{FunctionDeclaration, Parameters, PropertyDefinition};
use crate::agent::simple_gemini_agent::{GeminiToolExecutor, SimpleGeminiAgent};
use crate::AppState;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

const CONFIRMATION_POLL_INTERVAL_MS: u64 = 2_000;
const CONFIRMATION_TIMEOUT_SECS: i64 = 300; // 5 minutes

#[derive(Clone)]
pub struct ContentManagementAgent {
    app_state: Arc<AppState>,
}

impl ContentManagementAgent {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Entry point: run the agent for a given session.
    ///
    /// Spawned in a tokio background task from the handler.
    pub async fn run(
        session_id: i32,
        instruction: &str,
        destination_channel_id: i32,
        app_state: Arc<AppState>,
    ) {
        let agent = Self::new(app_state);
        match agent
            .execute(session_id, instruction, destination_channel_id)
            .await
        {
            Ok(summary) => {
                let _ = sqlx::query(
                    "UPDATE content_management_sessions
                     SET status = 'completed', result_summary = $1, updated_at = NOW()
                     WHERE id = $2",
                )
                .bind(&summary)
                .bind(session_id)
                .execute(&agent.app_state.db_pool)
                .await;
            }
            Err(e) => {
                tracing::error!(
                    "ContentManagementAgent session {} failed: {}",
                    session_id,
                    e
                );
                let _ = sqlx::query(
                    "UPDATE content_management_sessions
                     SET status = 'failed', result_summary = $1, updated_at = NOW()
                     WHERE id = $2",
                )
                .bind(format!("Agent error: {}", e))
                .bind(session_id)
                .execute(&agent.app_state.db_pool)
                .await;
            }
        }
    }

    async fn execute(
        &self,
        session_id: i32,
        instruction: &str,
        destination_channel_id: i32,
    ) -> Result<String, String> {
        let gemini = self
            .app_state
            .video_gemini_client
            .as_ref()
            .cloned()
            .or(self.app_state.gemini_client.as_ref().cloned())
            .ok_or("Gemini client not configured")?;

        let system_instruction = "You are a YouTube content management assistant. You help users manage published YouTube Shorts clips using the same canonical VideoSync Gemini agent runtime as the rest of the system. You can fetch published clips, inspect metadata, update metadata, delete videos with human confirmation, and repost clips. Always call get_published_clips first to understand what exists before making changes. Before ANY destructive action, you MUST call request_confirmation and wait for a granted response. When you are finished, respond with a concise plain-language summary of exactly what you changed or what blocked the action.";

        let domain_agent = self.clone();
        let tool_executor: GeminiToolExecutor = Arc::new(move |name, args, _ctx| {
            let domain_agent = domain_agent.clone();
            let name = name.to_string();
            let args = args.clone();
            Box::pin(async move {
                match domain_agent
                    .dispatch_tool(&name, &args, session_id, destination_channel_id)
                    .await
                {
                    Ok(value) => value,
                    Err(error) => json!({ "error": error }),
                }
            }) as Pin<Box<dyn std::future::Future<Output = Value> + Send>>
        });

        let runtime = SimpleGeminiAgent::new(Arc::new(gemini));
        runtime
            .execute_with_custom_tools(
                instruction,
                &format!("content-management:{session_id}"),
                None,
                self.app_state.clone(),
                None,
                system_instruction,
                self.build_tool_declarations(),
                None,
                tool_executor,
                None,
            )
            .await
    }

    fn build_tool_declarations(&self) -> Vec<FunctionDeclaration> {
        vec![
            FunctionDeclaration {
                name: "get_published_clips".to_string(),
                description: "Fetch published clips from the database. Returns clip IDs, YouTube \
                              video IDs, titles, and URLs."
                    .to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert(
                            "limit".to_string(),
                            PropertyDefinition {
                                prop_type: "integer".to_string(),
                                description: "Max number of clips to return (default 20)"
                                    .to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "status_filter".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Filter by upload_status, e.g. 'published'"
                                    .to_string(),
                                items: None,
                            },
                        );
                        p
                    },
                    required: vec![],
                },
            },
            FunctionDeclaration {
                name: "get_youtube_video_metadata".to_string(),
                description: "Fetch current title, description, tags, and privacy status of a \
                              YouTube video directly from the YouTube API."
                    .to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert(
                            "youtube_video_id".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "YouTube video ID (e.g. dQw4w9WgXcQ)".to_string(),
                                items: None,
                            },
                        );
                        p
                    },
                    required: vec!["youtube_video_id".to_string()],
                },
            },
            FunctionDeclaration {
                name: "update_video_metadata".to_string(),
                description: "Update the title, description, or tags of a published YouTube video."
                    .to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert(
                            "youtube_video_id".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "YouTube video ID".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "title".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "New title (optional)".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "description".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "New description (optional)".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "clip_db_id".to_string(),
                            PropertyDefinition {
                                prop_type: "integer".to_string(),
                                description: "Database ID of the extracted_clips row".to_string(),
                                items: None,
                            },
                        );
                        p
                    },
                    required: vec!["youtube_video_id".to_string(), "clip_db_id".to_string()],
                },
            },
            FunctionDeclaration {
                name: "delete_video".to_string(),
                description:
                    "Delete a YouTube video. Sets it to private first, then hard-deletes. \
                              REQUIRES confirmed=true (call request_confirmation first)."
                        .to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert(
                            "youtube_video_id".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "YouTube video ID".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "clip_db_id".to_string(),
                            PropertyDefinition {
                                prop_type: "integer".to_string(),
                                description: "Database ID of the clip row".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "reason".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Reason for deletion (for audit log)".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "confirmed".to_string(),
                            PropertyDefinition {
                                prop_type: "boolean".to_string(),
                                description: "Must be true — obtained via request_confirmation"
                                    .to_string(),
                                items: None,
                            },
                        );
                        p
                    },
                    required: vec![
                        "youtube_video_id".to_string(),
                        "clip_db_id".to_string(),
                        "reason".to_string(),
                        "confirmed".to_string(),
                    ],
                },
            },
            FunctionDeclaration {
                name: "repost_clip".to_string(),
                description: "Re-upload an existing extracted clip file to YouTube (optionally \
                              with a new title/description). REQUIRES confirmed=true."
                    .to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert(
                            "clip_db_id".to_string(),
                            PropertyDefinition {
                                prop_type: "integer".to_string(),
                                description: "Database ID of the extracted_clips row".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "new_title".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "New title for the repost (optional)".to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "new_description".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "New description for the repost (optional)"
                                    .to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "confirmed".to_string(),
                            PropertyDefinition {
                                prop_type: "boolean".to_string(),
                                description: "Must be true — obtained via request_confirmation"
                                    .to_string(),
                                items: None,
                            },
                        );
                        p
                    },
                    required: vec!["clip_db_id".to_string(), "confirmed".to_string()],
                },
            },
            FunctionDeclaration {
                name: "request_confirmation".to_string(),
                description: "Pause the agent and wait for the human to confirm a destructive \
                              action. Returns {granted: true} when confirmed or {granted: false} \
                              on timeout/cancel."
                    .to_string(),
                parameters: Parameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut p = HashMap::new();
                        p.insert(
                            "action_summary".to_string(),
                            PropertyDefinition {
                                prop_type: "string".to_string(),
                                description: "Human-readable summary of the action to confirm"
                                    .to_string(),
                                items: None,
                            },
                        );
                        p.insert(
                            "destructive".to_string(),
                            PropertyDefinition {
                                prop_type: "boolean".to_string(),
                                description: "True if the action is irreversible".to_string(),
                                items: None,
                            },
                        );
                        p
                    },
                    required: vec!["action_summary".to_string()],
                },
            },
        ]
    }

    async fn dispatch_tool(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
        session_id: i32,
        destination_channel_id: i32,
    ) -> Result<Value, String> {
        match name {
            "get_published_clips" => {
                self.tool_get_published_clips(args, destination_channel_id)
                    .await
            }
            "get_youtube_video_metadata" => self.tool_get_youtube_video_metadata(args).await,
            "update_video_metadata" => self.tool_update_video_metadata(args, session_id).await,
            "delete_video" => self.tool_delete_video(args, session_id).await,
            "repost_clip" => self.tool_repost_clip(args, session_id).await,
            "request_confirmation" => self.tool_request_confirmation(args, session_id).await,
            _ => Err(format!("Unknown tool: {}", name)),
        }
    }

    // ─── Tool Implementations ────────────────────────────────────────────────

    async fn tool_get_published_clips(
        &self,
        args: &HashMap<String, Value>,
        destination_channel_id: i32,
    ) -> Result<Value, String> {
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as i32;

        let status_filter = args
            .get("status_filter")
            .and_then(|v| v.as_str())
            .unwrap_or("published")
            .to_string();

        let rows = sqlx::query(
            "SELECT ec.id, ec.youtube_video_id, ec.ai_title, ec.youtube_url,
                    ec.upload_status, ec.created_at
             FROM extracted_clips ec
             JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
             JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
             WHERE ycl.destination_channel_id = $1
               AND ec.upload_status = $2
             ORDER BY ec.created_at DESC
             LIMIT $3",
        )
        .bind(destination_channel_id)
        .bind(&status_filter)
        .bind(limit)
        .fetch_all(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB query failed: {}", e))?;

        use sqlx::Row;
        let clips: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "clip_id": r.try_get::<i32, _>("id").unwrap_or(0),
                    "youtube_video_id": r.try_get::<Option<String>, _>("youtube_video_id").ok().flatten(),
                    "ai_title": r.try_get::<Option<String>, _>("ai_title").ok().flatten(),
                    "youtube_url": r.try_get::<Option<String>, _>("youtube_url").ok().flatten(),
                    "upload_status": r.try_get::<String, _>("upload_status").unwrap_or_default(),
                    "created_at": r.try_get::<chrono::DateTime<Utc>, _>("created_at").ok(),
                })
            })
            .collect();

        Ok(json!({ "clips": clips, "count": clips.len() }))
    }

    async fn tool_get_youtube_video_metadata(
        &self,
        args: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let video_id = args
            .get("youtube_video_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing youtube_video_id")?;

        let youtube = self
            .app_state
            .youtube_client
            .as_ref()
            .ok_or("YouTube client not configured")?;

        // Get access token from destination channel (latest connected channel)
        let token = self.get_any_access_token().await?;

        let metadata = youtube
            .get_video_metadata(&token, video_id)
            .await
            .map_err(|e| format!("YouTube API error: {}", e))?;

        Ok(json!({
            "title": metadata["snippet"]["title"],
            "description": metadata["snippet"]["description"],
            "tags": metadata["snippet"]["tags"],
            "privacy_status": metadata["status"]["privacyStatus"],
        }))
    }

    async fn tool_update_video_metadata(
        &self,
        args: &HashMap<String, Value>,
        session_id: i32,
    ) -> Result<Value, String> {
        let video_id = args
            .get("youtube_video_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing youtube_video_id")?;

        let clip_db_id = args
            .get("clip_db_id")
            .and_then(|v| v.as_i64())
            .ok_or("Missing clip_db_id")? as i32;

        let title = args.get("title").and_then(|v| v.as_str());
        let description = args.get("description").and_then(|v| v.as_str());

        let youtube = self
            .app_state
            .youtube_client
            .as_ref()
            .ok_or("YouTube client not configured")?;

        let token = self.get_any_access_token().await?;

        youtube
            .update_video_metadata(&token, video_id, title, description, None)
            .await
            .map_err(|e| format!("YouTube update failed: {}", e))?;

        // Audit log
        self.write_audit_log(
            session_id,
            "update_metadata",
            Some(video_id),
            Some(clip_db_id),
            json!({ "title": title, "description": description }),
        )
        .await;

        // Update proposed_title/description in DB
        if let Some(t) = title {
            let _ = sqlx::query(
                "UPDATE extracted_clips SET ai_title = $1, updated_at = NOW() WHERE id = $2",
            )
            .bind(t)
            .bind(clip_db_id)
            .execute(&self.app_state.db_pool)
            .await;
        }

        Ok(json!({ "success": true, "message": "Metadata updated" }))
    }

    async fn tool_delete_video(
        &self,
        args: &HashMap<String, Value>,
        session_id: i32,
    ) -> Result<Value, String> {
        let confirmed = args
            .get("confirmed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !confirmed {
            return Ok(json!({
                "requires_confirmation": true,
                "action_summary": format!(
                    "Delete YouTube video {}",
                    args.get("youtube_video_id").and_then(|v| v.as_str()).unwrap_or("?")
                )
            }));
        }

        let video_id = args
            .get("youtube_video_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing youtube_video_id")?;

        let clip_db_id = args
            .get("clip_db_id")
            .and_then(|v| v.as_i64())
            .ok_or("Missing clip_db_id")? as i32;

        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("No reason provided");

        let youtube = self
            .app_state
            .youtube_client
            .as_ref()
            .ok_or("YouTube client not configured")?;

        let token = self.get_any_access_token().await?;

        // Soft-delete first (set to private)
        if let Err(e) = youtube.set_video_private(&token, video_id).await {
            tracing::warn!(
                "session {}: Failed to set video {} to private before delete: {}",
                session_id,
                video_id,
                e
            );
        }

        // Audit log (before hard delete in case API fails)
        self.write_audit_log(
            session_id,
            "delete_video",
            Some(video_id),
            Some(clip_db_id),
            json!({ "reason": reason }),
        )
        .await;

        // Hard delete
        youtube
            .delete_video(&token, video_id)
            .await
            .map_err(|e| format!("YouTube delete failed: {}", e))?;

        // Mark clip as deleted in DB
        let _ = sqlx::query(
            "UPDATE extracted_clips
             SET upload_status = 'deleted', youtube_video_id = NULL, youtube_url = NULL,
                 updated_at = NOW()
             WHERE id = $1",
        )
        .bind(clip_db_id)
        .execute(&self.app_state.db_pool)
        .await;

        Ok(json!({ "success": true, "message": format!("Video {} deleted", video_id) }))
    }

    async fn tool_repost_clip(
        &self,
        args: &HashMap<String, Value>,
        session_id: i32,
    ) -> Result<Value, String> {
        let confirmed = args
            .get("confirmed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !confirmed {
            return Ok(json!({
                "requires_confirmation": true,
                "action_summary": format!(
                    "Repost clip DB id {}",
                    args.get("clip_db_id").and_then(|v| v.as_i64()).unwrap_or(0)
                )
            }));
        }

        let clip_db_id = args
            .get("clip_db_id")
            .and_then(|v| v.as_i64())
            .ok_or("Missing clip_db_id")? as i32;

        let new_title = args.get("new_title").and_then(|v| v.as_str());
        let new_description = args.get("new_description").and_then(|v| v.as_str());

        // Fetch clip from DB
        let row = sqlx::query(
            "SELECT ec.local_clip_path, ec.ai_title, ec.ai_description, ec.ai_tags,
                    ec.clip_number, ec.custom_thumbnail_path,
                    ycl.destination_channel_id
             FROM extracted_clips ec
             JOIN clipping_jobs cj ON ec.clipping_job_id = cj.id
             JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
             WHERE ec.id = $1",
        )
        .bind(clip_db_id)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or(format!("Clip {} not found", clip_db_id))?;

        use sqlx::Row;
        let local_path: String = row.try_get("local_clip_path").unwrap_or_default();
        let ai_title: Option<String> = row.try_get("ai_title").unwrap_or(None);
        let ai_description: Option<String> = row.try_get("ai_description").unwrap_or(None);
        let ai_tags: Option<serde_json::Value> = row.try_get("ai_tags").unwrap_or(None);
        let clip_number: i32 = row.try_get("clip_number").unwrap_or(0);
        let custom_thumbnail_path: Option<String> =
            row.try_get("custom_thumbnail_path").unwrap_or(None);
        let dest_channel_id: i32 = row.try_get("destination_channel_id").unwrap_or(0);

        let title = new_title
            .map(|s| s.to_string())
            .or(ai_title)
            .unwrap_or_else(|| format!("Clip {}", clip_number));
        let description = new_description
            .map(|s| s.to_string())
            .or(ai_description)
            .unwrap_or_default();
        let tags: Vec<String> = ai_tags
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let dest_channel = sqlx::query_as::<_, crate::models::youtube::ConnectedYouTubeChannel>(
            "SELECT * FROM connected_youtube_channels WHERE id = $1",
        )
        .bind(dest_channel_id)
        .fetch_optional(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB error fetching channel: {}", e))?
        .ok_or("Destination channel not found")?;

        let youtube = self
            .app_state
            .youtube_client
            .as_ref()
            .ok_or("YouTube client not configured")?;

        let oauth_client_id = self
            .app_state
            .google_oauth_client_id
            .as_ref()
            .ok_or("Google OAuth client ID not configured")?;

        let oauth_client_secret = self
            .app_state
            .google_oauth_client_secret
            .as_ref()
            .ok_or("Google OAuth client secret not configured")?;

        let uploader = crate::clipping::uploader::ClipUploader::new(
            std::sync::Arc::new(youtube.clone()),
            self.app_state.db_pool.clone(),
            oauth_client_id.clone(),
            oauth_client_secret.clone(),
        );

        let clip_data = crate::clipping::ai_clipper::ExtractedClipData {
            clip_number,
            local_clip_path: local_path,
            start_time_seconds: 0.0,
            end_time_seconds: 0.0,
            duration_seconds: 0.0,
            ai_title: title,
            ai_description: description,
            ai_tags: tags,
            ai_confidence_score: 1.0,
            viral_factors: vec![],
            custom_thumbnail_path: custom_thumbnail_path.clone(),
            thumbnail_generation_method: custom_thumbnail_path
                .as_ref()
                .map(|_| "manual".to_string()),
            enhancement_applied: false,
            enhancement_tools: Vec::new(),
            enhancement_reasoning: None,
            r2_clip_key: None,
            r2_thumb_key: None,
            r2_clip_url: None,
            qa_status: None,
            qa_score: None,
            qa_feedback: None,
            qa_retry_hint: None,
        };

        match uploader
            .upload_clip(&clip_data, clip_db_id, &dest_channel, false)
            .await
        {
            Ok(result) => {
                self.write_audit_log(
                    session_id,
                    "repost_clip",
                    Some(&result.video_id),
                    Some(clip_db_id),
                    json!({ "new_title": new_title }),
                )
                .await;

                Ok(json!({
                    "success": true,
                    "youtube_video_id": result.video_id,
                    "youtube_url": result.url,
                }))
            }
            Err(e) => Err(format!("Repost failed: {}", e)),
        }
    }

    async fn tool_request_confirmation(
        &self,
        args: &HashMap<String, Value>,
        session_id: i32,
    ) -> Result<Value, String> {
        let action_summary = args
            .get("action_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("Confirm action")
            .to_string();

        let destructive = args
            .get("destructive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Set session to awaiting_confirmation
        sqlx::query(
            "UPDATE content_management_sessions
             SET status = 'awaiting_confirmation',
                 confirmation_required = $1,
                 confirmation_granted = false,
                 updated_at = NOW()
             WHERE id = $2",
        )
        .bind(json!({ "action_summary": action_summary, "destructive": destructive }))
        .bind(session_id)
        .execute(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB update failed: {}", e))?;

        tracing::info!(
            "session {}: awaiting human confirmation for: {}",
            session_id,
            action_summary
        );

        // Poll for confirmation_granted
        let deadline = Utc::now() + chrono::Duration::seconds(CONFIRMATION_TIMEOUT_SECS);

        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(
                CONFIRMATION_POLL_INTERVAL_MS,
            ))
            .await;

            let row = sqlx::query(
                "SELECT confirmation_granted, status FROM content_management_sessions WHERE id = $1",
            )
            .bind(session_id)
            .fetch_optional(&self.app_state.db_pool)
            .await
            .map_err(|e| format!("DB poll failed: {}", e))?;

            if let Some(r) = row {
                use sqlx::Row;
                let granted: bool = r.try_get("confirmation_granted").unwrap_or(false);
                let status: String = r.try_get("status").unwrap_or_default();

                if granted {
                    // Reset confirmation state
                    let _ = sqlx::query(
                        "UPDATE content_management_sessions
                         SET status = 'running', confirmation_required = NULL,
                             confirmation_granted = false, updated_at = NOW()
                         WHERE id = $1",
                    )
                    .bind(session_id)
                    .execute(&self.app_state.db_pool)
                    .await;

                    return Ok(json!({ "granted": true }));
                }

                if status == "failed" {
                    return Ok(json!({ "granted": false, "reason": "cancelled_by_user" }));
                }
            }

            if Utc::now() > deadline {
                tracing::warn!("session {}: confirmation timed out", session_id);
                return Ok(json!({ "granted": false, "reason": "timeout" }));
            }
        }
    }

    // ─── Helpers ────────────────────────────────────────────────────────────

    /// Fetch a valid access token from any connected YouTube channel.
    async fn get_any_access_token(&self) -> Result<String, String> {
        let row: Option<(String,)> = sqlx::query_as::<_, (String,)>(
            "SELECT access_token FROM connected_youtube_channels
             WHERE token_expiry > NOW() + INTERVAL '5 minutes'
             LIMIT 1",
        )
        .fetch_optional(&self.app_state.db_pool)
        .await
        .map_err(|e| format!("DB error fetching token: {}", e))?;

        row.map(|(t,)| t)
            .ok_or_else(|| "No valid YouTube access token found".to_string())
    }

    async fn write_audit_log(
        &self,
        session_id: i32,
        action: &str,
        youtube_video_id: Option<&str>,
        clip_db_id: Option<i32>,
        details: Value,
    ) {
        // Look up user_id from session
        let user_id: Option<i32> =
            sqlx::query_scalar("SELECT user_id FROM content_management_sessions WHERE id = $1")
                .bind(session_id)
                .fetch_optional(&self.app_state.db_pool)
                .await
                .ok()
                .flatten();

        let _ = sqlx::query(
            "INSERT INTO content_management_audit_log
                 (session_id, user_id, action, youtube_video_id, clip_db_id, details)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(action)
        .bind(youtube_video_id)
        .bind(clip_db_id)
        .bind(details)
        .execute(&self.app_state.db_pool)
        .await;
    }
}
