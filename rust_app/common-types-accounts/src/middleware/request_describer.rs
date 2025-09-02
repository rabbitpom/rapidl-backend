use axum::{
    async_trait,
    middleware::Next,
    http::{Request, StatusCode},
    response::Response,
    body::Body,
    extract::FromRequest,
};

use crate::{
    Response::{ServerResponse, status_response},
    Constants,
};
use common_types::Ip::try_fetch_ipv6;

#[derive(Clone)]
pub struct RequestDescription {
    pub ip: String,
}

#[async_trait]
impl<S, B> FromRequest<S, B> for RequestDescription
where
    B: Send + 'static,
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request(req: Request<B>, _: &S) -> Result<Self, Self::Rejection> {
        if let Some(req) = req.extensions().get::<RequestDescription>() {
            Ok(req.clone())
        } else {
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[tracing::instrument(skip(req, next))]
pub async fn middleware(req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    let (parts, body) = req.into_parts();
    // Attempt to find client IP from headers
    let ipv6 = try_fetch_ipv6(&parts.headers, *Constants::DEVELOPMENT_MODE).ok_or(status_response(StatusCode::FORBIDDEN, "Forbidden headers"))?.to_string();
    let mut req = Request::from_parts(parts, body);
    req.extensions_mut().insert(RequestDescription {
        ip: ipv6,
    });
    // Call handler
    let response = next.run(req).await;
    Ok(response)
}

