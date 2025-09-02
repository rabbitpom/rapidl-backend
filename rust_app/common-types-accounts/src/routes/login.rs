use ::std::str;
use axum::{
    extract::{State, Extension},
    response::Json,
    http::StatusCode,
};
use garde::Validate;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Auth::TokenData,
    Middleware::gen_new_auth::TokenIdentifier,
    Schema::users,
    DB::UserQueryResult,
};

mod db;

use db::RequestPayload;

// POST /login API endpoint
// Body must be JSON, in format:
// {
//      email,     [Must be a valid email following HTML5 definition, maximum length 320]
//      password   [Must be ASCII, no spaces are allowed, minimum length 8, maximum length 16]
// }
// 
// 1. Attempt to deserialize to RequestPayload struct
// 2. Perform validation, handelled by garde
// 3. Hash password using bcrypt algorithm
// 4. Fetch a connection from the pool in state
// 5. Get record
// 6. Verify password
// 7. If successful respond with new cookies
// 
// Responds with OK if (7) > 0
#[tracing::instrument(skip(token_identifier, appstate, user_request), fields(email=%user_request.email,request="/login"))]
pub async fn request(Extension(token_identifier): Extension<TokenIdentifier>, State(appstate): State<AppState>, Json(user_request): Json<RequestPayload>) -> Result<(), ServerResponse> {
    tracing::info!("Processing login request");
    // Payload validation
    let validation_result = user_request.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    // Get record in database
    let user: UserQueryResult;
    {
        tracing::info!("Querying database");
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        user = users::table.filter(users::email.eq(&user_request.email)).first(&mut conn).await.map_err(|err| {
            tracing::info!("No matching email found, login request rejected, {err}");
            status_response(StatusCode::UNAUTHORIZED, "No matching credentials")
        })?;
    }
    let hash = str::from_utf8(user.bcryptpass.as_ref()).map_err(|err| {
            tracing::error!("Failed to convert hash bytes to utf8 string slice, {err}");
            internal_server_error("Internal Server Error")
        })?;

    let password_verified = bcrypt::verify(
        &user_request.password, 
        hash
    ).map_err(|err| {
        tracing::error!("Failed to verify password hash, {err}");
        internal_server_error("Internal Server Error")
    })?;

    if password_verified {
        *token_identifier.as_ref().identifier.write() = Some(TokenData {
            userid: user.userid,
            has_support_privilege: user.supportprivilege,
        });
        tracing::info!("Successfully logged in");
        return Ok(())
    }

    tracing::warn!("Could not verify password hash, rejected login");
    Err(status_response(StatusCode::UNAUTHORIZED, "No matching credentials"))
}
