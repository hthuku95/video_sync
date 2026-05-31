// Background job helper for token refresh
use crate::token_manager::TokenManager;
use sqlx::PgPool;

/// Refresh all YouTube channel tokens that are expiring soon
/// This function is called by the background worker every 15 minutes
pub async fn refresh_all_expiring_tokens(
    token_manager: &TokenManager,
    _db_pool: &PgPool,
) -> Result<usize, String> {
    token_manager.refresh_all_expiring_tokens().await
}
