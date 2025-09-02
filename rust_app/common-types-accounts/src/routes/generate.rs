use ::std::collections::HashSet;
use uuid::Uuid;
use axum::{
    extract::{
        Extension,
        State,
    },
    http::{StatusCode, HeaderMap, HeaderName, HeaderValue, header::CONTENT_TYPE},
    response::IntoResponse,
    Json
};
use diesel_async::RunQueryDsl;
use serde_json::to_string;
use deadpool_redis::redis::cmd;
use garde::Validate;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Credits::{get_total_credits, decrement_total_credits, increment_total_credits},
    Middleware::validate_access_auth::AccessTokenDescription,
    common_types::Generate::{SQSBody, GenerateOption},
    Schema::{generation, hooked_sql_types::GenerationStatus},
    Constants,
};
mod db;

use db::{RequestPayload, InsertableGeneration};

fn generate_options_to_string(ids: &[GenerateOption]) -> String {
    let mut result = String::new();
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            result.push_str(",");
        }
        result.push_str(&id.to_string());
    }
    result
}

// POST API endpoint
#[tracing::instrument(skip(access_token, appstate, user_request), fields(UserId=%access_token.user_id,request="/generate"))]
pub async fn request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Json(user_request): Json<RequestPayload>) -> Result<impl IntoResponse, ServerResponse> {
    // Payload validation
    let validation_result = user_request.validate(&user_request);
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }
    if user_request.choices.len() == 0 {
        return Err(status_response(StatusCode::BAD_REQUEST, "Cannot generate with no choices"));
    }
    if user_request.choices.len() > 4 {
        return Err(status_response(StatusCode::BAD_REQUEST, "Choices too long"));
    }

    {
        let mut encountered = HashSet::new();
        let has_duplicates = user_request.choices.iter().any(|x| !encountered.insert(x)); 
        if has_duplicates {
            return Err(status_response(StatusCode::BAD_REQUEST, "Cannot have duplicate choices"));
        }
    }

    if user_request.choices.len() > i16::MAX as usize {
        return Err(status_response(StatusCode::BAD_REQUEST, "Too many choices"));
    }
    
    let required_credits = user_request.choices.len() as i32;
    let user_id = access_token.user_id;
    let (credits, _) = get_total_credits(&appstate, user_id).await.map_err(|err| {
        tracing::error!("Failed to obtain total credits, {:?}", err);
        internal_server_error("Failed to query")
    })?;
    if required_credits as i64 > credits {
        return Err(status_response(StatusCode::BAD_REQUEST, "Insuffecient credits"));
    }
    let (next_total_credits, next_expire_at) = decrement_total_credits(appstate.clone(), user_id, required_credits, None, None).await.map_err(|err| {
        tracing::error!("Decrement total credits failed: {err}");
        internal_server_error("Unknown Error")
    })?;

    let generate_uuid = Uuid::new_v4();
    let generate_id = generate_uuid.to_string();
    let created_at = chrono::Utc::now().naive_utc();
    {
        let mut postgres_conn = match appstate.postgres.get().await {
            Ok(postgres_conn) => postgres_conn,
            Err(err) => {
                tracing::error!("Failed to open postgres connection, {err}");
                let rollback_result = increment_total_credits(appstate, user_id, required_credits, *Constants::STANDARD_CREDITS_EXPIRE_AFTER_SECS, None, None).await;
                if let Err(rollback_err) = rollback_result {
                    tracing::error!("Rollback total credits failed for {user_id}, error: {rollback_err}");
                }
                return Err(internal_server_error("Internal Service Error"));
            },
        };

        let insert_result = diesel::insert_into(generation::table)
                            .values(&InsertableGeneration {
                                userid: user_id,
                                status: GenerationStatus::Waiting,
                                createdat: created_at,
                                jobid: generate_uuid,
                                creditsused: required_credits as i16,
                                displayname: String::new(),
                                category: user_request.payload_id.to_string(),
                                options: generate_options_to_string(&user_request.choices),
                            })
                            .execute(&mut postgres_conn)
                            .await;
        
        if let Err(err) = insert_result {
            tracing::error!("Insert postgres failure: {}", err);
            let rollback_result = increment_total_credits(appstate, user_id, required_credits, *Constants::STANDARD_CREDITS_EXPIRE_AFTER_SECS, None, None).await;
            if let Err(rollback_err) = rollback_result {
                tracing::error!("Rollback total credits failed for {user_id}, error: {rollback_err}");
            }
            return Err(internal_server_error("Internal Service Error"));
        }
    }   
    let mut redis_conn = match appstate.redis.get().await {
        Ok(redis_conn) => redis_conn,
        Err(err) => {
            let rollback_result = increment_total_credits(appstate, user_id, required_credits, *Constants::STANDARD_CREDITS_EXPIRE_AFTER_SECS, None, None).await;
            if let Err(rollback_err) = rollback_result {
                tracing::error!("Rollback total credits failed for {user_id}, error: {rollback_err}");
            }
            tracing::error!("Failed to fetch Redis connection: {}", err);
            return Err(internal_server_error("Internal Service Error"));
        }
    };
    let generate_redis_key = format!("gen:job:{generate_id}");
    if let Err(err) = cmd("SET")
        .arg(&[&generate_redis_key, "Working", "EX", "1800"])
        .query_async::<_, ()>(&mut redis_conn)
        .await
    {
        let rollback_result = increment_total_credits(appstate, user_id, required_credits, *Constants::STANDARD_CREDITS_EXPIRE_AFTER_SECS, None, None).await;
        if let Err(rollback_err) = rollback_result {
            tracing::error!("Rollback total credits failed for {user_id}, error: {rollback_err}");
        }
        tracing::error!("Redis set command failed, {:?}", err);
        return Err(internal_server_error("Internal Service Error"))
    }
    let generate_payload = SQSBody {
        user_id,
        created_at,
        job_id: generate_id.clone(),
        gen_id: user_request.payload_id,
        opts: user_request.choices,
    };
    let sqs_result = appstate.sqs_client
                        .send_message()
                        .queue_url(&*Constants::GENERATE_QUEUE_URL)
                        .message_body(to_string(&generate_payload).expect("Failed to serialize generate info"))
                        .send()
                        .await;
    if let Err(sqs_err) = sqs_result {
        let rollback_result = increment_total_credits(appstate, user_id, required_credits, *Constants::STANDARD_CREDITS_EXPIRE_AFTER_SECS, None, None).await;
        if let Err(rollback_err) = rollback_result {
            tracing::error!("Rollback total credits failed for {user_id}, error: {rollback_err}");
        }
        if let Err(err) = cmd("DEL")
            .arg(&[&generate_redis_key])
            .query_async::<_, ()>(&mut redis_conn)
            .await
        {
            tracing::error!("Redis DEL command failed for rollback, {:?}", err);
        }
        tracing::error!("Failed to add generate task to queue due to {}", sqs_err.into_service_error());
        return Err(internal_server_error("Failed to add task to queue"));
    }
    
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "text/plain".parse().unwrap());
    headers.insert(HeaderName::from_static("x-set-credits"), HeaderValue::from_str(next_total_credits.to_string().as_ref()).unwrap());
    headers.insert(HeaderName::from_static("x-next-fetch"), HeaderValue::from_str(next_expire_at.and_utc().timestamp().to_string().as_ref()).unwrap());

    Ok((headers, generate_id))
}

