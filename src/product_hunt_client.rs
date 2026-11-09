use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const API_ENDPOINT: &str = "https://api.producthunt.com/v2/api/graphql";
const TOKEN_ENDPOINT: &str = "https://api.producthunt.com/v2/oauth/token";
const ENV_API_KEY: &str = "PRODUCT_HUNT_API_KEY";
const ENV_API_SECRET: &str = "PRODUCT_HUNT_API_SECRET";
const TOKEN_TTL: Duration = Duration::from_secs(23 * 60 * 60);

pub fn api_key() -> Option<String> {
    std::env::var(ENV_API_KEY).ok().filter(|v| !v.is_empty())
}

/// Minted OAuth token cache (client_credentials grant, per PH docs:
/// PRODUCT_HUNT_API_KEY is the OAuth client_id, PRODUCT_HUNT_API_SECRET the
/// client secret — a raw key stored in that env var is NOT a usable Bearer
/// token, it always 401s until exchanged).
static TOKEN_CACHE: std::sync::OnceLock<RwLock<Option<(String, Instant)>>> =
    std::sync::OnceLock::new();

fn token_cache() -> &'static RwLock<Option<(String, Instant)>> {
    TOKEN_CACHE.get_or_init(|| RwLock::new(None))
}

async fn mint_access_token(client_id: &str, client_secret: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_ENDPOINT)
        .basic_auth(client_id, Some(client_secret))
        .form(&[("grant_type", "client_credentials")])
        .send()
        .await
        .map_err(|e| format!("Product Hunt token exchange failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read Product Hunt token response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "Product Hunt token exchange returned {status}: {body}"
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse Product Hunt token response: {e} (body: {body})"))?;
    parsed["access_token"]
        .as_str()
        .map(|t| t.to_string())
        .ok_or_else(|| format!("Product Hunt token response missing access_token (body: {body})"))
}

async fn invalidate_token_cache() {
    *token_cache().write().await = None;
}

/// Fresh (or cached) Bearer token for the Product Hunt GraphQL API.
/// Prefers the OAuth client-credentials exchange; falls back to using
/// PRODUCT_HUNT_API_KEY directly as a Bearer token if no secret is present.
async fn access_token() -> Result<String, String> {
    {
        let cached = token_cache().read().await;
        if let Some((token, minted_at)) = cached.as_ref() {
            if minted_at.elapsed() < TOKEN_TTL {
                return Ok(token.clone());
            }
        }
    }
    let mut lock = token_cache().write().await;
    // Double-check after acquiring the write lock.
    if let Some((token, minted_at)) = lock.as_ref() {
        if minted_at.elapsed() < TOKEN_TTL {
            return Ok(token.clone());
        }
    }
    let client_id = api_key().ok_or_else(|| {
        format!("{ENV_API_KEY} not set — needed as the Product Hunt OAuth client_id")
    })?;

    if let Some(secret) = std::env::var(ENV_API_SECRET)
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let secret = secret.trim().trim_matches('"').to_string();
        let token = mint_access_token(&client_id, &secret).await?;
        *lock = Some((token.clone(), Instant::now()));
        tracing::info!("product_hunt: minted fresh access token (client_credentials)");
        return Ok(token);
    }

    tracing::warn!("product_hunt: {ENV_API_SECRET} not set — trying PRODUCT_HUNT_API_KEY directly (works only for developer tokens)");
    let _ = lock.replace((client_id.clone(), Instant::now()));
    Ok(client_id)
}

#[derive(Debug, Deserialize)]
pub struct ProductHuntPost {
    pub id: String,
    pub name: String,
    pub tagline: Option<String>,
    pub description: Option<String>,
    pub website: Option<String>,
    pub url: Option<String>,
    pub votes_count: Option<i64>,
    pub thumbnail: Option<ProductHuntThumbnail>,
}

#[derive(Debug, Deserialize)]
pub struct ProductHuntThumbnail {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQLResponse {
    pub data: Option<GraphQLData>,
    pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQLData {
    pub posts: Option<PostConnection>,
    pub search: Option<SearchResult>,
    pub post: Option<ProductHuntPost>,
}

#[derive(Debug, Deserialize)]
pub struct PostConnection {
    pub edges: Option<Vec<PostEdge>>,
}

#[derive(Debug, Deserialize)]
pub struct PostEdge {
    pub node: ProductHuntPost,
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub posts: Option<PostConnection>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQLError {
    pub message: String,
}

/// Top-level categories/topics on Product Hunt
pub fn default_topics() -> Vec<&'static str> {
    vec![
        "ai",
        "saas",
        "developer-tools",
        "productivity",
        "design-tools",
        "marketing",
        "analytics",
        "automation",
        "no-code",
        "data-science",
        "machine-learning",
        "api",
        "chrome-extension",
        "note-taking",
        "task-management",
        "customer-communication",
        "growth-hacking",
        "seo",
        "email-marketing",
        "social-media",
    ]
}

/// Shared GraphQL runner: mints the token (cached), posts the query, and
/// retries ONCE with a fresh token if the cached one went stale (401).
async fn graphql(body: serde_json::Value) -> Result<String, String> {
    let client = reqwest::Client::new();

    for attempt in 0..=1 {
        let token = access_token().await?;
        let resp = client
            .post(API_ENDPOINT)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Product Hunt API request failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        let auth_failed = status == reqwest::StatusCode::UNAUTHORIZED
            || text.contains("invalid_oauth_token")
            || text.contains("unauthenticated");
        if auth_failed && attempt == 0 {
            invalidate_token_cache().await;
            continue;
        }

        if !status.is_success() {
            return Err(format!("Product Hunt API returned {status}: {text}"));
        }
        return Ok(text);
    }
    Err("Product Hunt API kept rejecting the access token after refresh".to_string())
}

fn parse_graphql(text: &str) -> Result<GraphQLResponse, String> {
    serde_json::from_str(text).map_err(|e| {
        format!(
            "Failed to parse Product Hunt response: {e} (body head: {})",
            { text.chars().take(240).collect::<String>() }
        )
    })
}

/// Fetch top posts from Product Hunt by topic/order.
pub async fn fetch_top_posts(
    topic: Option<&str>,
    first: i32,
    order: Option<&str>,
) -> Result<Vec<ProductHuntPost>, String> {
    let mut query_vars = HashMap::new();
    query_vars.insert("first".to_string(), serde_json::json!(first));
    query_vars.insert(
        "order".to_string(),
        serde_json::json!(order.unwrap_or("VOTES")),
    );

    let topic_filter = if let Some(t) = topic {
        format!(r#", topic: "{}""#, t)
    } else {
        String::new()
    };

    let graphql_query = serde_json::json!({
        "query": format!(r#"
            query($first: Int!, $order: PostsOrder!) {{
                posts(first: $first, order: $order{topic_filter}) {{
                    edges {{
                        node {{
                            id
                            name
                            tagline
                            description
                            website
                            url
                            votesCount
                            thumbnail {{ url }}
                        }}
                    }}
                }}
            }}
        "#),
        "variables": query_vars,
    });

    let text = graphql(graphql_query).await?;
    let graphql_resp = parse_graphql(&text)?;

    if let Some(errors) = &graphql_resp.errors {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        if graphql_resp.data.is_none() {
            return Err(format!("Product Hunt GraphQL errors: {:?}", msgs));
        }
    }

    Ok(graphql_resp
        .data
        .and_then(|d| d.posts)
        .and_then(|c| c.edges)
        .unwrap_or_default()
        .into_iter()
        .map(|edge| edge.node)
        .collect())
}

pub async fn fetch_post_by_slug(slug: &str) -> Result<ProductHuntPost, String> {
    let graphql_query = serde_json::json!({
        "query": r#"
            query($slug: String!) {
                post(slug: $slug) {
                    id
                    name
                    tagline
                    description
                    website
                    url
                    votesCount
                    thumbnail { url }
                }
            }
        "#,
        "variables": {
            "slug": slug,
        },
    });

    let text = graphql(graphql_query).await?;
    let graphql_resp = parse_graphql(&text)?;

    if let Some(errors) = &graphql_resp.errors {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        if graphql_resp.data.is_none() {
            return Err(format!("Product Hunt GraphQL errors: {:?}", msgs));
        }
        tracing::warn!("Product Hunt GraphQL partial errors: {:?}", msgs);
    }

    graphql_resp
        .data
        .and_then(|d| d.post)
        .ok_or_else(|| format!("Product not found: {}", slug))
}

/// Search Product Hunt for posts matching a query
pub async fn search_posts(query: &str, first: i32) -> Result<Vec<ProductHuntPost>, String> {
    let graphql_query = serde_json::json!({
        "query": r#"
            query($query: String!, $first: Int!) {
                search(query: $query, first: $first) {
                    posts {
                        edges {
                            node {
                                id
                                name
                                tagline
                                description
                                website
                                url
                                votesCount
                                thumbnail { url }
                            }
                        }
                    }
                }
            }
        "#,
        "variables": {
            "query": query,
            "first": first,
        },
    });

    let text = graphql(graphql_query).await?;
    let graphql_resp = parse_graphql(&text)?;

    if let Some(errors) = &graphql_resp.errors {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        if graphql_resp.data.is_none() {
            return Err(format!("Product Hunt GraphQL errors: {:?}", msgs));
        }
        tracing::warn!("Product Hunt GraphQL partial errors: {:?}", msgs);
    }

    Ok(graphql_resp
        .data
        .and_then(|d| d.search)
        .and_then(|s| s.posts)
        .and_then(|conn| conn.edges)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.node)
        .collect())
}
