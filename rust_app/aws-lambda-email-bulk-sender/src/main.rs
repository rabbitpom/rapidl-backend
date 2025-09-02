use ::std::sync::Arc;
use aws_lambda_events::event::sqs::{SqsEvent, SqsMessage};
use aws_config::BehaviorVersion;
use lambda_runtime::{service_fn, Error as LambdaError, LambdaEvent};
use serde_json::to_string;
use aws_sdk_sesv2::types::{
    ListContactsFilter,
    SubscriptionStatus,
    TopicFilter,
    EmailContent,
    BulkEmailContent,
    BulkEmailEntry,
    Template,
    Destination,
    ListManagementOptions,
    MessageHeader,
};
use lazy_static::lazy_static;
use common_types::SQSEmail::SQSBody;

lazy_static!{
    static ref SQS_URL: String = {
        dotenvy::var("SQS_URL").expect("No environment variable for SQS_URL").to_owned()            
    };
}

async fn delete_message(sqs_client: Arc<aws_sdk_sqs::Client>, record: &SqsMessage) -> Result<(), LambdaError> {
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

#[tracing::instrument(skip(ses_client, sqs_client, event), fields(req_id = %event.context.request_id))]
async fn handler(
    ses_client: Arc<aws_sdk_sesv2::Client>,
    sqs_client: Arc<aws_sdk_sqs::Client>,
    event: LambdaEvent<SqsEvent>,
) -> Result<(), LambdaError> {
    for record in event.payload.records.iter() {
        // process the record
        if let Some(body) = &record.body {
            if let Ok(body) = serde_json::from_str::<SQSBody>(body) {
                let contacts_output = match body.requires_subscription {
                    true => ses_client
                                .list_contacts()
                                .contact_list_name("list-all")
                                .page_size(50)
                                .filter(
                                    ListContactsFilter::builder()
                                        .filtered_status(SubscriptionStatus::OptIn)
                                        .topic_filter(
                                                TopicFilter::builder()
                                                    .topic_name(&body.topic)
                                                    .use_default_if_preference_unavailable(false)
                                                    .build()
                                            )
                                        .build()
                                    )
                                .set_next_token(body.next_token.clone())
                                .send()
                                .await?,
                    false => ses_client
                                .list_contacts()
                                .contact_list_name("list-all")
                                .page_size(50)
                                .set_next_token(body.next_token.clone())
                                .send()
                                .await?,
                };
                if let Some(contacts) = contacts_output.contacts {
                    match body.send_bulk {
                        true => {
                            let destination = Destination::builder();
                            let entry = BulkEmailEntry::builder();
                            let mut bcc_addresses = Vec::new();
                            for contact in contacts.into_iter() {
                                if let Some(email_address) = contact.email_address {
                                    bcc_addresses.push(email_address);
                                }
                            }
                            let destination = destination.set_bcc_addresses(Some(bcc_addresses));
                            let destination = destination.build();
                            ses_client
                                .send_bulk_email()
                                .from_email_address("no-reply@rapidl.co.uk")
                                .bulk_email_entries(entry.destination(destination).build())
                                .default_content(
                                        BulkEmailContent::builder()
                                            .template(
                                                    Template::builder()
                                                        .template_name(&body.template_name)
                                                        .template_data(&body.template_data)
                                                        .build()
                                                )
                                            .build()
                                    )
                                .send()
                                .await?;
                        },
                        false => {
                            let partial_body = Arc::new(body.partial_clone());
                            for contact in contacts.into_iter() {
                                if let Some(email_address) = contact.email_address {
                                    let handle : tokio::task::JoinHandle<Result<(), LambdaError>>;
                                    {
                                        let partial_body = Arc::clone(&partial_body);
                                        let ses_client = Arc::clone(&ses_client);
                                        handle = tokio::spawn(async move {
                                            ses_client
                                                .send_email()
                                                .from_email_address("no-reply@rapidl.co.uk")
                                                .destination(
                                                        Destination::builder()
                                                            .to_addresses(&email_address)
                                                            .build()
                                                    )
                                                .content(
                                                    EmailContent::builder()
                                                        .template(
                                                                Template::builder()
                                                                    .template_name(&partial_body.template_name)
                                                                    .template_data(&partial_body.template_data)
                                                                    .headers(
                                                                            MessageHeader::builder()
                                                                                .name("List-Unsubscribe")
                                                                                .value("<https://www.rapidl.co.uk>")
                                                                                .build()?
                                                                        )
                                                                    .headers(
                                                                            MessageHeader::builder()
                                                                                .name("List-Unsubscribe-Post")
                                                                                .value("List-Unsubscribe=One-Click")
                                                                                .build()?
                                                                        )
                                                                    .build()
                                                            )
                                                        .build()
                                                    )
                                                .list_management_options(
                                                        ListManagementOptions::builder()
                                                            .contact_list_name("list-all")
                                                            .topic_name(&partial_body.topic)
                                                            .build()?
                                                    )
                                                .send()
                                                .await?;
                                            Ok(())
                                        });
                                    }
                                    /* dnc about errors lol */
                                    let _ = handle.await;
                                }
                            }
                        },
                    }
                }
                if let Some(next_token) = contacts_output.next_token {
                    let next_info = SQSBody {
                        send_bulk: body.send_bulk,
                        requires_subscription: body.requires_subscription,
                        topic: body.topic.clone(),
                        next_token: Some(next_token),
                        template_name: body.template_name.clone(),
                        template_data: body.template_data.clone(),
                    };
                    let _ = sqs_client
                                .send_message()
                                .queue_url(&*SQS_URL)
                                .message_body(to_string(&next_info).expect("Failed to serialize next bulk email info"))
                                .send()
                                .await?;
                }
            } else {
                tracing::error!("Failed to deserialize body: {}", body);
            }
        } else {
            tracing::warn!("Empty body encountered in record");
        }
        delete_message(sqs_client.clone(), record).await?;
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
    let sqs_client = Arc::new(aws_sdk_sqs::Client::new(&config));
    let ses_client = Arc::new(aws_sdk_sesv2::Client::new(&config));

    lambda_runtime::run(service_fn(|event: LambdaEvent<SqsEvent>| async {
        handler(ses_client.clone(), sqs_client.clone(), event).await
    }))
    .await
}

