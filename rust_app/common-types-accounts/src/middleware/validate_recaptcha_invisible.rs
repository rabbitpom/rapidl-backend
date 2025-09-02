use ::std::collections::HashSet;
use axum::{
    middleware::Next,
    extract::State,
    http::{Request, StatusCode},
    response::Response,
    body::Body,
};
use serde::Deserialize;

use crate::{
    Response::{ServerResponse, status_response, internal_server_error},
    State::AppState,
    Constants,
};

#[derive(Debug, Deserialize)]
pub struct RecaptchaResponse {
    pub success: bool,
    #[serde(rename="error-codes")]
    pub error_codes: Option<HashSet<String>>
}

#[tracing::instrument(skip(appstate, req, next))]
pub async fn middleware(State(appstate): State<AppState>, req: Request<Body>, next: Next<Body>) -> Result<Response, ServerResponse> {
    if *Constants::DEVELOPMENT_MODE {
        tracing::warn!("Skipped RECAPTCHA check as development mode is enabled");
        return Ok(next.run(req).await);
    }
    let (parts, body) = req.into_parts();

    let captcha = parts.headers.get("recaptcha")
        .ok_or(status_response(StatusCode::BAD_REQUEST, "No RECAPTCHA header"))?
        .to_str().map_err(|err| { 
            tracing::error!("Failed to read RECAPTCHA, {err}");
            internal_server_error("Internal Server Error")
        })?;

    let captcha_response = appstate.http_client.post("https://www.google.com/recaptcha/api/siteverify")
        .form(&[
            ("secret", Constants::GOOGLE_INVISIBLE_RECAPTCHA_SECRET_KEY.as_ref()),
            ("response", captcha),
        ]).send().await.map_err(|err| {
        tracing::error!("Failed to send POST request to RECAPTCHA for verification, {err}");
        internal_server_error("Internal Server Error")
    })?;
    let captcha_response = captcha_response.json::<RecaptchaResponse>().await.map_err(|err| { 
            tracing::error!("Failed to deserialize response, {err}");
            internal_server_error("Internal Server Error")
        })?;
    
    if !captcha_response.success {
        tracing::warn!("Request dropped due to Google ReCAPTCHA token being marked as a robot, full response: {:?}", captcha_response);
        return Err(status_response(StatusCode::FORBIDDEN, ""));
    }

    let response = next.run(Request::from_parts(parts,body)).await;
    return Ok(response);
}

