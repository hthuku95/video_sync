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

    /// Find the Sales Navigator Search Export agent ID
    pub async fn find_sales_nav_agent(&self) -> Result<Option<PbAgent>, String> {
        let agents = self.list_agents().await?;
        let found = agents.into_iter().find(|a| {
            let name = a.name.to_lowercase();
            name.contains("sales navigator") || name.contains("salesnav") || name.contains("sales nav")
        });
        Ok(found)
    }

    /// Launch an agent with a Sales Navigator search URL
    pub async fn launch_agent(
        &self,
        agent_id: &str,
        search_url: &str,
        session_cookie: &str,
        max_profiles: u32,
    ) -> Result<String, String> {
        let argument = serde_json::json!({
            "sessionCookie": session_cookie,
            "salesNavigatorUrl": search_url,
            "numberOfProfiles": max_profiles,
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

    /// Parse raw PhantomBuster output rows into LinkedInLead structs
    pub fn parse_leads(rows: Vec<serde_json::Value>) -> Vec<LinkedInLead> {
        rows.into_iter().filter_map(|row| {
            let get = |key: &str| row.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());

            let full_name = get("fullName")
                .or_else(|| get("firstName").zip(get("lastName")).map(|(f, l)| format!("{} {}", f, l)))
                .or_else(|| get("name"))
                .unwrap_or_default();

            if full_name.is_empty() { return None; }

            Some(LinkedInLead {
                full_name,
                job_title:    get("title").or_else(|| get("jobTitle")),
                company_name: get("companyName").or_else(|| get("company")),
                company_size: get("companySize"),
                linkedin_url: get("profileUrl").or_else(|| get("linkedinUrl")),
                email:        get("email"),
                location:     get("location"),
                seniority:    get("seniorityLevel").or_else(|| get("seniority")),
            })
        }).collect()
    }
}
