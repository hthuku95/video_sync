// PhantomBuster API client
// Launches Sales Navigator export phantoms and fetches lead results.
//
// Flow:
//   1. list_agents()            — find the Sales Navigator Search Export agent ID
//   2. launch_agent(id, url)    — start a scrape run for a given search URL
//   3. poll_agent_status(id)    — wait until status = "finished"
//   4. fetch_agent_output(id)   — download the CSV/JSON result
//   5. parse_leads(output)      — convert to Vec<LinkedInLead>

use reqwest::Client;
use serde::{Deserialize, Serialize};

const PB_BASE: &str = "https://api.phantombuster.com/api/v2";

#[derive(Clone)]
pub struct PhantomBusterClient {
    api_key: String,
    http:    Client,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PbAgent {
    pub id:             String,
    pub name:           String,
    #[serde(rename = "lastEndStatus")]
    pub last_end_status: Option<String>,
    #[serde(rename = "lastEndMessage")]
    pub last_end_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PbLaunchResponse {
    pub status:       String,
    #[serde(rename = "containerId")]
    pub container_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LinkedInLead {
    pub full_name:    String,
    pub job_title:    Option<String>,
    pub company_name: Option<String>,
    pub company_size: Option<String>,
    pub linkedin_url: Option<String>,
    pub email:        Option<String>,
    pub location:     Option<String>,
    pub seniority:    Option<String>,
}

impl PhantomBusterClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: Client::new(),
        }
    }

    /// List all agents in the account
    pub async fn list_agents(&self) -> Result<Vec<PbAgent>, String> {
        let resp = self.http
            .get(format!("{}/agents/fetch-all", PB_BASE))
            .header("X-Phantombuster-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| format!("PhantomBuster request failed: {}", e))?;

        let agents: Vec<PbAgent> = resp.json().await
            .map_err(|e| format!("Failed to parse agents: {}", e))?;
        Ok(agents)
    }

    /// Find the best Sales Navigator agent for search-URL-based scraping.
    /// Prefers "Search Export" over "List Export".
    pub async fn find_sales_nav_agent(&self) -> Result<Option<PbAgent>, String> {
        let agents = self.list_agents().await?;
        // Prefer Search Export (accepts salesNavigatorUrl)
        let search_export = agents.iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains("sales navigator") && n.contains("search")
        }).cloned();
        if search_export.is_some() { return Ok(search_export); }
        // Fall back to any Sales Nav agent
        let any = agents.into_iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains("sales navigator") || n.contains("salesnav")
        });
        Ok(any)
    }

    /// Find a Sales Navigator List Export agent (uses spreadsheetUrl / saved list).
    pub async fn find_list_export_agent(&self) -> Result<Option<PbAgent>, String> {
        let agents = self.list_agents().await?;
        let found = agents.into_iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains("list export") || (n.contains("sales navigator") && n.contains("list"))
        });
        Ok(found)
    }

    /// Launch a Sales Navigator **Search Export** phantom with a search URL.
    /// The phantom argument key is `salesNavigatorUrl`.
    pub async fn launch_agent(
        &self,
        agent_id: &str,
        search_url: &str,
        session_cookie: &str,
        max_profiles: u32,
    ) -> Result<String, String> {
        let argument = serde_json::json!({
            "sessionCookie": session_cookie,
            "searches": search_url,
            "numberOfResultsPerSearch": max_profiles,
            "csvName": format!("leads_{}", chrono::Utc::now().timestamp())
        });

        let body = serde_json::json!({
            "id": agent_id,
            "argument": argument.to_string()
        });

        let resp = self.http
            .post(format!("{}/agents/launch", PB_BASE))
            .header("X-Phantombuster-Key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Launch failed: {}", e))?;

        let result: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse launch response: {}", e))?;

        if result.get("status").and_then(|s| s.as_str()) == Some("error") {
            return Err(format!("PhantomBuster launch error: {}", result.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")));
        }

        let container_id = result.get("containerId")
            .and_then(|c| c.as_str())
            .unwrap_or(agent_id)
            .to_string();
        Ok(container_id)
    }

    /// Launch a Sales Navigator **List Export** phantom with a saved list URL.
    /// The phantom argument key is `spreadsheetUrl`.
    pub async fn launch_list_export(
        &self,
        agent_id: &str,
        list_url: &str,
        session_cookie: &str,
        max_profiles: u32,
    ) -> Result<String, String> {
        let argument = serde_json::json!({
            "sessionCookie": session_cookie,
            "spreadsheetUrl": list_url,
            "numberOfResultsPerLaunch": max_profiles,
            "csvName": format!("leads_{}", chrono::Utc::now().timestamp())
        });

        let body = serde_json::json!({
            "id": agent_id,
            "argument": argument.to_string()
        });

        let resp = self.http
            .post(format!("{}/agents/launch", PB_BASE))
            .header("X-Phantombuster-Key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Launch failed: {}", e))?;

        let result: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse launch response: {}", e))?;

        if result.get("status").and_then(|s| s.as_str()) == Some("error") {
            return Err(format!("PhantomBuster launch error: {}", result.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")));
        }

        let container_id = result.get("containerId")
            .and_then(|c| c.as_str())
            .unwrap_or(agent_id)
            .to_string();
        Ok(container_id)
    }

    /// Check if the agent has finished running
    pub async fn get_agent_status(&self, agent_id: &str) -> Result<(String, Option<String>), String> {
        let resp = self.http
            .get(format!("{}/agents/fetch?id={}", PB_BASE, agent_id))
            .header("X-Phantombuster-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| format!("Status check failed: {}", e))?;

        let agent: PbAgent = resp.json().await
            .map_err(|e| format!("Failed to parse agent status: {}", e))?;

        Ok((
            agent.last_end_status.unwrap_or_else(|| "running".to_string()),
            agent.last_end_message,
        ))
    }

    /// Fetch the output JSON from the last completed run
    pub async fn fetch_output(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        // Get agent output container
        let resp = self.http
            .get(format!("{}/agents/fetch-output?id={}&withoutResultObject=true", PB_BASE, agent_id))
            .header("X-Phantombuster-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| format!("Output fetch failed: {}", e))?;

        let output: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse output: {}", e))?;

        // Output is a JSON array of lead objects
        let leads = if let Some(arr) = output.as_array() {
            arr.clone()
        } else if let Some(arr) = output.get("output").and_then(|o| o.as_array()) {
            arr.clone()
        } else {
            vec![]
        };

        Ok(leads)
    }

    /// Inspect the last run of an agent. Returns whether PB reports a terminal
    /// error and, if so, the last ~800 chars of the log so callers can mark
    /// their DB row `failed` with a meaningful error instead of hanging on
    /// `running` forever when the Phantom errored at launch.
    ///
    /// Returns `Ok((is_errored, log_tail_opt))`:
    /// * `is_errored = true` when `containerStatus == "not running"` combined
    ///   with a non-zero `exitCode`, OR when the log contains `[error]`.
    /// * `log_tail_opt` is the last ~800 chars of the log when errored.
    pub async fn fetch_run_error(&self, agent_id: &str) -> Result<(bool, Option<String>), String> {
        let resp = self.http
            .get(format!("{}/agents/fetch-output?id={}", PB_BASE, agent_id))
            .header("X-Phantombuster-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| format!("fetch_run_error request failed: {}", e))?;

        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("fetch_run_error parse failed: {}", e))?;

        // PB wraps the real payload under `data` in some responses.
        let payload = body.get("data").cloned().unwrap_or(body);

        let exit_code = payload.get("exitCode").and_then(|v| v.as_i64());
        let container_status = payload
            .get("containerStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let log = payload
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let finished_with_error =
            (container_status == "not running" && exit_code.unwrap_or(0) != 0)
                || log.contains("[error]");

        if finished_with_error {
            // Walk back to a valid UTF-8 boundary before slicing — PB logs can
            // contain emoji (e.g. ❌) that would panic on mid-codepoint slice.
            let mut start = log.len().saturating_sub(800);
            while start > 0 && !log.is_char_boundary(start) {
                start -= 1;
            }
            let tail = log[start..].trim().to_string();
            Ok((true, Some(tail)))
        } else {
            Ok((false, None))
        }
    }

    /// Build a Sales Navigator people-search URL from filter params.
    ///
    /// Supported filters:
    ///   - job_titles   : e.g. ["YouTuber", "Podcast Host", "Content Creator"]
    ///   - industries   : e.g. ["Online Media", "E-Learning", "Marketing and Advertising"]
    ///   - company_sizes: e.g. ["A", "B"] where A=1-10, B=11-50, C=51-200, D=201-500
    ///   - locations    : LinkedIn geo IDs or plain text names (text-only, no geo ID lookup)
    ///   - seniority    : e.g. ["OWNER", "PARTNER", "CXO", "VP", "DIRECTOR", "MANAGER"]
    pub fn build_search_url(
        job_titles:    &[String],
        industries:    &[String],
        company_sizes: &[String],
        locations:     &[String],
        seniority:     &[String],
    ) -> String {
        let mut filters: Vec<String> = vec![];

        if !job_titles.is_empty() {
            let vals: Vec<String> = job_titles.iter()
                .map(|t| format!("(text:{},selectionType:INCLUDED)", Self::encode_lurn(t)))
                .collect();
            filters.push(format!("(type:CURRENT_TITLE,values:List({}))", vals.join(",")));
        }

        if !industries.is_empty() {
            let vals: Vec<String> = industries.iter()
                .map(|i| format!("(text:{},selectionType:INCLUDED)", Self::encode_lurn(i)))
                .collect();
            filters.push(format!("(type:INDUSTRY,values:List({}))", vals.join(",")));
        }

        if !company_sizes.is_empty() {
            // Map human-readable sizes to LinkedIn codes
            let vals: Vec<String> = company_sizes.iter().map(|s| {
                let upper = s.to_uppercase();
                let code = match upper.as_str() {
                    "1-10"    | "A" => "A",
                    "11-50"   | "B" => "B",
                    "51-200"  | "C" => "C",
                    "201-500" | "D" => "D",
                    "501-1000"| "E" => "E",
                    other           => other,
                };
                format!("(id:{},selectionType:INCLUDED)", code)
            }).collect();
            filters.push(format!("(type:COMPANY_HEADCOUNT,values:List({}))", vals.join(",")));
        }

        if !locations.is_empty() {
            let vals: Vec<String> = locations.iter()
                .map(|l| format!("(text:{},selectionType:INCLUDED)", Self::encode_lurn(l)))
                .collect();
            filters.push(format!("(type:GEOGRAPHY,values:List({}))", vals.join(",")));
        }

        if !seniority.is_empty() {
            let vals: Vec<String> = seniority.iter()
                .map(|s| format!("(id:{},selectionType:INCLUDED)", s.to_uppercase()))
                .collect();
            filters.push(format!("(type:SENIORITY_LEVEL,values:List({}))", vals.join(",")));
        }

        if filters.is_empty() {
            return "https://www.linkedin.com/sales/search/people".to_string();
        }

        format!(
            "https://www.linkedin.com/sales/search/people?query=(filters:List({}))",
            filters.join(",")
        )
    }

    fn encode_lurn(s: &str) -> String {
        // Minimal encoding: spaces → %20, special chars that break the filter syntax
        s.replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
            .replace(',', "%2C")
            .replace(':', "%3A")
    }

    /// Parse raw PhantomBuster output rows into LinkedInLead structs.
    /// Handles both Sales Navigator Search Export and List Export field names.
    pub fn parse_leads(rows: Vec<serde_json::Value>) -> Vec<LinkedInLead> {
        rows.into_iter().filter_map(|row| {
            let get = |key: &str| row.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());

            // fullName is set by both export types; fall back to first+last
            let full_name = get("fullName")
                .or_else(|| {
                    let f = get("firstName").unwrap_or_default();
                    let l = get("lastName").unwrap_or_default();
                    let combined = format!("{} {}", f, l).trim().to_string();
                    if combined.is_empty() { None } else { Some(combined) }
                })
                .or_else(|| get("name"))
                .unwrap_or_default();

            if full_name.is_empty() { return None; }

            // Profile URL — Sales Nav export uses "profileUrl" and "linkedInProfileUrl"
            let linkedin_url = get("linkedInProfileUrl")
                .or_else(|| get("profileUrl"))
                .or_else(|| get("defaultProfileUrl"))
                .or_else(|| get("linkedinUrl"));

            Some(LinkedInLead {
                full_name,
                job_title:    get("title").or_else(|| get("jobTitle")),
                company_name: get("companyName").or_else(|| get("company")),
                company_size: get("companySize"),
                linkedin_url,
                email:        get("email"),
                location:     get("location"),
                seniority:    get("seniorityLevel").or_else(|| get("seniority")),
            })
        }).collect()
    }
}

// ── Instagram lead scraping ───────────────────────────────────────────────────

/// An Instagram profile/creator discovered via PhantomBuster.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstagramLead {
    pub username:        String,
    pub full_name:       Option<String>,
    pub bio:             Option<String>,
    pub followers_count: Option<i64>,
    pub following_count: Option<i64>,
    pub posts_count:     Option<i32>,
    pub profile_url:     Option<String>,
    pub profile_pic_url: Option<String>,
    pub is_private:      bool,
    pub is_verified:     bool,
    pub external_url:    Option<String>,
    pub email:           Option<String>,
}

impl PhantomBusterClient {
    /// Find the Instagram Hashtag Search Export phantom.
    pub async fn find_instagram_hashtag_agent(&self) -> Result<Option<PbAgent>, String> {
        let agents = self.list_agents().await?;
        let found = agents.into_iter().find(|a| {
            let n = a.name.to_lowercase();
            (n.contains("instagram") && n.contains("hashtag")) ||
            (n.contains("instagram") && n.contains("search"))
        });
        Ok(found)
    }

    /// Find the Instagram Profile Scraper phantom.
    pub async fn find_instagram_profile_scraper(&self) -> Result<Option<PbAgent>, String> {
        let agents = self.list_agents().await?;
        let found = agents.into_iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains("instagram") && (n.contains("profile") || n.contains("scraper"))
        });
        Ok(found)
    }

    /// Launch an Instagram Hashtag Search Export to find profiles posting under a hashtag.
    ///
    /// * `agent_id`       — the Instagram Hashtag phantom agent ID
    /// * `session_cookie` — Instagram `sessionid` cookie value
    /// * `hashtag`        — hashtag (with or without #, e.g. "contentcreator")
    /// * `max_posts`      — number of posts to scrape (each post → one lead candidate)
    pub async fn launch_instagram_hashtag_search(
        &self,
        agent_id:       &str,
        session_cookie: &str,
        hashtag:        &str,
        max_posts:      u32,
    ) -> Result<String, String> {
        let tag = hashtag.trim_start_matches('#');

        let argument = serde_json::json!({
            "sessionCookie":          session_cookie,
            "spreadsheetUrl":         format!("#{}", tag),
            "maxPosts":               max_posts,
            "numberOfLinesPerLaunch": 10,
        });

        let body = serde_json::json!({
            "id":       agent_id,
            "argument": argument.to_string()
        });

        let resp = self.http
            .post(format!("{}/agents/launch", PB_BASE))
            .header("X-Phantombuster-Key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Instagram launch failed: {}", e))?;

        let result: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse Instagram launch response: {}", e))?;

        if result.get("status").and_then(|s| s.as_str()) == Some("error") {
            return Err(format!("PhantomBuster launch error: {}", result.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")));
        }

        let container_id = result.get("containerId")
            .and_then(|c| c.as_str())
            .unwrap_or(agent_id)
            .to_string();
        Ok(container_id)
    }

    /// Parse raw PhantomBuster Instagram Hashtag output into InstagramLead structs.
    pub fn parse_instagram_leads(rows: Vec<serde_json::Value>) -> Vec<InstagramLead> {
        rows.into_iter().filter_map(|row| {
            let get_str  = |key: &str| row.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
            let get_i64  = |key: &str| row.get(key).and_then(|v| v.as_i64());
            let get_i32  = |key: &str| row.get(key).and_then(|v| v.as_i64()).map(|v| v as i32);
            let get_bool = |key: &str| row.get(key).and_then(|v| v.as_bool()).unwrap_or(false);

            let username = get_str("username")
                .or_else(|| get_str("handle"))
                .unwrap_or_default();
            if username.is_empty() { return None; }

            let profile_url = get_str("profileUrl")
                .or_else(|| get_str("url"))
                .or_else(|| Some(format!("https://www.instagram.com/{}/", username)));

            Some(InstagramLead {
                username,
                full_name:       get_str("fullName").or_else(|| get_str("name")),
                bio:             get_str("biography").or_else(|| get_str("bio")),
                followers_count: get_i64("followersCount").or_else(|| get_i64("followers")),
                following_count: get_i64("followingCount").or_else(|| get_i64("following")),
                posts_count:     get_i32("postsCount").or_else(|| get_i32("posts")),
                profile_url,
                profile_pic_url: get_str("profilePictureUrl").or_else(|| get_str("imgUrl")),
                is_private:      get_bool("isPrivate"),
                is_verified:     get_bool("isVerified") || get_bool("verified"),
                external_url:    get_str("externalUrl").or_else(|| get_str("website")),
                email:           get_str("email"),
            })
        }).collect()
    }
}
