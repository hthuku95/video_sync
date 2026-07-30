use std::sync::Arc;

use crate::models::auth::{Claims, ErrorResponse};
use crate::AppState;
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

async fn is_whitelisted(db_pool: &sqlx::PgPool, email: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM whitelist_emails WHERE email = $1)",
    )
    .bind(email)
    .fetch_one(db_pool)
    .await
    .unwrap_or(false)
}

pub async fn admin_middleware(request: Request, next: Next) -> Result<Response, impl IntoResponse> {
    let claims = request.extensions().get::<Claims>();

    match claims {
        Some(claims) => {
            if claims.is_superuser || claims.is_staff {
                Ok(next.run(request).await)
            } else {
                Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        success: false,
                        message: "Admin access required. You must be staff or superuser."
                            .to_string(),
                    }),
                ))
            }
        }
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                success: false,
                message: "Authentication required for admin access.".to_string(),
            }),
        )),
    }
}

pub async fn admin_or_whitelisted_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let claims = request.extensions().get::<Claims>().cloned();

    match claims {
        Some(claims) => {
            if claims.is_superuser || claims.is_staff {
                return Ok(next.run(request).await);
            }
            let state = request.extensions().get::<Arc<AppState>>().cloned();
            match state {
                Some(state) => {
                    if is_whitelisted(&state.db_pool, &claims.email).await {
                        Ok(next.run(request).await)
                    } else {
                        Err((
                            StatusCode::FORBIDDEN,
                            Json(ErrorResponse {
                                success: false,
                                message: "Access restricted. Email not whitelisted.".to_string(),
                            }),
                        ))
                    }
                }
                None => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        success: false,
                        message: "Server state not available.".to_string(),
                    }),
                )),
            }
        }
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                success: false,
                message: "Authentication required.".to_string(),
            }),
        )),
    }
}

pub async fn superuser_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    // Get the claims from request extensions (set by auth middleware)
    let claims = request.extensions().get::<Claims>();

    match claims {
        Some(claims) => {
            if claims.is_superuser {
                Ok(next.run(request).await)
            } else {
                Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        success: false,
                        message: "Superuser access required.".to_string(),
                    }),
                ))
            }
        }
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                success: false,
                message: "Authentication required for superuser access.".to_string(),
            }),
        )),
    }
}
