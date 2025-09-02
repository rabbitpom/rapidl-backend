// Entry point for lambda

use ::std::sync::Arc;
use aws_config::BehaviorVersion;
use aws_lambda_events::event::sqs::{SqsEvent, SqsMessage};
use lambda_runtime::{service_fn, Error as LambdaError, LambdaEvent};
use lazy_static::lazy_static;
use common_types::Generate::SQSBody;
use common_types_accounts::Schema::{generation, hooked_sql_types::GenerationStatus};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use deadpool_redis::redis::cmd;

mod generate;
use generate::GenerationError;

lazy_static!{
    static ref GENERATE_QUEUE_URL: String = {
        dotenvy::var("GENERATE_QUEUE_URL").expect("No environment variable for GENERATE_QUEUE_URL").to_owned()
    };
    static ref GENERATED_BUCKET_NAME: String = {
        dotenvy::var("GENERATED_BUCKET_NAME").expect("No environment variable for GENERATED_BUCKET_NAME").to_owned()
    };
}

async fn delete_message(sqs_client: Arc<aws_sdk_sqs::Client>, record: &SqsMessage) -> Result<(), LambdaError> {
    if let Some(ref receipt_handle) = record.receipt_handle {
                    let _ = sqs_client
                        .delete_message()
                        .queue_url(&*GENERATE_QUEUE_URL)
                        .receipt_handle(receipt_handle)
                        .send()
                        .await?;
    }
    Ok(())
}
async fn delete_from_receipt(sqs_client: Arc<aws_sdk_sqs::Client>, receipt_handle: String) -> Result<(), LambdaError> {
    let _ = sqs_client
        .delete_message()
        .queue_url(&*GENERATE_QUEUE_URL)
        .receipt_handle(receipt_handle)
        .send()
        .await?;
    Ok(())
}

async fn flag_as_failure(appstate: common_types_accounts::MinimalState::AppState, jobid: String) -> bool {
    let uuid_job_id = uuid::Uuid::try_parse(&jobid);
    let Ok(uuid_job_id) = uuid_job_id else {
        return false; 
    };
    let postgres_conn = appstate.postgres.get()
                                .await;
    let Ok(mut postgres_conn) = postgres_conn else {
        return true; // try again later
    };
    match diesel::update(generation::table.filter(generation::jobid.eq(uuid_job_id)))
        .set(generation::status.eq(GenerationStatus::Failed))
        .execute(&mut postgres_conn)
        .await
    {
        Ok(_) => {
            match appstate.redis.get().await {
                Ok(mut redis_conn) => {
                    let generate_redis_key = format!("gen:job:{uuid_job_id}");
                    if let Err(err) = cmd("SET")
                        .arg(&[&generate_redis_key, "Failed", "EX", "120"])
                        .query_async::<_, ()>(&mut redis_conn)
                        .await
                    {
                        tracing::error!("Redis set command failed to flag as failure but won't try again, {:?}", err);
                        // We won't retry though!
                    }
                },
                Err(err) => tracing::error!("Failed to get redis connection, won't try again!, {:?}", err),
            }
            false
        },
        Err(err) => {
            tracing::error!("Failed to update generation record to failure, due to {err}, will try again later, {uuid_job_id}");
            true // try again later
        },
    }
}

#[tracing::instrument(skip(appstate, sqs_client, s3_client, event), fields(req_id = %event.context.request_id))]
async fn handler(
    appstate: common_types_accounts::MinimalState::AppState,
    sqs_client: Arc<aws_sdk_sqs::Client>,
    s3_client: Arc<aws_sdk_s3::Client>,
    event: LambdaEvent<SqsEvent>,
) -> Result<(), LambdaError> {
    for record in event.payload.records.iter() {
        // process the record
        if let Some(body) = &record.body {
            if let (Ok(body), Some(ref receipt)) = (serde_json::from_str::<SQSBody>(body), &record.receipt_handle) {
                let handle : tokio::task::JoinHandle<Result<(), LambdaError>>;
                {
                    let receipt = receipt.clone();
                    let appstate = appstate.clone();
                    let sqs_client = sqs_client.clone();
                    let s3_client = s3_client.clone();
                    handle = tokio::spawn(async move {
                        let job_id = body.job_id.clone();
                        let result = generate::generate(appstate.clone(), s3_client, body).await;
                        match result {
                            Ok(()) => {
                                delete_from_receipt(sqs_client.clone(), receipt).await?;
                                Ok(())
                            },
                            Err(err) => {
                                use GenerationError::*;
                                match err {
                                    DeleteImmediately => (), // do nothing and let it be deleted
                                                             // from queue
                                    RedisConnectionFailure | PostgresConnectionFailure | PostgresCommandFailure => return Ok(()), // dont delete if it reaches this
                                    InternalGenerationFailure(failure) => {
                                        tracing::error!("Failed to generate due to {:?}", failure);
                                        if flag_as_failure(appstate.clone(), job_id).await {
                                            return Ok(()) // if returns true then we wont delete
                                                          // message and will try again later
                                        }
                                    },
                                    _ => {
                                        if flag_as_failure(appstate.clone(), job_id).await {
                                            return Ok(()) // if returns true then we wont delete
                                                          // message and will try again later
                                        }
                                    },
                                }
                                delete_from_receipt(sqs_client.clone(), receipt).await?;
                                Ok(())
                            }
                        }
                    });
                }
                /* dnc about errors lol */
                let _ = handle.await;
            } else {
                tracing::error!("Failed to deserialize body: {}", body);
                delete_message(sqs_client.clone(), record).await?;
            }
        } else {
            tracing::warn!("Empty body encountered in record");
            delete_message(sqs_client.clone(), record).await?;
        }
    }
    Ok(())

}

#[tokio::main]
async fn main() -> Result<(), LambdaError> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let s3_client = Arc::new(aws_sdk_s3::Client::new(&config));
    let sqs_client = Arc::new(aws_sdk_sqs::Client::new(&config));

    let appstate = common_types_accounts::MinimalState::make_state().await?;

    lambda_runtime::run(service_fn(|event: LambdaEvent<SqsEvent>| async {
        handler(appstate.clone(), sqs_client.clone(), s3_client.clone(), event).await
    }))
    .await
}
