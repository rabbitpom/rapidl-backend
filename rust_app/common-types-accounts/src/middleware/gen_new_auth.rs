use ::std::sync::Arc;
use parking_lot::RwLock;
use axum::{
    middleware::Next,
    extract::State,
    http::{Request, StatusCode, header::{SET_COOKIE, HeaderValue}},
    response::Response,
    body::Body,
};
use deadpool_redis::redis::cmd;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState,
    Auth::{gen_refresh_and_access_tokens, TokenData},
    Constants,
};
use common_types::Ip::try_fetch_ipv6;

pub struct InternalTokenIdentifier {
    pub identifier: RwLock<Option<TokenData>>,
}
pub type TokenIdentifier = Arc<InternalTokenIdentifier>;

// If the handle in 'Next' succeeds,
// generate new access and refresh tokens.
#[tracing::instrument(skip(appstate, req, next))]
pub async fn middleware(State(appstate): State<AppState>, mut req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let token_identifier = Arc::new(InternalTokenIdentifier {
        identifier: RwLock::new(None),
    });
    req.extensions_mut().insert(token_identifier.clone());
    let (parts, body) = req.into_parts();
    // Attempt to find client IP from headers
    let ipv6 = try_fetch_ipv6(&parts.headers, *Constants::DEVELOPMENT_MODE).ok_or(status_response(StatusCode::FORBIDDEN, "Forbidden headers"))?.to_string();

    // Call handler, they should give us an identifier
    let response = next.run(Request::from_parts(parts,body)).await;
    if !response.status().is_success() {
        return Ok(response);
    }
    let (mut parts, body) = response.into_parts();

    // Generate our cookies
    tracing::info!("Generating new access and refresh tokens");
    let token_data = Arc::into_inner(token_identifier).ok_or_else(|| {
        tracing::error!("Failed to unwrap token_identifier arc into InternalTokenIdentifier, there exists more than one strong reference?");
        internal_server_error("Internal Server Error")
    })?.identifier.into_inner().ok_or_else(|| {
        tracing::error!("No token data set");
        internal_server_error("Internal Server Error")
    })?;
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

