use ::std::collections::BTreeMap;
use axum::{
    extract::{State, Extension},
    http::StatusCode,
};
use jwt::SignWithKey;
use base64::prelude::*;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use deadpool_redis::redis::cmd;
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
    Middleware::validate_access_auth::AccessTokenDescription,
    Schema::users,
    Constants,
    DB::UserQueryResult,
};

// PUT /send-verify API endpoint
#[tracing::instrument(skip(access_token, appstate), fields(user_id=%access_token.user_id,request="/send-verify"))]
pub async fn request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>) -> Result<(), ServerResponse> {
    {
        let mut redis_conn = appstate.redis.get().await.map_err(|err|{
            tracing::error!("Failed to fetch Redis connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        
        /* Check redis cache if this request has already been served in the last
         * SEND_VERIFICATION_COOLDOWN */
        let redis_key = format!("user:{}:verify", access_token.user_id);
        {
            let previous_sent = match cmd("GET").arg(&[&redis_key]).query_async::<_, Option<String>>(&mut redis_conn).await {
                Ok(x) => x,
                Err(err) => {
                    tracing::error!("Redis GET command failed, {:?}", err);
                    return Err(internal_server_error("Internal Service Error"));
                }
            };
            if let Some(_) = previous_sent {
                return Err(status_response(StatusCode::TOO_MANY_REQUESTS, "You have already submitted this request. Please try again in a few minutes"));
            }
        }

        /* Mark in redis cache */
        {
            if let Err(err) = cmd("SET")
                .arg(&[&redis_key, "true", "EX", &(*Constants::SEND_VERIFICATION_COOLDOWN).to_string()])
                .query_async::<_, ()>(&mut redis_conn)
                .await
            {
                tracing::error!("Redis set command failed, {:?}", err);
                return Err(internal_server_error("Internal Service Error"))
            }
        }
    }

    // Query database and check if they're really not verified (also get email)
    let user: UserQueryResult;
    {
        tracing::info!("Querying database");
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        user = users::table.filter(users::userid.eq(&access_token.user_id)).first(&mut conn).await.map_err(|_| {
            tracing::info!("No matching userid found for {}, could not verify.", access_token.user_id);
            status_response(StatusCode::UNAUTHORIZED, "No matching credentials")
        })?;
    }

    if user.emailverified {
        return Err(status_response(StatusCode::CONFLICT, "You already have a verified email"));
    }

    // Send the email
    let jwt_key = &*Constants::JWT_KEY;
    let b64_email = BASE64_STANDARD.encode(&user.email);
    let token = VerifyToken {
        username: user.username,
        email: b64_email,
        userid: access_token.user_id,
    };
    let serialized_token = serde_json::to_string(&token).unwrap();
    let mut verify_claims = BTreeMap::new();
    verify_claims.insert("type", "v-confirmemail");
    verify_claims.insert("value", &serialized_token);
    let Ok(verify_token) = verify_claims.sign_with_key(jwt_key) else {
        tracing::error!("Failed to sign email verification for {}", access_token.user_id);
        return Err(internal_server_error("Failed to sign email verification token"));
    };

    // SAFETY: Safe to use username directly as its guaranteed to be alphanumeric only
    let template = SendIndividual {
        template_name: "verifyemailtemplate".to_string(),
        template_data: format!(r#"{{ "verifyurl": "{}" }}"#, format!("{}/verify?token={verify_token}", &*Constants::ORIGIN_URL)),
    };
    let lambda_request = Request {
        commands: Command::SendIndividual(template),
        email: user.email,
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
            Err(internal_server_error("Internal Server Error"))
        },
        Ok(lambda_response) => {
            if lambda_response.status_code() < 200 && lambda_response.status_code() >= 300 {
                tracing::error!("Email lambda experienced an error: {}", lambda_response.function_error().unwrap_or(&format!("No error was returned in payload but status code is outside OK range: {}", lambda_response.status_code())));
                return Err(internal_server_error("Internal Server Error"));
            }
            Ok(())
        },
    }
}
