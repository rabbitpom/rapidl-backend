use chrono::Utc;
use axum::{
    async_trait,
    middleware::Next,
    http::{Request, StatusCode, header::{SET_COOKIE, HeaderValue, HeaderMap}},
    response::Response,
    body::Body,
    extract::{FromRequest, State},
};
use axum_extra::extract::cookie;
use deadpool_redis::redis::cmd;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Auth::{is_valid_signed_token, gen_refresh_and_access_tokens, TokenData, is_timestamp_expired},
    Constants,
};
use common_types::Ip::try_fetch_ipv6;

#[derive(Copy, Clone)]
pub struct AccessTokenDescription {
    pub user_id: i64,
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

pub fn is_timestamp_close_to_expire_or_expired(now: i64, compare: i64, range: i64) -> bool {
    if now >= compare {
        return true;
    }
    compare - now < range
}

// Returns Ok(()) if access and refresh tokens are valid, otherwise Err(())
async fn are_tks_valid_from_header(appstate: &AppState, headers: &HeaderMap, ipv6: &String) -> Result<TokenData, ()> {
    // Attempt to find the refresh and access tokens
    let jar = cookie::CookieJar::from_headers(headers);
    let read_user_id;
    let has_support_privilege;
    if let Some(access_token) = jar.get("X-ATK") {
        tracing::info!("Verifying X-ATK token");
        let Ok(claims) = is_valid_signed_token(access_token.value()) else {
            tracing::warn!("X-ATK token provided was not valid");
            return Err(())
        };
        let ip = claims.get("ip").ok_or_else(|| {
            tracing::error!("X-ATK token has no 'ip' field");
        })?;
        if ip != ipv6 {
            tracing::warn!("X-ATK token rejected as IPV6 mismatch");
            return Err(())
        }
        let user_id = claims.get("userId").ok_or_else(|| {
            tracing::error!("X-ATK token has no 'userId' field");
        })?;
        read_user_id = user_id.parse::<i64>().or_else(|_| {
            tracing::error!("X-ATK token 'userId' failed to cast to i64");
            Err(())
        })?;
        let expire = claims.get("expire").ok_or_else(|| {
            tracing::error!("X-ATK token has no 'expire' field");
        })?.parse::<i64>().map_err(|_| {
            tracing::error!("X-ATK token 'expire' field failed to parse into 'i64'");
        })?;
        tracing::info!("Verified X-ATK token, now using heuristic comparison for expiration");
        if is_timestamp_close_to_expire_or_expired(Utc::now().timestamp(), expire, 20) {
            tracing::info!("X-ATK expiration is valid");
        } else {
            tracing::info!("X-ATK expiration is invalid");
            return Err(())
        }
        has_support_privilege = claims.get("supportprivilege").is_some();
    } else {
        tracing::warn!("Could not find X-ATK token, failed to verify");
        return Err(())
    }
    if let Some(refresh_token) = jar.get("X-RTK") {
        tracing::info!("Verifying X-RTK token");
        let Ok(claims) = is_valid_signed_token(refresh_token.value()) else {
            tracing::warn!("X-RTK token provided was not valid");
            return Err(())
        };
        let ip = claims.get("ip").ok_or_else(|| {
            tracing::error!("X-RTK token has no 'ip' field");
        })?;
        if ip != ipv6 {
            tracing::warn!("X-RTK token rejected as IPV6 mismatch");
            return Err(())
        }
        let expire = claims.get("rtk-expire").ok_or_else(|| {
            tracing::error!("X-RTK token has no 'rtk-expire' field");
        })?.parse::<i64>().map_err(|_| {
            tracing::error!("X-RTK token 'rtk-expire' field failed to parse into 'i64'");
        })?;
        if is_timestamp_expired(expire) {
            tracing::warn!("X-RTK token rejected as expired");
            return Err(())
        }
        let token_id = claims.get("id").ok_or_else(|| {
            tracing::error!("X-RTK token has no 'id' field");
        })?;
        let user_id = claims.get("userId").ok_or_else(|| {
            tracing::error!("X-RTK token has no 'userId' field");
        })?;
        let user_id = user_id.parse::<i64>().or_else(|_| {
            tracing::error!("X-RTK token 'userId' failed to cast to i64");
            Err(())
        })?;
        if user_id != read_user_id {
            tracing::error!("X-ATK and X-RTK tokens have mismatching 'userId'");
            return Err(())
        }
        let token_key = format!("user:rtk:{}", user_id);
        let mut conn = appstate.redis.get().await.map_err(|err|{
            tracing::info!("Failed to fetch Redis connection, {err}");
        })?;
        // Check if we get a matching ID
        tracing::info!("Querying redis database and comparing token id");
        let stored_token_id = match cmd("GET").arg(&[&token_key]).query_async::<_, Option<String>>(&mut conn).await {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("Redis GET command failed, {:?}", err);
                return Err(())
            }
        };
        let Some(stored_token_id) = stored_token_id else {
            tracing::warn!("No such X-RTK token exists for the user id");
            return Err(())
        };
        if &stored_token_id != token_id {
            tracing::warn!("X-RTK token id is invalid");
            return Err(())
        }
        tracing::info!("Verified X-RTK token");
        return Ok(TokenData {
            userid: read_user_id, 
            has_support_privilege
        });
    }
    tracing::warn!("Could not find X-RTK token, failed to verify");
    Err(())
}

// Checks for X-ATK token
// Checks if ip claim matches
// Checks if token is close to expiration (within 20 seconds), or if it has expired already
// Checks for valid userId
// Checks for valid X-RTK token
// Generates new access and refresh tokens
#[tracing::instrument(skip(appstate, req, next))]
pub async fn middleware(State(appstate): State<AppState>,req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let (parts, body) = req.into_parts();
    // Attempt to find client IP from headers
    let ipv6 = try_fetch_ipv6(&parts.headers, *Constants::DEVELOPMENT_MODE).ok_or(status_response(StatusCode::FORBIDDEN, "Forbidden headers"))?.to_string();
    // Verify token
    let Ok(token_data) = are_tks_valid_from_header(&appstate, &parts.headers, &ipv6).await else {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Invalid Token"))
    };

    // Call handler, they should give us an identifier
    let response = next.run(Request::from_parts(parts,body)).await;
    if !response.status().is_success() {
        return Ok(response);
    }
    let (mut parts, body) = response.into_parts();

    // Generate our cookies
    tracing::info!("Generating new access and refresh tokens");
    let tokens_package = gen_refresh_and_access_tokens(ipv6, &token_data).map_err(|err|{
        tracing::error!("Failed to generate tokens, {:?}", err);
        internal_server_error("Internal Server Error")
    })?;

    // Set cookie headers
    if *Constants::DEVELOPMENT_MODE {
        tracing::warn!("Using development mode cookies");
        let _ = parts.headers.remove("X-ATK");
        let _ = parts.headers.remove("X-RTK");
        parts.headers.append(SET_COOKIE, HeaderValue::from_str(
                format!("X-ATK={}; Path=/; Domain=.127.0.0.1; Expires={}; HttpOnly", tokens_package.access_token, tokens_package.access_expire_format).as_ref()
            ).unwrap()
        );
        parts.headers.append(SET_COOKIE, HeaderValue::from_str(
                format!("X-RTK={}; Path=/; Domain=.127.0.0.1; Expires={}; HttpOnly", tokens_package.refresh_token, tokens_package.refresh_expire_format).as_ref()
            ).unwrap()
        );
    } else {
        let _ = parts.headers.remove("X-ATK");
        let _ = parts.headers.remove("X-RTK");
        parts.headers.append(SET_COOKIE, HeaderValue::from_str(
                format!("X-ATK={}; Path=/; Domain=.rapidl.co.uk; Expires={}; SameSite=Strict; Secure; HttpOnly", tokens_package.access_token, tokens_package.access_expire_format).as_ref()
            ).unwrap()
        );
        parts.headers.append(SET_COOKIE, HeaderValue::from_str(
                format!("X-RTK={}; Path=/; Domain=.rapidl.co.uk; Expires={}; SameSite=Strict; Secure; HttpOnly", tokens_package.refresh_token, tokens_package.refresh_expire_format).as_ref()
            ).unwrap()
        );
    }

    tracing::info!("Querying redis database with refresh tokens");
    let mut conn = appstate.redis.get().await.map_err(|err|{
        tracing::error!("Failed to fetch Redis connection, {err}");
        internal_server_error("Internal service error")
    })?;
    if let Err(err) = cmd("SET")
        .arg(&[&format!("user:rtk:{}", token_data.userid), &tokens_package.refresh_id.to_string(), "EX", &(*Constants::REFRESH_TOKEN_EXPIRES_SEC).to_string()])
        .query_async::<_, ()>(&mut conn)
        .await
    {
        tracing::error!("Redis set command failed, {:?}", err);
        return Err(internal_server_error("Internal Service Error"))
    }

    tracing::info!("Successfully generated, and responded with new refresh and access tokens");
    Ok(Response::from_parts(parts, body))
}

