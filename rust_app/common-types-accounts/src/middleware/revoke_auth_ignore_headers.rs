use axum::{
    middleware::Next,
    extract::State,
    http::{Request, header::HeaderMap},
    response::Response,
    body::Body,
};
use axum_extra::extract::cookie;
use deadpool_redis::redis::cmd;

use crate::{
    Response::ServerResponse,
    State::AppState,
    Auth::is_valid_signed_token,
};

async fn try_remove_x_rtk_from_header(appstate: &AppState, headers: &HeaderMap) -> Result<(), ()> {
    // Attempt to find the refresh tokens
    let jar = cookie::CookieJar::from_headers(headers);
    if let Some(refresh_token) = jar.get("X-RTK") {
        tracing::info!("Verifying X-RTK token");
        let Ok(claims) = is_valid_signed_token(refresh_token.value()) else {
            tracing::warn!("X-RTK token provided was not valid, rejected request to revoke token");
            // Would be more right to return BAD_REQUEST
            // but that gives hints to the attacker!
            return Err(())
        };
        let token_id = claims.get("id").ok_or_else(|| {
            tracing::error!("X-RTK token has no 'id' field");
        })?;
        let user_id = claims.get("userId").ok_or_else(|| {
            tracing::error!("X-RTK token has no 'userId' field");
        })?;
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
            tracing::warn!("No such X-RTK token exists for the user id, request rejected");
            return Err(())
        };
        if &stored_token_id != token_id {
            tracing::warn!("X-RTK token id is invalid, rejected request");
            return Err(())
        }
        // Delete the token
        tracing::info!("X-RTK token verified, querying redis database to delete X-RTK token");
        if let Err(err) = cmd("DEL")
            .arg(&[&token_key])
            .query_async::<_, ()>(&mut conn)
            .await
        {
            tracing::error!("Redis DEL command failed, {:?}", err);
            return Err(())
        }
        tracing::info!("Successfully deleted X-RTK token");
    } else {
        tracing::warn!("Could not find X-RTK token, failed to verify");
    }
    Ok(())
}

// Revoke any tokens as long as
// they are valid
#[tracing::instrument(skip(appstate, req, next))]
pub async fn middleware(State(appstate): State<AppState>, req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let (parts, body) = req.into_parts();
    let _ = try_remove_x_rtk_from_header(&appstate, &parts.headers).await;
    return Ok(next.run(Request::from_parts(parts, body)).await)
}

