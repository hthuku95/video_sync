use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct ArtifactVerificationResult {
    pub verified: bool,
    pub details: serde_json::Value,
}

pub struct ArtifactVerifier;

impl ArtifactVerifier {
    pub async fn verify_links(pool: &PgPool, links: &[String]) -> ArtifactVerificationResult {
        let mut verified_links = Vec::new();

        for link in links {
            verified_links.push(Self::verify_link(pool, link).await);
        }

        let verified = !verified_links.is_empty() && verified_links.iter().all(|item| {
            item.get("exists")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        });

        ArtifactVerificationResult {
            verified,
            details: serde_json::json!({
                "verified": verified,
                "links": verified_links,
            }),
        }
    }

    async fn verify_link(pool: &PgPool, link: &str) -> serde_json::Value {
        if let Some(file_id) = link.strip_prefix("/api/outputs/stream/") {
            return Self::verify_output_file(pool, link, file_id).await;
        }

        if let Some(file_id) = link.strip_prefix("/api/outputs/download/") {
            return Self::verify_output_file(pool, link, file_id).await;
        }

        if let Some(delivery_id) = link.strip_prefix("/delivery/") {
            return Self::verify_delivery(pool, link, delivery_id).await;
        }

        serde_json::json!({
            "link": link,
            "kind": "unknown",
            "exists": false,
            "reason": "Unsupported artifact link format",
        })
    }

    async fn verify_output_file(pool: &PgPool, link: &str, file_id: &str) -> serde_json::Value {
        if let Ok(Some(artifact)) =
            crate::services::GeneratedArtifactService::find_by_legacy_file_id(pool, file_id).await
        {
            let local_file = artifact
                .file_path
                .as_deref()
                .map(std::path::Path::new)
                .map(|path| path.exists())
                .unwrap_or(false);

            let manifest_exists = artifact.bytes.unwrap_or(0) > 0
                || artifact.public_url.as_deref().map(|url| !url.is_empty()).unwrap_or(false)
                || artifact.preview_url.as_deref().map(|url| !url.is_empty()).unwrap_or(false);

            let result = serde_json::json!({
                "link": link,
                "kind": "output_file",
                "exists": manifest_exists || local_file,
                "reason": if manifest_exists { "artifact_manifest" } else if local_file { "local_file" } else { "artifact_manifest_missing_media" },
                "file_id": file_id,
                "artifact_id": artifact.artifact_id,
                "workflow_id": artifact.workflow_id,
                "storage_backend": artifact.storage_backend,
                "storage_key": artifact.storage_key,
                "file_path": artifact.file_path,
                "file_size_bytes": artifact.bytes,
                "public_url": artifact.public_url,
                "preview_url": artifact.preview_url,
                "local_file_present": local_file,
            });

            tracing::info!(
                link = %link,
                file_id = %file_id,
                artifact_id = %artifact.artifact_id,
                workflow_id = ?artifact.workflow_id,
                exists = manifest_exists || local_file,
                reason = result["reason"].as_str().unwrap_or(""),
                "Verified output artifact via manifest"
            );

            return result;
        }

        let path = crate::handlers::output::resolve_output_file_path_for_verification(file_id);
        match path {
            Some(path) => {
                let metadata = std::fs::metadata(&path).ok();
                let file_size = metadata.as_ref().map(|value| value.len()).unwrap_or(0);
                let result = serde_json::json!({
                    "link": link,
                    "kind": "output_file",
                    "exists": metadata.is_some(),
                    "reason": if metadata.is_some() { "legacy_local_scan" } else { "legacy_local_missing" },
                    "file_id": file_id,
                    "file_path": path.to_string_lossy(),
                    "file_size_bytes": file_size,
                });
                tracing::warn!(
                    link = %link,
                    file_id = %file_id,
                    exists = metadata.is_some(),
                    file_path = %path.to_string_lossy(),
                    "Fell back to legacy local-file artifact verification"
                );
                result
            }
            None => {
                tracing::warn!(
                    link = %link,
                    file_id = %file_id,
                    "Artifact verification failed: no manifest or file-path match"
                );
                serde_json::json!({
                    "link": link,
                    "kind": "output_file",
                    "exists": false,
                    "reason": "No artifact manifest or file path matched the output file identifier",
                })
            }
        }
    }

    async fn verify_delivery(pool: &PgPool, link: &str, delivery_id: &str) -> serde_json::Value {
        let uuid = match uuid::Uuid::parse_str(delivery_id) {
            Ok(value) => value,
            Err(_) => {
                return serde_json::json!({
                    "link": link,
                    "kind": "delivery",
                    "exists": false,
                    "reason": "Delivery identifier is not a valid UUID",
                });
            }
        };

        let delivery_row = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT output_r2_url, preview_r2_url, status FROM deliveries WHERE id = $1",
        )
        .bind(uuid)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        if let Some((output_r2_url, preview_r2_url, status)) = delivery_row {
            let has_media = output_r2_url
                .as_deref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
                || preview_r2_url
                    .as_deref()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false);

            return serde_json::json!({
                "link": link,
                "kind": "delivery",
                "exists": has_media,
                "delivery_id": delivery_id,
                "status": status,
                "has_output_r2_url": output_r2_url.as_deref().map(|value| !value.trim().is_empty()).unwrap_or(false),
                "has_preview_r2_url": preview_r2_url.as_deref().map(|value| !value.trim().is_empty()).unwrap_or(false),
            });
        }

        let test_result_row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT output_r2_url, status FROM test_results WHERE id = $1",
        )
        .bind(uuid)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        if let Some((output_r2_url, status)) = test_result_row {
            let has_media = output_r2_url
                .as_deref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);

            return serde_json::json!({
                "link": link,
                "kind": "delivery",
                "exists": has_media,
                "delivery_id": delivery_id,
                "status": status,
                "has_output_r2_url": has_media,
                "source_table": "test_results",
            });
        }

        serde_json::json!({
            "link": link,
            "kind": "delivery",
            "exists": false,
            "delivery_id": delivery_id,
            "reason": "No delivery or test result record matched the identifier",
        })
    }
}
