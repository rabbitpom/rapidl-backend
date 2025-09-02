use axum::{
    async_trait,
    middleware::Next,
    http::{Request, StatusCode},
    response::Response,
    body::Body,
    extract::FromRequest,
};
use axum_extra::extract::cookie;

use crate::{
    Response::{ServerResponse, status_response, internal_server_error},
    Auth::{is_valid_signed_token, is_timestamp_expired},
    Constants,
};
use common_types::Ip::try_fetch_ipv6;

#[derive(Copy, Clone)]
pub struct AccessTokenDescription {
    pub user_id: i64,
    pub has_support_privilege: bool,
}

#[async_trait]
impl<S, B> FromRequest<S, B> for AccessTokenDescription
where
    B: Send + 'static,
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request(req: Request<B>, _: &S) -> Result<Self, Self::Rejection> {
        if let Some(token) = req.extensions().get::<AccessTokenDescription>() {
            Ok(token.clone())
        } else {
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// Checks for X-ATK token
// Checks if ip claim matches
// Checks if token has not expired
// Checks for valid userId
// Then calls next, with Extension<AccessTokenDescription>
#[tracing::instrument(skip(req, next))]
pub async fn middleware(req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let (mut parts, body) = req.into_parts();
    // Attempt to find client IP from headers
    let ipv6 = try_fetch_ipv6(&parts.headers, *Constants::DEVELOPMENT_MODE).ok_or(status_response(StatusCode::FORBIDDEN, "Forbidden headers"))?.to_string();
    // Attempt to find the refresh tokens
    let jar = cookie::CookieJar::from_headers(&parts.headers);
    if let Some(access_token) = jar.get("X-ATK") {
        tracing::info!("Verifying X-ATK token");
        let Ok(claims) = is_valid_signed_token(access_token.value()) else {
            tracing::warn!("X-ATK token provided was not valid, rejected request to revoke token");
            // Would be more right to return BAD_REQUEST
            // but that gives hints to the attacker!
            return Err(internal_server_error("Internal Server Error"))
        };
        let ip = claims.get("ip").ok_or_else(|| {
            tracing::error!("X-ATK token has no 'ip' field");
            internal_server_error("Internal Server Error")
        })?;
        if ip != &ipv6 {
            tracing::warn!("X-ATK token rejected as IPV6 mismatch");
            return Err(status_response(StatusCode::UNAUTHORIZED, "Invalid Token"))
        }
        let expire = claims.get("expire").ok_or_else(|| {
            tracing::error!("X-ATK token has no 'expire' field");
            internal_server_error("Internal Server Error")
        })?.parse::<i64>().map_err(|_| {
            tracing::error!("X-ATK token 'expire' field failed to parse into 'i64'");
            internal_server_error("Failed to cast")
        })?;
        if is_timestamp_expired(expire) {
            tracing::warn!("X-ATK token rejected as expired");
            return Err(status_response(StatusCode::UNAUTHORIZED, "Invalid Token"))
        }
        let user_id = claims.get("userId").ok_or_else(|| {
            tracing::error!("X-ATK token has no 'userId' field");
            internal_server_error("Internal Server Error")
        })?.parse::<i64>().map_err(|_| {
            tracing::error!("X-ATK token 'userId' field failed to parse into 'i64'");
            internal_server_error("Failed to cast")
        })?;

        tracing::info!("X-ATK token verified");

        parts.extensions.insert(AccessTokenDescription {
            user_id,
            has_support_privilege: claims.get("supportprivilege").is_some(),
        });

        let response = next.run(Request::from_parts(parts,body)).await;
        return Ok(response)
    }
    tracing::warn!("Could not find X-ATK token, failed to verify");

    Err(status_response(StatusCode::UNAUTHORIZED, "Invalid token"))
}

