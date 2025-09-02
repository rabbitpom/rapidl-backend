use axum::{
    middleware::Next,
    http::{
        Request, 
        header::{
            HeaderValue, 
            ACCESS_CONTROL_ALLOW_ORIGIN,
            ACCESS_CONTROL_ALLOW_HEADERS,
            ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            ACCESS_CONTROL_EXPOSE_HEADERS,
        }
    },
    response::Response,
    body::Body,
};

use crate::{
    Response::ServerResponse,
    Constants,
};

#[tracing::instrument(skip(req, next))]
pub async fn middleware(req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let response = next.run(req).await;
    let (mut parts, body) = response.into_parts();
    parts.headers.append(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_str(&*Constants::ORIGIN_URL).unwrap());
    parts.headers.append(ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_str("GET,PUT,POST,OPTIONS,DELETE").unwrap());
    parts.headers.append(ACCESS_CONTROL_ALLOW_CREDENTIALS, HeaderValue::from_str("true").unwrap());
    parts.headers.append(ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_str("content-type,withcredentials,recaptcha").unwrap());
    parts.headers.append(ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_str("x-atk-ex,X-Atk-Ex,x-set-credits,X-Set-Credits").unwrap());
    return Ok(Response::from_parts(parts, body))
}

