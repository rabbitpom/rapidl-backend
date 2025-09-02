use ::std::collections::BTreeMap;
use ::std::sync::Arc;
use axum::{
    extract::State,
    response::Json,
    http::StatusCode,
};
use garde::Validate;
use deadpool_redis::redis::cmd;
use jwt::SignWithKey;
use base64::prelude::*;
use common_types::SESContacts::{
    Response,
    RequestType,
    TopicType,
    Request,
    SendIndividual,
    Command,
};

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Email::verify_email,
    Constants,
};

mod db;

use db::RequestPayload;

// POST /subscribe/newsletter API endpoint
// Body must be JSON, in format:
// {
//      username,  [Must be ASCII Alphanumeric, minimum length 3, maximum length 16]
// }
// 
// 1. Attempt to deserialize to RequestPlayoad struct
// 2. Perform validation, handelled by garde
// 3. Fetch redis connection, then base64 encode email
// 4. Check if the encoded email exists in redis cache, if it does then reject email
// 5. If it does not then check if email is not in mail list
// 6. If it isn't then construct a subscription token and send email to user
// 7. If sent successfully, add encoded email to redis cache to expire later
// 
// Responds with OK if nothing has gone wrong
#[tracing::instrument(skip(appstate, user_request), fields(email=%user_request.email,request="/subscribe/newsletter"))]
pub async fn request(State(appstate): State<AppState>, Json(user_request): Json<RequestPayload>) -> Result<(), ServerResponse> {
    let validation_result = user_request.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    // Verify email
    if !verify_email(Arc::clone(&appstate), &user_request.email).await {
        tracing::warn!("Provided email failed to pass verification check");
        return Err(status_response(StatusCode::BAD_REQUEST, "Invalid email"))
    }

    let mut redis_conn = appstate.redis.get().await.map_err(|err|{
        tracing::error!("Failed to fetch Redis connection, {err}");
        internal_server_error("Internal Service Error")
    })?;
    
    let b64_email = BASE64_STANDARD.encode(&user_request.email);
    /* Check redis cache if this request has already been served in the last
     * SUBSCRPTION_NEWSLETTER_COOLDOWN */
    {
        let previous_sent = match cmd("GET").arg(&[&b64_email]).query_async::<_, Option<String>>(&mut redis_conn).await {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("Redis GET command failed, {:?}", err);
                return Err(internal_server_error("Internal Service Error"));
            }
        };
        if let Some(_) = previous_sent {
            return Err(status_response(StatusCode::TOO_MANY_REQUESTS, "You have already submitted this email. Please try again later."));
        }
    }

    /* Is this email already in the mail list? */
    let lambda_request = Request {
        commands: Command::ActionType(RequestType::IsInMailList, TopicType::Advertising),
        email: user_request.email.clone(),
    };
    let lambda_response = appstate.lambda_client
                            .invoke()
                            .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                            .invocation_type(aws_sdk_lambda::types::InvocationType::RequestResponse)
                            .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                            .send()
                            .await;
    match lambda_response {
        Err(err) => {
            tracing::error!("Failed to invoke lambda, err: {}", err);
            return Err(internal_server_error("Failed to invoke lambda"));
        },
        Ok(lambda_response) => {
            if lambda_response.status_code() < 200 && lambda_response.status_code() >= 300 {
                tracing::error!("Email lambda experienced an error: {}", lambda_response.function_error().unwrap_or(&format!("No error was returned in payload but status code is outside OK range: {}", lambda_response.status_code())));
                return Err(internal_server_error("Internal Server Error"));
            }
            let Some(blob) = lambda_response.payload() else {
                tracing::error!("Email lambda did not return any blob response");
                return Err(internal_server_error("Internal Server Error"));
            };
            let mail_response = serde_json::from_slice::<Response>(blob.as_ref()).map_err(|err| {
                tracing::error!("Failed to deserialize blob from email lambda: {}", err);
                internal_server_error("Internal Server Error")
            })?;
            let Some(is_email_in_mail_list) = mail_response.is_email_in_mail_list else {
                tracing::error!("Email lambda did not return expected is_email_in_mail_list field");
                return Err(internal_server_error("Internal Server Error"));
            };
            if is_email_in_mail_list {
                return Err(status_response(StatusCode::CONFLICT, "You have already subscribed to our newsletter."));
            }
        },
    }

    /* Create a subscription token */
    let jwt_key = &*Constants::JWT_KEY;
    let mut subscription_claims = BTreeMap::new();
    subscription_claims.insert("type", "s-newsletter");
    subscription_claims.insert("value", &b64_email);
    let subscription_token = subscription_claims.sign_with_key(jwt_key).map_err(|err| {
        tracing::error!("Failed to sign subscription token, err: {}", err);
        internal_server_error("Internal Server Error")
    })?;
    /* Invoke lambda to send email */
    let template = SendIndividual {
        template_name: "newsletterconfirmationtemplate".to_string(),
        template_data: format!(r#"{{ "confirmationurl": "{}" }}"#, format!("{}/verify?token={subscription_token}", &*Constants::ORIGIN_URL)),
    };
    let lambda_request = Request {
        commands: Command::SendIndividual(template),
        email: user_request.email.clone(),
    };
    let lambda_response = appstate.lambda_client
                            .invoke()
                            .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                            .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                            .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                            .send()
                            .await;
    /* If all goes well we can update redis cache to block future requests */
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
            if let Err(err) = cmd("SET")
                .arg(&[&b64_email, "true", "EX", &(*Constants::SUBSCRPTION_NEWSLETTER_COOLDOWN).to_string()])
                .query_async::<_, ()>(&mut redis_conn)
                .await
            {
                tracing::error!("Redis set command failed, {:?}", err);
                return Err(internal_server_error("Internal Service Error"))
            }
            Ok(())
        },
    }
}
 
