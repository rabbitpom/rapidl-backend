use ::std::sync::Arc;
use ::std::io::Write;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::RunQueryDsl;
use deadpool_redis::redis::cmd;
use chrono::NaiveDateTime;
use common_types::Generate::SQSBody;
use common_types_accounts::Schema::{generation, hooked_sql_types::GenerationStatus};
use serde::Serialize;
use rmp_serde::Serializer;
use flate2::{Compression, write::GzEncoder};

mod questionstacker;
mod engine;
mod helper;
mod checker;
mod oncelabel;
mod paper;
mod question;
mod formatter;

pub enum GenerationError {
    RedisConnectionFailure,
    PostgresConnectionFailure,
    RedisCommandFailure,
    PostgresCommandFailure,
    InternalGenerationFailure(engine::GenerateFailure),
    SerializeError,
    UUIDParseFailure,
    S3PutError,
    CompressionError,
    DeleteImmediately,
}

#[derive(Insertable)]
#[diesel(table_name = generation)]
#[allow(non_snake_case)]
struct InsertableGeneration {
    userid: i64,
    status: GenerationStatus,
    createdat: NaiveDateTime,
    finishedon: NaiveDateTime,
    jobid: uuid::Uuid,
}

pub async fn generate(appstate: common_types_accounts::MinimalState::AppState, s3_client: Arc<aws_sdk_s3::Client>, generate_options: SQSBody) -> Result<(), GenerationError> {
    let uuid_job_id = uuid::Uuid::try_parse(&generate_options.job_id);
    let Ok(uuid_job_id) = uuid_job_id else {
        return Err(GenerationError::UUIDParseFailure);
    };

    {
        let mut postgres_conn = appstate.postgres.get()
                                .await.map_err(|err| {
                                    tracing::error!("Failed to open postgres connection, {err}");
                                    GenerationError::PostgresConnectionFailure
                                })?;
        let ret = postgres_conn.build_transaction()
                        .read_write()
                        .serializable()
                        .run::<Result<(), GenerationError>, diesel::result::Error, _>(|conn| async move {
                            let status: GenerationStatus = generation::table.filter(generation::userid.eq(generate_options.user_id).and(generation::jobid.eq(uuid_job_id)))
                                                .select(generation::status)
                                                .for_update()
                                                .first(conn)
                                                .await?;
                            match status {
                                GenerationStatus::Success => { 
                                    tracing::error!("Generation {uuid_job_id} is in success state already?"); 
                                    return Ok(Err(GenerationError::DeleteImmediately));
                                },
                                GenerationStatus::Failed => { 
                                    tracing::warn!("Generation {uuid_job_id} is in failed state but attempted to generate?");
                                    return Ok(Err(GenerationError::DeleteImmediately));
                                },
                                GenerationStatus::Deleting => {
                                    return Ok(Err(GenerationError::DeleteImmediately));
                                },
                                GenerationStatus::Working => return Ok(Ok(())),
                                GenerationStatus::Waiting => (),
                            }
                            let _ = diesel::update(generation::table.filter(generation::userid.eq(generate_options.user_id).and(generation::jobid.eq(uuid_job_id))))
                                                .set(generation::status.eq(GenerationStatus::Working))
                                                .execute(conn)
                                                .await?;
                            Ok(Ok(()))
                        }.scope_boxed())
                        .await
                        .map_err(|err| {
                            tracing::error!("Transaction error: {err}");
                            GenerationError::PostgresCommandFailure
                        })?;
        if let Err(err) = ret {
            return Err(err);
        }
    }

    let mut paper = paper::Paper::new(generate_options.user_id, generate_options.gen_id, generate_options.opts);
    let population_result = paper.populate();

    match population_result {
        Ok(()) => (),
        Err(failure) => return Err(GenerationError::InternalGenerationFailure(failure)),
    }

    let mut serialize_buf = Vec::new();
    let serialize_result = paper.serialize(&mut Serializer::new(&mut serialize_buf));

    match serialize_result {
        Ok(()) => (),
        Err(_) => return Err(GenerationError::SerializeError),
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if let Err(err) = encoder.write_all(&serialize_buf) {
        tracing::error!("Failed to write to compression buffer due to: {err}");
        return Err(GenerationError::CompressionError);
    }
    let serialize_gzip_buf = match encoder.finish() {
        Ok(buf) => buf,
        Err(err) => {
            tracing::error!("Failed to compress buffer due to: {err}");
            return Err(GenerationError::CompressionError);
        },
    };
    
    let put_result = s3_client.put_object()
                                .body(aws_sdk_s3::primitives::ByteStream::from(serialize_gzip_buf))
                                .bucket(&*crate::GENERATED_BUCKET_NAME)
                                .key(format!("{}.rapidl.gz", generate_options.job_id))
                                .content_encoding("gzip")
                                .send()
                                .await;
    if let Err(put_err) = put_result {
        tracing::error!("Failed to put serialised object to S3 due to {put_err}");
        return Err(GenerationError::S3PutError);
    }
    
    let finished_on = chrono::Utc::now().naive_utc();

    {
        let mut postgres_conn = appstate.postgres.get()
                                .await.map_err(|err| {
                                    tracing::error!("Failed to open postgres connection, {err}");
                                    GenerationError::PostgresConnectionFailure
                                })?;

        let _ = diesel::update(generation::table.filter(generation::jobid.eq(uuid_job_id)))
                    .set((
                            generation::status.eq(GenerationStatus::Success),
                            generation::finishedon.eq(finished_on),
                    ))
                    .execute(&mut postgres_conn)
                    .await.map_err(|err| {
                                tracing::error!("Insert postgres failure: {}", err);
                                GenerationError::PostgresCommandFailure
                            })?;
    }
    let mut redis_conn = appstate.redis.get()
                            .await.map_err(|err| {
                                tracing::error!("Failed to open redis connection, {err}");
                                GenerationError::RedisConnectionFailure
                            })?;

    let generate_redis_key = format!("gen:job:{}", generate_options.job_id);
    if let Err(err) = cmd("SET")
        .arg(&[&generate_redis_key, "Success", "EX", "240"])
        .query_async::<_, ()>(&mut redis_conn)
        .await
    {
        tracing::error!("Redis set command failed, {:?}", err);
        return Err(GenerationError::RedisCommandFailure);
    }

    Ok(())
}
