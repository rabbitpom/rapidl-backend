use axum::{
    extract::{
        Extension,
        State,
        Path,
    },
    http::StatusCode,
    Json
};
use serde::Serialize;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Credits::get_total_credits,
    Middleware::validate_access_auth::AccessTokenDescription,
    Schema::users,
    DB::UserQueryResult,
};

#[derive(Serialize)]
pub struct UserInfoPayload {
    // All fields below are public
    pub username: String,
    pub user_id: i64,
    // All fields below are private
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_call: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_support_privilege: Option<bool>,
}

// GET API endpoint
// Requires valid access token
// Responds with OK and JSON in UserInfoPayload
// Some fields will be None if the desired user id
// does not match the tokens user id
#[tracing::instrument(skip(access_token, appstate), fields(UserId=%access_token.user_id,request="/get-profile"))]
pub async fn request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, _desired_user_id: Option<Path<u32>>) -> Result<Json<UserInfoPayload>, ServerResponse> {
    let desired_user_id: i64;
    if let Some(Path(_desired_user_id)) = _desired_user_id {
        desired_user_id = _desired_user_id.try_into().map_err(internal_server_error)?;
    } else {
        desired_user_id = access_token.user_id;
    }
    // Get record in database
    let user: UserQueryResult;
    {
        tracing::info!("Querying database");
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        user = users::table.filter(users::userid.eq(&desired_user_id)).first(&mut conn).await.map_err(|err| {
            tracing::info!("No matching UserId, {err}");
            status_response(StatusCode::BAD_REQUEST, "No matching UserId")
        })?;
    }

    if access_token.user_id == desired_user_id {
        let (credits, next_call) = get_total_credits(&appstate, desired_user_id).await.map_err(|err| {
            tracing::error!("Failed to obtain total credits, {:?}", err);
            internal_server_error("Failed to query")
        })?;
        Ok(Json(UserInfoPayload {
            username: user.username,
            user_id: desired_user_id,
            credits: Some(credits),
            email: Some(user.email),
            email_verified: Some(user.emailverified),
            next_call: Some(next_call.and_utc().timestamp()),
            has_support_privilege: Some(user.supportprivilege),
        }))
    } else {
        Ok(Json(UserInfoPayload {
            username: user.username,
            user_id: desired_user_id,
            credits: None,
            email: None,
            email_verified: None,
            next_call: None,
            has_support_privilege: None,
        }))
    }
}

