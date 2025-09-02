use ::std::error::Error;
use ::std::sync::Arc;
use aws_config::BehaviorVersion;
use aws_sdk_sesv2::{
    error::SdkError,
    operation::get_contact_list::GetContactListError,
    types::{
        Destination,
        EmailContent,
        Template,
        Topic,
        TopicPreference,
        SubscriptionStatus,
    },
};
use serde::Serialize;
use lambda_runtime::{service_fn, Error as LambdaError, LambdaEvent};
use lazy_static::lazy_static;
use common_types::{
    SESContacts::{
        Request,
        RequestType,
        TopicType,
        Command,
        Response,
        ResponseBuilder,
    },
    SQSEmail::SQSBody,
};

lazy_static!{
    static ref NEWSLETTER_BUCKET_NAME: String = {
        dotenvy::var("NEWSLETTER_BUCKET_NAME").expect("No environment variable for NEWSLETTER_BUCKET_NAME").to_owned()            
    };
    static ref NEWSLETTER_LATEST_FILE: String = {
        dotenvy::var("NEWSLETTER_LATEST_FILE").expect("No environment variable for NEWSLETTER_LATEST_FILE").to_owned()
    };
    static ref BULK_EMAIL_QUEUE_URL: String = {
        dotenvy::var("BULK_EMAIL_QUEUE_URL").expect("No environment variable for BULK_EMAIL_QUEUE_URL").to_owned()
    };
}
use common_types_accounts::{State, Email};

#[tracing::instrument(skip(appstate, sqs_client, s3_client, ses_client, event), fields(req_id = %event.context.request_id))]
async fn handler(
    appstate: Arc<State::InternalAppState>,
    sqs_client: &aws_sdk_sqs::Client,
    s3_client: &aws_sdk_s3::Client,
    ses_client: &aws_sdk_sesv2::Client,
    event: LambdaEvent<Request>,
) -> Result<Response, LambdaError> {

    match event.payload.commands {
        Command::ActionType(request_type, topic_type) => {
            if let RequestType::IsInMailList = request_type {
                if let Ok(contact) = ses_client
                                        .get_contact()
                                        .contact_list_name("list-all")
                                        .email_address(&event.payload.email)
                                        .send()
                                        .await
                {
                    if let Some(subscribed_topics) = contact.topic_preferences {
                        for topic in subscribed_topics.iter() {
                            if topic.topic_name == topic_type.to_string() {
                                if let SubscriptionStatus::OptIn = topic.subscription_status {
                                    return Ok(ResponseBuilder::default().is_email_in_mail_list(true).build().unwrap());
                                }
                                break;
                            }
                        }
                    }
                }
                return Ok(ResponseBuilder::default().is_email_in_mail_list(false).build().unwrap());
            }

            let preferences = vec![
                    TopicPreference::builder()
                        .topic_name(topic_type.to_string())
                        .subscription_status(match request_type {
                            RequestType::AddToMailList => SubscriptionStatus::OptIn,
                            RequestType::RemoveFromMailList => SubscriptionStatus::OptOut,
                            _ => unreachable!(),
                        })
                        .build()
                        .unwrap()

                ];

            if let Ok(contact) = ses_client
                .get_contact()
                .contact_list_name("list-all")
                .email_address(&event.payload.email)
                .send()
                .await 
            {
                if let Some(subscribed_topics) = contact.topic_preferences {
                    for topic in subscribed_topics.iter() {
                        if topic.topic_name == topic_type.to_string() {
                            match (&topic.subscription_status, request_type) {
                                (&SubscriptionStatus::OptIn, RequestType::AddToMailList) => {
                                    return Ok(ResponseBuilder::default().build().unwrap());
                                },
                                (&SubscriptionStatus::OptOut, RequestType::RemoveFromMailList) => {
                                    return Ok(ResponseBuilder::default().build().unwrap());
                                },
                                _ => (),
                            }
                            break
                        }
                    }
                }
                ses_client
                    .update_contact()
                    .contact_list_name("list-all")
                    .email_address(&event.payload.email)
                    .set_topic_preferences(Some(preferences))
                    .send()
                    .await?;
                return Ok(ResponseBuilder::default().build().unwrap());
            }

            ses_client
                .create_contact()
                .contact_list_name("list-all")
                .email_address(&event.payload.email)
                .set_topic_preferences(Some(preferences))
                .send()
                .await?;
        },
        Command::SendBulk(topic) => {
            match topic {
                TopicType::Advertising => return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Cannot send bulk using a subscription-only template")) as Box<dyn Error + Send + Sync>),
            }
        }
        Command::SendBulkSubscription(topic) => {
            match topic {
                TopicType::Advertising => {
                    /* fetch latest.txt from bucket */
                    /* then pass onto queue */
                    let object = s3_client
                                    .get_object()
                                    .bucket(&*NEWSLETTER_BUCKET_NAME)
                                    .key(&*NEWSLETTER_LATEST_FILE)
                                    .send()
                                    .await?;
                    let bytes = object.body.collect().await.map(|d| d.into_bytes())?;
                    let news_data = String::from_utf8(bytes.into()).expect("Newsletter contains invalid bytes");
                    /* we expect this format IMAGE/#n/TITLE/#n/DESCRIPTION/#n/... */
                    let news_slices = news_data.split(r#"/#n/"#).collect::<Vec<&str>>();
                    if news_slices.len() % 3 != 0 {
                        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Invalid news slices")) as Box<dyn Error + Send + Sync>);
                    }
                    #[derive(Serialize)]
                    struct TemplateData {
                        griddata: String,
                        plainnews: String,
                    }
                    let mut template_data = TemplateData {
                        griddata: String::new(),
                        plainnews: String::new(),
                    };
                    for chunk in news_slices.chunks(3) {
                        let image = chunk[0];
                        let title = chunk[1];
                        let description = chunk[2];
                        template_data.plainnews.push_str(
                                &format!(
                                        "{title}: {description}\r\n"
                                    )
                            );
                        template_data.griddata.push_str(
                                &format!(
                                        r#"<div style="border-radius:10px;overflow:hidden;margin-bottom:20px"><img src="{image}" style="width:100%;height:auto;border-radius:10px"><div style="padding:15px"><h4 style="color:#fff;margin:0">{title}</h4><p style="color:#aaa;margin-top:5px">{description}</p></div></div>"#
                                    )
                            );
                    }
                    let template_info = SQSBody {
                        requires_subscription: true,
                        send_bulk: false,
                        topic: TopicType::Advertising.to_string(),
                        next_token: None,
                        template_name: "newslettertemplate".to_string(),
                        template_data: serde_json::to_string(&template_data).expect("Newsletter data serialization error"),
                    };
                    let template_info = serde_json::to_string(&template_info).expect("Newsletter info Serialization error");
                    sqs_client
                        .send_message()
                        .queue_url(&*BULK_EMAIL_QUEUE_URL)
                        .message_body(template_info)
                        .delay_seconds(5)
                        .send()
                        .await?;
                },
            }
        }
        Command::SendIndividual(template) => {
            if template.template_name == "newslettertemplate" {
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "You cannot send Newsletter template to an individual")) as Box<dyn Error + Send + Sync>);
            }
            if !Email::is_safe_to_send_to(Arc::clone(&appstate), &event.payload.email).await {
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "This address is not safe to send to due to high complaints or bounce count")) as Box<dyn Error + Send + Sync>);
            }
            ses_client
                .send_email()
                .from_email_address("no-reply@rapidl.co.uk")
                .destination(
                        Destination::builder()
                            .to_addresses(&event.payload.email)
                            .build()
                    )
                .content(
                        EmailContent::builder()
                            .template(
                                    Template::builder()
                                        .template_name(template.template_name)
                                        .template_data(template.template_data)
                                        .build()
                                )
                            .build()
                    )
                .send()
                .await?;
        },
        Command::SendIndividualCustomReplyTo(template, replyto) => {
            if template.template_name == "newslettertemplate" {
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "You cannot send Newsletter template to an individual")) as Box<dyn Error + Send + Sync>);
            }
            if !Email::is_safe_to_send_to(Arc::clone(&appstate), &event.payload.email).await {
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "This address is not safe to send to due to high complaints or bounce count")) as Box<dyn Error + Send + Sync>);
            }
            ses_client
                .send_email()
                .from_email_address(format!("{replyto}@ses.rapidl.co.uk"))
                .destination(
                        Destination::builder()
                            .to_addresses(&event.payload.email)
                            .build()
                    )
                .content(
                        EmailContent::builder()
                            .template(
                                    Template::builder()
                                        .template_name(template.template_name)
                                        .template_data(template.template_data)
                                        .build()
                                )
                            .build()
                    )
                .send()
                .await?;

        },
    }
    
    Ok(ResponseBuilder::default().build().unwrap())
}



async fn _make_contact_list_if_not_exist(
    ses_client: &aws_sdk_sesv2::Client,
    contact_list: &str,
    contact_description: &str,
    topics: Option<Vec<Topic>>,
) -> Result<(),()> {
    let contact_error = match ses_client.get_contact_list().contact_list_name(contact_list).send().await {
        Ok(_) => return Ok(()),
        Err(err) => err,
    };
    
    let SdkError::ServiceError(service) = contact_error else { 
        tracing::error!("Failed to get contact list, {contact_error}");
        return Err(()) 
    };
    let &GetContactListError::NotFoundException(_) = service.err() else { 
        tracing::error!("Failed to get contact list, expected NotFound but got, {}", service.err());
        return Err(()) 
    };

    let _ = ses_client
                .create_contact_list()
                .contact_list_name(contact_list)
                .description(contact_description)
                .set_topics(topics)
                .send()
                .await
                .map_err(|_| ())?;

    Ok(())
}

async fn build_contact_lists(
    ses_client: &aws_sdk_sesv2::Client,
) -> Result<(),()> {
    let _ = _make_contact_list_if_not_exist(
        ses_client, 
        "list-all", 
        "A contact list of opt-in topics",
        Some(Vec::from([
            Topic::builder()
                .topic_name(TopicType::Advertising.to_string())
                .display_name("Weekly newsletter")
                .description("Stay updated with our weekly newsletter, featuring the latest news, including new features, promotions, and more.")
                .default_subscription_status(SubscriptionStatus::OptIn)
                .build()
                .unwrap()
        ])),
    ).await?;
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
    let ses_client = aws_sdk_sesv2::Client::new(&config);
    let s3_client = aws_sdk_s3::Client::new(&config);
    let sqs_client = aws_sdk_sqs::Client::new(&config);

    match build_contact_lists(&ses_client).await {
        Ok(_) => (),
        Err(_) => return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Failed to build contact lists")) as Box<dyn Error + Send + Sync>),
    }

    let appstate = common_types_accounts::State::make_state().await?;

    lambda_runtime::run(service_fn(|event: LambdaEvent<Request>| async {
        handler(Arc::clone(&appstate), &sqs_client, &s3_client, &ses_client, event).await
    }))
    .await
}

