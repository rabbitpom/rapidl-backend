use ::std::sync::Arc;
use aws_lambda_events::event::sqs::{SqsEvent, SqsMessage};
use aws_config::BehaviorVersion;
use lambda_runtime::{service_fn, Error as LambdaError, LambdaEvent};
use lazy_static::lazy_static;
use common_types::{
    SESSNS::{
        SQSSNSBody,
        Message,
        NotificationType,
    },
    SESContacts::{
        Request,
        RequestType,
        TopicType,
        Command,
    },
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use common_types_accounts::Constants;
use db_schema::problematicemails;
use sha2::{Sha256, Digest};

lazy_static!{
    static ref SQS_URL: String = {
        dotenvy::var("SQS_URL").expect("No environment variable for SQS_URL").to_owned()            
    };
    static ref LAMBDA_EMAIL_ARN: String = {
        dotenvy::var("LAMBDA_EMAIL_ARN").expect("No environment variable for LAMBDA_EMAIL_ARN").to_owned()
    };
}

async fn delete_message(sqs_client: &aws_sdk_sqs::Client, record: &SqsMessage) -> Result<(), LambdaError> {
    if let Some(ref receipt_handle) = record.receipt_handle {
                    let _ = sqs_client
                        .delete_message()
                        .queue_url(&*SQS_URL)
                        .receipt_handle(receipt_handle)
                        .send()
                        .await?;
    }
    Ok(())
}

async fn raise_count_in_db(appstate: Arc<common_types_accounts::State::InternalAppState>, email: &str) {
    let email_identifier;
    {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}rapidl-nonce!#?", email));
        email_identifier = hex::encode(hasher.finalize());
    }
    let Ok(mut conn) = appstate.postgres.get().await else {
        return;
    };
    let now = chrono::Utc::now().naive_utc();
    let base_next_reset = now + chrono::Duration::seconds(*Constants::COMPLAINT_BOUNCE_NEXT_RESET);
    let _ = diesel::insert_into(problematicemails::table)
                .values((
                    problematicemails::hash.eq(email_identifier),
                    problematicemails::count.eq(1),
                    problematicemails::nextreset.eq(base_next_reset),
                ))
                .on_conflict(problematicemails::hash)
                .do_update()
                .set((
                    problematicemails::count.eq(problematicemails::count + 1),
                    problematicemails::nextreset.eq(base_next_reset),
                ))
                .execute(&mut conn)
                .await;
}

#[tracing::instrument(skip(appstate, lambda_client, sqs_client, event), fields(req_id = %event.context.request_id))]
async fn handler(
    appstate: Arc<common_types_accounts::State::InternalAppState>,
    lambda_client: Arc<aws_sdk_lambda::Client>,
    sqs_client: &aws_sdk_sqs::Client,
    event: LambdaEvent<SqsEvent>,
) -> Result<(), LambdaError> {
    for record in event.payload.records.iter() {
        // process the record
        if let Some(body) = &record.body {
            if let Ok(body) = serde_json::from_str::<SQSSNSBody>(body) {
                if let Ok(message) = serde_json::from_str::<Message>(&body.message) {
                    match message.notification_type {
                        NotificationType::Bounce => {
                            let bounce = message.bounce.as_ref().unwrap();
                            for recipient in bounce.bounced_recipients.iter() {
                                let email_address = recipient.email_address.clone();
                                let lambda_client = Arc::clone(&lambda_client);
                                let appstate = Arc::clone(&appstate);
                                let _ = tokio::spawn(async move {
                                    raise_count_in_db(appstate, &email_address).await;

                                    let lambda_request = Request {
                                        commands: Command::ActionType(RequestType::RemoveFromMailList, TopicType::Advertising),
                                        email: email_address,
                                    };
                                    let Err(error) = lambda_client
                                        .invoke()
                                        .function_name(&*LAMBDA_EMAIL_ARN)
                                        .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                                        .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                                        .send()
                                        .await else { return; };
                                    tracing::error!("Failed to invoke lambda: {}", error);
                                }).await;
                            }
                        },
                        NotificationType::Complaint => {
                            let complaint = message.complaint.as_ref().unwrap();
                            for recipient in complaint.complained_recipients.iter() {
                                let email_address = recipient.email_address.clone();
                                let lambda_client = Arc::clone(&lambda_client);
                                let appstate = Arc::clone(&appstate);
                                let _ = tokio::spawn(async move {
                                    raise_count_in_db(appstate, &email_address).await;

                                    let lambda_request = Request {
                                        commands: Command::ActionType(RequestType::RemoveFromMailList, TopicType::Advertising),
                                        email: email_address,
                                    };
                                    let Err(error) = lambda_client
                                        .invoke()
                                        .function_name(&*LAMBDA_EMAIL_ARN)
                                        .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                                        .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                                        .send()
                                        .await else { return; };
                                    tracing::error!("Failed to invoke lambda: {}", error);
                                }).await;
                            }
                        },
                        _ => {},
                    }
                } else {
                    tracing::error!("Failed to deserialize message from body: {}", body.message);
                }
            } else {
                tracing::error!("Failed to deserialize body: {}", body);
            }
        } else {
            tracing::warn!("Empty body encountered in record");
        }
        delete_message(sqs_client, record).await?;
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
    let lambda_client = Arc::new(aws_sdk_lambda::Client::new(&config));
    let sqs_client = aws_sdk_sqs::Client::new(&config);

    let appstate = common_types_accounts::State::make_state().await?;

    lambda_runtime::run(service_fn(|event: LambdaEvent<SqsEvent>| async {
        handler(Arc::clone(&appstate), Arc::clone(&lambda_client), &sqs_client, event).await
    }))
    .await
}

