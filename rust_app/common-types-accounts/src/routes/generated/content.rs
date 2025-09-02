use ::std::collections::HashSet;
use axum::{
    extract::{
        Extension,
        State,
        Query,
    },
    http::StatusCode,
    Json
};
use chrono::NaiveDateTime;
use serde::Serialize;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::RunQueryDsl;
use aws_sdk_s3::operation::get_object::GetObjectError;
use deadpool_redis::redis::cmd;
use garde::Validate;
use base64::prelude::*;

use crate::{
    Schema::{generation, hooked_sql_types::GenerationStatus},
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Middleware::validate_access_auth::AccessTokenDescription,
    common_types::Generate::{SQSBody, str_to_generation_options, str_to_generation_id},
    Constants,
};

#[derive(Serialize)]
pub struct GenerationContent {
    status: GenerationStatus,
    content: Option<GenerationBlob>,
}

#[derive(Serialize)]
pub struct GenerationBlob {
    blob: String,
    createdat: NaiveDateTime,
    finishedon: NaiveDateTime,
    displayname: String,
    options: String,
    category: String,
    creditsused: i16,
}

#[derive(Serialize)]
pub struct GenerationNoContent {
    status: GenerationStatus,
    createdat: NaiveDateTime,
    finishedon: Option<NaiveDateTime>,
    displayname: String,
    options: String,
    category: String,
    creditsused: i16,
    jobid: uuid::Uuid,
}

mod db;
use db::{GenerationNameChangeQuery, GenerationQuery, GenerationBatchQuery, GenerationSelectable, GenerationSelectableWithJobId};

// POST API endpoint (retry)
#[tracing::instrument(skip(access_token, appstate, query), fields(UserId=%access_token.user_id,request="/generated/content[post]",id=%query.id))]
pub async fn post_retry_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(query): Query<GenerationQuery>) -> Result<(), ServerResponse> {
    let validation_result = query.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let uuid_job_id = uuid::Uuid::try_parse(&query.id).map_err(|_| status_response(StatusCode::BAD_REQUEST, "Invalid ID"))?;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        let appstate = appstate.clone();
        let _ = conn.build_transaction()
                        .repeatable_read()
                        .run::<Result<&'static str, ServerResponse>, diesel::result::Error, _>(|conn| async move {
                            let generation_details = generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id)))
                                                                        .select(GenerationSelectable::as_select())
                                                                        .for_update()
                                                                        .first(conn)
                                                                        .await?;
                            match generation_details.status {
                                GenerationStatus::Failed => (),
                                _ => return Ok(Err(status_response( StatusCode::CONFLICT, "You cannot retry a generation that has not failed" ))),
                            }

                            let (Ok(gen_id), Ok(gen_opts)) = (str_to_generation_id(&generation_details.category), str_to_generation_options(&generation_details.options)) else {
                                tracing::error!("Generation {uuid_job_id} for {} has bad category/options, failed to serialize", access_token.user_id);
                                return Ok(Err(internal_server_error("Bad record data")));
                            };

                            let updated_rows = diesel::update(generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id))))
                                                        .set(generation::status.eq(GenerationStatus::Waiting))
                                                        .execute(conn)
                                                        .await?;
                            if updated_rows == 0 {
                                return Ok(Err(internal_server_error("Updated 0 rows")));
                            }

                            let generate_payload = SQSBody {
                                gen_id,
                                user_id: access_token.user_id,
                                created_at: generation_details.createdat,
                                job_id: uuid_job_id.to_string(),
                                opts: gen_opts,
                            };
                            let sqs_result = appstate.sqs_client
                                                .send_message()
                                                .queue_url(&*Constants::GENERATE_QUEUE_URL)
                                                .message_body(serde_json::to_string(&generate_payload).map_err(|x| {
                                                    tracing::error!("Failed to serialize SQSBody for generation retry: {x}"); 
                                                    diesel::result::Error::RollbackTransaction
                                                })?)
                                                .send()
                                                .await;
                            if let Err(sqs_err) = sqs_result {
                                tracing::error!("Failed to add retry generate task to queue due to {}", sqs_err.into_service_error());
                                return Err(diesel::result::Error::RollbackTransaction);
                            }

                            // redis cache dont really matter
                            if let Ok(mut redis_conn) = appstate.redis.get().await {
                                let generate_redis_key = format!("gen:job:{uuid_job_id}");
                                let _ = cmd("SET")
                                    .arg(&[&generate_redis_key, "Working", "EX", "1800"])
                                    .query_async::<_, ()>(&mut redis_conn)
                                    .await;
                            }

                            Ok(Ok("Success"))
                        }.scope_boxed())
                        .await.map_err(|err| {
                            tracing::error!("Transaction error: {err}");
                            internal_server_error("Internal Service Error")
                        })?;
    }

    Ok(())
}

// POST API endpoint
#[tracing::instrument(skip(access_token, appstate, query), fields(UserId=%access_token.user_id,request="/generated/content[post]",id=%query.id,displayname=%query.displayname))]
pub async fn post_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(query): Query<GenerationNameChangeQuery>) -> Result<(), ServerResponse> {
    let validation_result = query.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let uuid_job_id = uuid::Uuid::try_parse(&query.id).map_err(|_| status_response(StatusCode::BAD_REQUEST, "Invalid ID"))?;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        let updated = diesel::update(generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id))))
                            .set(generation::displayname.eq(&query.displayname))
                            .execute(&mut conn)
                            .await.map_err(|err| {
                                tracing::error!("Failed, Postgres operation, could not change generation name for {}, {uuid_job_id}, because {err}", access_token.user_id);
                                internal_server_error("Internal Service Error")
                            })?;
        if updated == 0 {
            return Err(internal_server_error("No records changed"));
        }
    }

    Ok(())
}

// GET BATCH API endpoint
#[tracing::instrument(skip(access_token, appstate, query), fields(UserId=%access_token.user_id,request="/generated/content/batch"))]
pub async fn get_batch_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(query): Query<GenerationBatchQuery>) -> Result<Json<Vec<GenerationNoContent>>, ServerResponse> {
    if query.ids.is_empty() {
        return Ok(Json(Vec::new()));
    }
    if query.ids.len() > 10 {
        return Err(status_response(StatusCode::BAD_REQUEST, "Too many ids"));
    }
    let validation_result = query.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let uuid_job_ids: Result<Vec<uuid::Uuid>, _> = query.ids.into_iter().map(|s| {
                                            uuid::Uuid::try_parse(&s)
                                                .map_err(|_| status_response(StatusCode::BAD_REQUEST, "Invalid ID"))
                                        }).collect();
    let uuid_job_ids = match uuid_job_ids {
        Ok(ids) => ids,
        Err(err) => {
            return Err(err);
        },
    };

    let previous_size = uuid_job_ids.len();

    let uuid_job_ids: Vec<uuid::Uuid> = {
        let mut set = HashSet::new();
        uuid_job_ids.into_iter().filter(|uuid| set.insert(*uuid)).collect()
    };

    if previous_size != uuid_job_ids.len() {
        return Err(status_response(StatusCode::BAD_REQUEST, "Cannot have duplicate ids"));
    }

    let generation_details: Vec<GenerationSelectableWithJobId>;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        let filter = generation::userid.eq(access_token.user_id).and(generation::jobid.eq_any(&uuid_job_ids));
        generation_details = generation::table.filter(filter)
                                                .select(GenerationSelectableWithJobId::as_select())
                                                .get_results(&mut conn)
                                                .await.map_err(|err| {
                                                    tracing::error!("Failed to query for generation details, for ids {:?}, error: {err}", &uuid_job_ids);
                                                    internal_server_error("Internal Service Error")
                                                })?;
    }

    let returned_details: Vec<GenerationNoContent> = generation_details.into_iter().map(|s| {
                                                                GenerationNoContent {
                                                                    status: s.status,
                                                                    createdat: s.createdat,
                                                                    finishedon: s.finishedon,
                                                                    displayname: s.displayname,
                                                                    options: s.options,
                                                                    category: s.category,
                                                                    creditsused: s.creditsused,
                                                                    jobid: s.jobid,
                                                                }
                                                            }).collect();
    Ok(Json(returned_details))
}

// GET API endpoint
#[tracing::instrument(skip(access_token, appstate, query), fields(UserId=%access_token.user_id,request="/generated/content[get]",id=%query.id))]
pub async fn get_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(query): Query<GenerationQuery>) -> Result<Json<GenerationContent>, ServerResponse> {
    let validation_result = query.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let uuid_job_id = uuid::Uuid::try_parse(&query.id).map_err(|_| status_response(StatusCode::BAD_REQUEST, "Invalid ID"))?;

    {
        let mut redis_conn = appstate.redis.get().await.map_err(|err|{
            tracing::error!("Failed to fetch Redis connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        let generate_redis_key = format!("gen:job:{uuid_job_id}");
        let cached_status = match cmd("GET").arg(&[&generate_redis_key]).query_async::<_, Option<String>>(&mut redis_conn).await {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("Redis GET command failed, {:?}", err);
                return Err(internal_server_error("Internal Service Error"));
            }
        };
        if let Some(cached_status) = cached_status {
            match cached_status.as_ref() {
                "Failed" => return Ok(Json(GenerationContent { status: GenerationStatus::Failed, content: None })),
                "Working" => return Ok(Json(GenerationContent { status: GenerationStatus::Working, content: None })),
                "Deleting" => return Ok(Json(GenerationContent { status: GenerationStatus::Deleting, content: None })),
                "Waiting" => return Ok(Json(GenerationContent { status: GenerationStatus::Waiting, content: None })),
                "Success" => (),
                _ => tracing::warn!("Unexpected cached status: {cached_status}"),
            }
        }
    }

    let generation_details: GenerationSelectable;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        generation_details = generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id)))
                                                                        .select(GenerationSelectable::as_select())
                                                                        .first(&mut conn)
                                                                        .await.map_err(|err| {
                                                                            tracing::error!("Failed to query for generation details, id {uuid_job_id}, error: {err}");
                                                                            internal_server_error("Internal Service Error")
                                                                        })?;
    }

    match generation_details.status {
        GenerationStatus::Working | GenerationStatus::Failed | GenerationStatus::Deleting | GenerationStatus::Waiting => return Ok(Json(GenerationContent { status: generation_details.status, content: None })),
        GenerationStatus::Success => (),
    }

    let Some(finishedon) = generation_details.finishedon else {
        tracing::error!("Generation status is successful yet there is no finishedon timestamp for {uuid_job_id}");
        return Err(internal_server_error("Unexpected error"));
    };


    let get_result = appstate.s3_client
                            .get_object()
                            .bucket(&*Constants::GENERATED_BUCKET_NAME)
                            .key(format!("{uuid_job_id}.rapidl.gz"))
                            .send()
                            .await;

    let blob = match get_result {
        Err(sdk_err) => {
            match sdk_err.into_service_error() {
                GetObjectError::NoSuchKey(_) => {
                    tracing::error!("No such key for job {uuid_job_id} but job was marked as success?");
                    return Err(internal_server_error("Unexpected error"));
                },
                GetObjectError::InvalidObjectState(_) => {
                    tracing::error!("Object {uuid_job_id}.rapidl.gz has an invalid state?");
                    return Err(internal_server_error("Object has invalid state"));
                },
                err @ _ => {
                    tracing::error!("Handelled service error: {err}");
                    return Err(internal_server_error("Bad error"));
                },
            }
        },
        Ok(blob) => blob,
    };
    let bytes = blob.body.collect().await.map(|data| data.into_bytes()).map_err(|err| {
                                                                                tracing::error!("Bytestream error: {err}");
                                                                                internal_server_error("Failed to read object")
                                                                            })?;
    let data_blob = BASE64_STANDARD.encode(bytes);

    Ok(Json(GenerationContent {
        status: GenerationStatus::Success,
        content: Some( GenerationBlob {
            finishedon,
            blob: data_blob,
            createdat: generation_details.createdat,
            displayname: generation_details.displayname,
            options: generation_details.options,
            category: generation_details.category,
            creditsused: generation_details.creditsused,
        }),
    }))
}

// DELETE API endpoint
#[tracing::instrument(skip(access_token, appstate, query), fields(UserId=%access_token.user_id,request="/generated/content[delete]",id=%query.id))]
pub async fn delete_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(query): Query<GenerationQuery>) -> Result<&'static str, ServerResponse> {
    let validation_result = query.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let uuid_job_id = uuid::Uuid::try_parse(&query.id).map_err(|_| status_response(StatusCode::BAD_REQUEST, "Invalid ID"))?;

    let ret;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        let appstate = appstate.clone();
        ret = conn.build_transaction()
                        .read_committed()
                        .serializable()
                        .run::<Result<&'static str, ServerResponse>, diesel::result::Error, _>(|conn| async move {
                            let generation_details: GenerationSelectable = match generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id)))
                                                                            .select(GenerationSelectable::as_select())
                                                                            .first(conn)
                                                                            .await {
                                                                                Ok(data) => data,
                                                                                Err(err) => match err {
                                                                                    diesel::result::Error::NotFound => return Ok(Err(status_response(StatusCode::NOT_FOUND, "Content not found"))),
                                                                                    _ => return Err(err),
                                                                                },
                                                                            };
                            // If generation status is Working we cannot cancel it
                            if let GenerationStatus::Working = generation_details.status {
                                return Ok(Err(status_response(StatusCode::LOCKED, "Cannot cancel a generation")));
                            }
                            // If generation status is Waiting then we'll flag this to be deleted
                            // later (by the generator function)
                            if let GenerationStatus::Waiting = generation_details.status {
                                let set_records = diesel::update(generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id))))
                                                            .set(generation::status.eq(GenerationStatus::Deleting))
                                                            .execute(conn)
                                                            .await?;
                                if set_records == 0 {
                                    tracing::error!("Somehow marked no records to be deleted for user {} and job {uuid_job_id}", access_token.user_id);
                                    return Err(diesel::result::Error::RollbackTransaction);
                                }

                                return Ok(Ok("Deleting"));
                            }
                            // If generation status is Success we will delete object from S3
                            if let GenerationStatus::Success = generation_details.status {
                                let _ = appstate.s3_client.delete_object()
                                                            .bucket(&*Constants::GENERATED_BUCKET_NAME)
                                                            .key(format!("{uuid_job_id}.rapidl.gz"))
                                                            .send()
                                                            .await.map_err(|err| {
                                                                match err.as_service_error() {
                                                                    err @_ => tracing::error!("Unhandelled S3 DeleteObjectError: {:?}", err),
                                                                }
                                                                diesel::result::Error::RollbackTransaction
                                                            })?;
                            }
                            // Delete record
                            let deleted_records = diesel::delete(generation::table.filter(generation::userid.eq(access_token.user_id).and(generation::jobid.eq(uuid_job_id)))).execute(conn).await?;
                            if deleted_records == 0 {
                                tracing::error!("Somehow deleted no records for user {} and job {uuid_job_id}", access_token.user_id);
                                return Err(diesel::result::Error::RollbackTransaction);
                            }
                            Ok(Ok("Success"))
                        }.scope_boxed())
                        .await
                        .map_err(|err| {
                            tracing::error!("Transaction error: {err}");
                            internal_server_error("Internal Service Error")
                        })?;
    }

    if let Ok(_) = ret {
        // Delete from cache if possible, ignore any error, the keys have a short TTL anyway
        if let Ok(mut redis_conn) = appstate.redis.get().await {
            let _ = cmd("DEL")
                    .arg(&[&format!("gen:job:{uuid_job_id}")])
                    .query_async::<_, ()>(&mut redis_conn)
                    .await;
        }
    }

    ret
}
