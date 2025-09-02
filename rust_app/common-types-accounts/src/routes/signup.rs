use ::std::collections::BTreeMap;
use ::std::sync::Arc;
use axum::{
    extract::{State, Json, Extension},
    http::StatusCode,
};
use garde::Validate;
use zxcvbn::zxcvbn;
use jwt::SignWithKey;
use base64::prelude::*;
use diesel_async::RunQueryDsl;
use common_types::{
    SESContacts::{
        Request,
        SendIndividual,
        Command,
    },
    Token::VerifyToken,
};

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Email::verify_email,
    Middleware::gen_new_auth::TokenIdentifier,
    Schema::users,
    Auth::TokenData,
    Constants,
};

mod db;

use db::{RequestPayload, User};

// POST /sign-up API endpoint
// Body must be JSON, in format:
// {
//      username,  [Must be ASCII Alphanumeric, minimum length 3, maximum length 16]
//      email,     [Must be a valid email following HTML5 definition, maximum length 320]
//      password   [Must be ASCII, no spaces are allowed, minimum length 8, maximum length 16]
// }
// 
// 1. Attempt to deserialize to RequestPlayoad struct
// 2. Perform validation, handelled by garde
// 3. Perform password strength estimation using zxcvbn
// 4. Reject if password strength is lower than 3
// 5. Hash password using bcrypt algorithm
// 6. Fetch a connection from the pool in state
// 7. Perform an INSERT IGNORE INTO query
// 
// Responds with OK if (7) > 0
#[tracing::instrument(skip(token_identifier, appstate, user_request), fields(username=%user_request.username,email=%user_request.email,request="/sign-up"))]
pub async fn request(Extension(token_identifier): Extension<TokenIdentifier>, State(appstate): State<AppState>, Json(user_request): Json<RequestPayload>) -> Result<(), ServerResponse> {
    tracing::info!("Processing sign up request");
    // Payload validation
    let validation_result = user_request.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    // Password estimation
    let zxcvbn_proccessed_password = zxcvbn(&user_request.password, &[&user_request.username, &user_request.email]).map_err(internal_server_error)?;
    if zxcvbn_proccessed_password.score() <= 2 {
        tracing::info!("Password too weak, rejected request");
        return Err(status_response(StatusCode::BAD_REQUEST, "Password is too weak"))
    }

    // Verify email
    if !verify_email(Arc::clone(&appstate), &user_request.email).await {
        tracing::warn!("Provided email failed to pass verification check");
        return Err(status_response(StatusCode::BAD_REQUEST, "Invalid email"))
    }

    // Hash password first to avoid timing based attack
    tracing::info!("Hashing password");
    let hashed = bcrypt::hash(&user_request.password, Constants::HASH_COST).map_err(internal_server_error)?;

    // Adding the user
    tracing::info!("Querying database");
    let mut conn = appstate.postgres.get().await.map_err(|err| {
        tracing::error!("Failed to fetch Postgres connection, {err}");
        internal_server_error("Internal Service Error")
    })?;

    let new_user_id = diesel::insert_into(users::table)
        .values(&User {
                username: &user_request.username,
                email: &user_request.email,
                emailverified: false,
                bcryptpass: hashed.as_bytes(),
            })
        .on_conflict_do_nothing()
        .returning(users::userid)
        .get_result(&mut conn).await.map_err(|err| {
            tracing::error!("Conflicting emails found, rejecting request, {err}");
            status_response(StatusCode::CONFLICT, format!("{} is already in use", &user_request.email))
        })?;

    *token_identifier.as_ref().identifier.write() = Some(TokenData {
        userid: new_user_id,
        has_support_privilege: false, // change manually in DB if needed
    });
    tracing::info!("Successfully created account, sending out email now");

    let _ = send_welcome_email_ignore_error(&appstate, new_user_id, &user_request.username, user_request.email.clone()).await;

    Ok(())
}

async fn send_welcome_email_ignore_error(appstate: &AppState, userid: i64, username: &str, email: String) -> Result<(), ()> {
    let jwt_key = &*Constants::JWT_KEY;
    let b64_email = BASE64_STANDARD.encode(&email);
    let token = VerifyToken {
        username: username.to_string(),
        email: b64_email,
        userid,
    };
    let serialized_token = serde_json::to_string(&token).unwrap();
    let mut verify_claims = BTreeMap::new();
    verify_claims.insert("type", "v-confirmemail");
    verify_claims.insert("value", &serialized_token);
    let verify_token = verify_claims.sign_with_key(jwt_key).map_err(|_| ())?;

    // SAFETY: Safe to use username directly as its guaranteed to be alphanumeric only
    let template = SendIndividual {
        template_name: "welcometemplate".to_string(),
        template_data: format!(r#"{{ "username": "{}", "verifyurl": "{}" }}"#, username, format!("{}/verify?token={verify_token}", &*Constants::ORIGIN_URL)),
    };
    let lambda_request = Request {
        commands: Command::SendIndividual(template),
        email,
    };

    let lambda_response = appstate.lambda_client
                            .invoke()
                            .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                            .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                            .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                            .send()
                            .await;
    
    match lambda_response {
        Err(err) => {
            tracing::error!("Failed to invoke lambda, err: {}", err);
            Err(())
        },
        Ok(lambda_response) => {
            if lambda_response.status_code() < 200 && lambda_response.status_code() >= 300 {
                tracing::error!("Email lambda experienced an error: {}", lambda_response.function_error().unwrap_or(&format!("No error was returned in payload but status code is outside OK range: {}", lambda_response.status_code())));
                return Err(());
            }
            Ok(())
        },
    }
}
