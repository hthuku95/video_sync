use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const API_ENDPOINT: &str = "https://api.producthunt.com/v2/api/graphql";
const ENV_API_KEY: &str = "PRODUCT_HUNT_API_KEY";

pub fn api_key() -> Option<String> {
    std::env::var(ENV_API_KEY).ok().filter(|v| !v.is_empty())
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
        "ai", "saas", "developer-tools", "productivity", "design-tools",
        "marketing", "analytics", "automation", "no-code", "data-science",
        "machine-learning", "api", "chrome-extension", "note-taking",
        "task-management", "customer-communication", "growth-hacking",
        "seo", "email-marketing", "social-media",
    ]
}

/// Fetch top posts from Product Hunt by topic/order.
/// Uses the API key as Bearer token (no OAuth flow needed for public data).
pub async fn fetch_top_posts(
    topic: Option<&str>,
    first: i32,
    order: Option<&str>,
) -> Result<Vec<ProductHuntPost>, String> {
    let key = api_key().ok_or_else(|| "PRODUCT_HUNT_API_KEY not set".to_string())?;

    // Build the GraphQL query
    let mut query_vars = HashMap::new();
    query_vars.insert("first".to_string(), serde_json::json!(first));
    query_vars.insert("order".to_string(), serde_json::json!(order.unwrap_or("VOTES")));

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

    let client = reqwest::Client::new();
    let resp = client
        .post(API_ENDPOINT)
        .header("Authorization", format!("Bearer {}", key))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&graphql_query)
        .send()
        .await
        .map_err(|e| format!("Product Hunt API request failed: {}", e))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Product Hunt API returned {}: {}", status, body));
    }

    let graphql_resp: GraphQLResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse Product Hunt API response: {} (body: {})", e, body))?;

    if let Some(errors) = &graphql_resp.errors {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        if graphql_resp.data.is_none() {
            return Err(format!("Product Hunt GraphQL errors: {:?}", msgs));
        }
        tracing::warn!("Product Hunt GraphQL partial errors: {:?}", msgs);
    }

    let posts = graphql_resp
        .data
        .and_then(|d| d.posts)
        .and_then(|c| c.edges)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.node)
        .collect();

    Ok(posts)
}

pub async fn fetch_post_by_slug(slug: &str) -> Result<ProductHuntPost, String> {
    let key = api_key().ok_or_else(|| "PRODUCT_HUNT_API_KEY not set".to_string())?;

    let graphql_query = serde_json::json!({
        "query": format!(r#"
            query($slug: String!) {{
                post(slug: $slug) {{
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
        "#),
        "variables": {
            "slug": slug,
        },
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(API_ENDPOINT)
        .header("Authorization", format!("Bearer {}", key))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&graphql_query)
        .send()
        .await
        .map_err(|e| format!("Product Hunt API request failed: {}", e))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Product Hunt API returned {}: {}", status, body));
    }

    let graphql_resp: GraphQLResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse response: {} (body: {})", e, body))?;

    if let Some(errors) = &graphql_resp.errors {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        return Err(format!("Product Hunt GraphQL errors: {:?}", msgs));
    }

    graphql_resp
        .data
        .and_then(|d| d.post)
        .ok_or_else(|| format!("Product not found: {}", slug))
}

/// Search Product Hunt for posts matching a query
pub async fn search_posts(query: &str, first: i32) -> Result<Vec<ProductHuntPost>, String> {
    let key = api_key().ok_or_else(|| "PRODUCT_HUNT_API_KEY not set".to_string())?;

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

    let client = reqwest::Client::new();
    let resp = client
        .post(API_ENDPOINT)
        .header("Authorization", format!("Bearer {}", key))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&graphql_query)
        .send()
        .await
        .map_err(|e| format!("Product Hunt API request failed: {}", e))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Product Hunt API returned {}: {}", status, body));
    }

    let graphql_resp: GraphQLResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse response: {} (body: {})", e, body))?;

    if let Some(errors) = &graphql_resp.errors {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        if graphql_resp.data.is_none() {
            return Err(format!("Product Hunt GraphQL errors: {:?}", msgs));
        }
        tracing::warn!("Product Hunt GraphQL partial errors: {:?}", msgs);
    }

    let posts = graphql_resp
        .data
        .and_then(|d| d.search)
        .and_then(|s| s.posts)
        .and_then(|c| c.edges)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.node)
        .collect();

    Ok(posts)
}
