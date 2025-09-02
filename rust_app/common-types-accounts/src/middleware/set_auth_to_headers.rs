use axum::{
    middleware::Next,
    http::{Request, header::{HeaderValue, HeaderMap, SET_COOKIE}},
    response::Response,
    body::Body,
};
use axum_extra::extract::cookie;

use crate::{
    Response::ServerResponse,
    Auth::{is_valid_signed_token, IGNORE_SET_AUTH_TO_HEADERS},
};

fn find_atk_token(headers: &HeaderMap) -> Option<String> { 
    let jar = cookie::CookieJar::from_headers(headers);
    if let Some(access_token) = jar.get("X-ATK") {
        tracing::info!("Using already set X-ATK cookie in request header");
        let Ok(claims) = is_valid_signed_token(access_token.value()) else {
            tracing::warn!("X-ATK token provided was not valid, failed to copy X-ATK-EX");
            return None
        };
        let Some(expire) = claims.get("expire") else {
            tracing::error!("X-ATK token has no 'expire' field");
            return None
        };
        tracing::warn!("ATK-Read Expire: {expire}");
        return Some(expire.clone())
    }
    tracing::info!("Could not find any set X-ATK cookie in request header, scanning response headers for new cookies");
    for raw_cookie in headers.get_all(SET_COOKIE).iter() {
        let raw_cookie = raw_cookie.to_str();
        let Ok(raw_cookie) = raw_cookie else { continue };
        let parsed_cookie = cookie::Cookie::parse(raw_cookie);
        let Ok(cookie) = parsed_cookie else { continue };
        let (name, value) = cookie.name_value();
        if name == "X-ATK" {
            tracing::info!("Found an X-ATK token in response header");
            let Ok(claims) = is_valid_signed_token(value) else {
                tracing::warn!("Scanned X-ATK token provided was not valid, failed to copy X-ATK-EX");
                return None
            };
            let Some(expire) = claims.get("expire") else {
                tracing::error!("Scanned X-ATK token has no 'expire' field");
                return None
            };
            tracing::warn!("Scanned ATK-Read Expire: {expire}");
            return Some(expire.clone())
        }
    }
    tracing::info!("Scan came back empty handed");
    None
}

// Checks for X-ATK token
// Read the expiry and set as `x-atk-ex` header
#[tracing::instrument(skip(req, next))]
pub async fn middleware(req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let (parts, body) = req.into_parts();
    let mut atk_token = find_atk_token(&parts.headers);
    let response = next.run(Request::from_parts(parts,body)).await;
    let (mut parts, body) = response.into_parts();
    let updated_atk_token = find_atk_token(&parts.headers);
    if let Some(updated_atk_token) = updated_atk_token {
        atk_token = Some(updated_atk_token);
    }
    if parts.extensions.get::<IGNORE_SET_AUTH_TO_HEADERS>().is_some() {
        parts.headers.append("x-atk-ex", HeaderValue::from_str("0").unwrap());
    } else {
        if let Some(expire) = atk_token {
            // Copy expire into `x-atk-ex` header
            parts.headers.append("x-atk-ex", HeaderValue::from_str(&expire).unwrap());
        } 
    }
    return Ok(Response::from_parts(parts, body))
}

