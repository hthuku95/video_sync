use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct WhitelistEmail {
    pub id: i32,
    pub email: String,
    pub added_by: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct SystemSetting {
    pub id: i32,
    pub setting_key: String,
    pub setting_value: String,
    pub setting_type: String,
    pub description: Option<String>,
    pub updated_by: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WhitelistEmailRequest {
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WhitelistEmailResponse {
    pub id: i32,
    pub email: String,
    pub added_by: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WhitelistToggleRequest {
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToggleActiveRequest {
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToggleDfyCustomerRequest {
    pub is_dfy_customer: bool,
}

impl From<WhitelistEmail> for WhitelistEmailResponse {
    fn from(whitelist: WhitelistEmail) -> Self {
        WhitelistEmailResponse {
            id: whitelist.id,
            email: whitelist.email,
            added_by: whitelist.added_by,
            created_at: whitelist.created_at,
        }
    }
}

impl SystemSetting {
    pub fn as_bool(&self) -> Result<bool, String> {
        match self.setting_type.as_str() {
            "boolean" => match self.setting_value.as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(format!("Invalid boolean value: {}", self.setting_value)),
            },
            _ => Err(format!(
                "Setting {} is not a boolean type",
                self.setting_key
            )),
        }
    }

}
