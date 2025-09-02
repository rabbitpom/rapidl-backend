use ::std::sync::{Arc, Mutex};
use axum::{
    response::IntoResponse,
    extract::{State, Json},
    http::{StatusCode, header, HeaderMap},
};
use chrono::{Utc, TimeDelta};
use diesel::sql_types::BigInt;
use diesel::sql_query;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::RunQueryDsl;
use garde::Validate;
use base64::prelude::*;
use common_types::{
    SESContacts::{
        Request,
        RequestType,
        TopicType,
        Command,
        SendIndividual,
    },
    Token::VerifyToken,
};
use deadpool_redis::redis::pipe;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Auth::is_valid_signed_token,
    Schema::{users, allocatedcredits},
    Constants,
};
pub mod db;

use db::{RequestQuery, InsertableAllocatedCredits};

// POST /verify API endpoint
// Body must be JSON, in format:
// {
//      token,  [ASCII only]
// }
// 
// 1. Attempt to deserialize to RequestPlayoad struct
// 2. Perform validation, handelled by garde
// 3. We expect "type" and "value" in token
// 4. Undefined, has different behaviour depending on "type"
// 
// Responds with OK if nothing has gone wrong
#[tracing::instrument(skip(appstate, user_request), fields(request="/verify"))]
pub async fn request(State(appstate): State<AppState>, Json(user_request): Json<RequestQuery>) -> Result<impl IntoResponse, ServerResponse> {
    // Payload validation
    let validation_result = user_request.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let token = user_request.token;
    let Ok(claims) = is_valid_signed_token(&token) else {
        return Err(status_response(StatusCode::BAD_REQUEST, "Invalid token."))
    };
    let token_type = claims.get("type").ok_or(status_response(StatusCode::BAD_REQUEST, "Invalid token."))?;
    let token_value = claims.get("value").ok_or(status_response(StatusCode::BAD_REQUEST, "Invalid token."))?;

    match token_type.as_ref() {
        "v-confirmemail" => {
            let verify_token = serde_json::from_str::<VerifyToken>(token_value).map_err(|_| status_response(StatusCode::BAD_REQUEST, "Invalid token."))?;
            let email_bytes = BASE64_STANDARD.decode(&verify_token.email).map_err(|_| status_response(StatusCode::BAD_REQUEST,"Invalid token."))?;
            let email = String::from_utf8(email_bytes).map_err(|_| status_response(StatusCode::BAD_REQUEST,"Invalid token."))?;
            let verified_before: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

            let expireat = Utc::now().checked_add_signed(TimeDelta::new(*Constants::FREE_CREDITS_ON_VERIFY_EXPIRE_AFTER_SECS,0).unwrap()).unwrap().naive_utc();
            let user_id = verify_token.userid;
            {
                let m_verified_before = Arc::clone(&verified_before);
                let mut conn = appstate.postgres.get().await.map_err(|err| {
                    tracing::error!("Failed to fetch Postgres connection, {err}");
                    internal_server_error("Internal Service Error")
                })?;
                let _ = conn.build_transaction()
                        .read_write()
                        .serializable()
                        .run::<_, diesel::result::Error, _>(|conn| async move {
                            let verified_before: bool;
                            {
                                struct Verified {
                                    emailverified: bool,
                                }
                                impl<DB> diesel::deserialize::QueryableByName<DB> for Verified
                                where
                                    DB: diesel::backend::Backend,
                                    bool: diesel::deserialize::FromSql<diesel::dsl::SqlTypeOf<users::emailverified>, DB>,
                                {
                                    fn build<'a>(row: &impl diesel::row::NamedRow<'a, DB>) -> diesel::deserialize::Result<Self> {
                                        let emailverified = diesel::row::NamedRow::get::<diesel::dsl::SqlTypeOf<users::emailverified>, _>(row, "emailverified")?;
                                        Ok(Self{emailverified})
                                    }
                                }
                                let s_verified_before: Verified = sql_query("UPDATE users SET emailverified = TRUE WHERE userid = $1 RETURNING (SELECT emailverified FROM users WHERE userid = $1)")
                                                            .bind::<BigInt, _>(user_id)
                                                            .get_result(conn)
                                                            .await?;
                                verified_before = s_verified_before.emailverified;
                            }
                            /* If they were verified before then do not grant rewards */
                            if verified_before {
                                return Ok(())
                            }
                            /* Grant rewards */
                            let _ = diesel::insert_into(allocatedcredits::table)
                                        .values(&InsertableAllocatedCredits {
                                            credits: *Constants::FREE_CREDITS_ON_VERIFY,
                                            userid: user_id,
                                            expireat,
                                        })
                                        .execute(conn)
                                        .await?;

                            *m_verified_before.lock().unwrap() = verified_before;
                            Ok::<(),_>(())
                        }.scope_boxed()).await;
            }
            if *verified_before.lock().unwrap() {
                return Err(status_response(StatusCode::BAD_REQUEST, "You already verified this email."))
            }
            /* Clear redis caches */
            let mut redis_conn = appstate.redis.get().await.map_err(|err|{
                tracing::error!("Failed to fetch Redis connection, {err}");
                internal_server_error("Internal Service Error")
            })?;
            let credits_key = format!("user:{user_id}:cred:t");
            let expire_key = format!("user:{user_id}:cred:e");
            let pipe_result = pipe()
                .cmd("DEL").arg(&[&credits_key]).ignore()
                .cmd("DEL").arg(&[&expire_key]).ignore()
                .query_async::<_, ()>(&mut redis_conn).await;

            if let Err(err) = pipe_result {
                tracing::warn!("Failed to del multiple keys through pipeline, {err}");
                return Err(internal_server_error("Internal Service Error"))
            }

            let _ = send_verified_email_ignore_error(&appstate, &verify_token.username, email, *Constants::FREE_CREDITS_ON_VERIFY, expireat.format("%d-%m-%Y %H:%M:%S").to_string()).await;

            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
            headers.insert("x-api-resfresh-at", "0".parse().unwrap());
            Ok((
                StatusCode::OK,
                headers,
                "Your account has been successfully verified.",
            ))
        },
        "s-newsletter" => {
            let email_bytes = BASE64_STANDARD.decode(&token_value).map_err(|_| internal_server_error("Internal Server Error"))?;
            let email = String::from_utf8(email_bytes).map_err(|_| internal_server_error("Internal Server Error"))?;
            let lambda_request = Request {
                commands: Command::ActionType(RequestType::AddToMailList, TopicType::Advertising),
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
                    Err(internal_server_error("Failed to invoke lambda"))
                },
                Ok(lambda_response) => {
                    if lambda_response.status_code() < 200 && lambda_response.status_code() >= 300 {
                        tracing::error!("Email lambda experienced an error: {}", lambda_response.function_error().unwrap_or(&format!("No error was returned in payload but status code is outside OK range: {}", lambda_response.status_code())));
                        return Err(internal_server_error("Internal Server Error"));
                    }
                    let mut headers = HeaderMap::new();
                    headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
                    Ok((
                        StatusCode::OK,
                        headers,
                        "You have been successfully subscribed to our mail list.",
                    ))
                },
            }
        },
        _ => Err(status_response(StatusCode::BAD_REQUEST, "Invalid token.")),
    }
}

async fn send_verified_email_ignore_error(appstate: &AppState, username: &str, email: String, credits: i32, expireat: String) -> Result<(), ()> {
    let template = SendIndividual {
        template_name: "verifiedrewardtemplate".to_string(),
        template_data: format!(r#"{{ "username": "{username}", "credits": "{credits}", "expireat": "{expireat}" }}"#),
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
