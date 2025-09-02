use ::std::sync::Arc;
use aws_lambda_events::event::ses::{SimpleEmailEvent, SimpleEmailVerdict};
use aws_config::BehaviorVersion;
use lambda_runtime::{service_fn, tracing, Error as LambdaError, LambdaEvent};
use lazy_static::lazy_static;
use mail_parser::*;
use unicode_normalization::UnicodeNormalization;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_async::scoped_futures::ScopedFutureExt;
use db_schema::{supporttickets, supportticketmessages};
use db_schema::hooked_sql_types::SupportTicketState;
use common_types_accounts::{DB::SupportTicket, Constants};
use rustrict::CensorStr;
use summarizer::summarize;
use chrono::{Utc, NaiveDateTime};
use common_types::SESContacts::{
    Request,
    SendIndividual,
    Command,
};
use serde_json::json;

#[derive(Insertable)]
#[diesel(table_name = supportticketmessages)]
#[allow(non_snake_case)]
pub struct SupportTicketMessage<'a> {
    pub ticketid: i32,
    pub message: &'a str,
    pub createdat: NaiveDateTime,
}

lazy_static!{
    static ref SUPPORT_INBOX_BUCKET_NAME: String = {
        dotenvy::var("SUPPORT_INBOX_BUCKET_NAME").expect("No environment variable for SUPPORT_INBOX_BUCKET_NAME").to_owned()            
    };
}

pub fn extract_first_text_segment(text: &str) -> Option<&str> {
    // Search for every \r\n and check for the next \r\n>
    // 1. If we found an \r\n and \r\n> is right next to it,
    //    find the previous \r\n (before our first \r\n).
    //    This will be our extracted text segment.
    // 2. If we found an \r\n\r\n and \r\n\r\n> after,
    //    then everything before the \r\n\r\n is our
    //    extracted text segment

    let tag_short_end = text.find("\r\n>");
    let tag_large_end = text.find("\r\n\r\n>");
    
    match (tag_large_end, tag_short_end) {
        (Some(tag_large_end), _) => {
            let (left, _) = text.split_at(tag_large_end);
            let tag_large_begin = left.rfind("\r\n\r\n")?;
            if tag_large_begin == 0 {
                return None;
            }
            Some(&left[0..tag_large_begin])
        },
        (None, Some(tag_short_end)) => {
            let (left, _) = text.split_at(tag_short_end);
            let tag_short_begin = left.rfind("\r\n")?;
            if tag_short_begin == 0 {
                return None;
            }
            Some(&left[0..tag_short_begin])
        },
        (_, _) => None,
    }
}

async fn delete_message(s3_client: Arc<aws_sdk_s3::Client>, message_id: &str) {
    let _ = s3_client
                .delete_object()
                .bucket(&*SUPPORT_INBOX_BUCKET_NAME)
                .key(message_id)
                .send()
                .await;
}

#[tracing::instrument(skip(appstate, lambda_client, s3_client, event), fields(req_id = %event.context.request_id))]
async fn handler(
    appstate: Arc<common_types_accounts::State::InternalAppState>,
    lambda_client: Arc<aws_sdk_lambda::Client>,
    s3_client: Arc<aws_sdk_s3::Client>,
    event: LambdaEvent<SimpleEmailEvent>,
) -> Result<&'static str, LambdaError> {
    let mut tasks: Vec<(tokio::task::JoinHandle<Result<(), String>>, String)> = Vec::new();

    for record in event.payload.records.into_iter() {
        let Some(message_id) = record.ses.mail.message_id else {
            continue;
        };

        match record.ses.receipt.dkim_verdict {
            SimpleEmailVerdict { status: Some(status) } => {
                match status.as_str() {
                    "PASS" => (),
                    _ => {
                        delete_message(Arc::clone(&s3_client), &message_id).await;
                        continue;
                    },
                }
            },
            _ => tracing::warn!("No spamVerdict status!"),
        }


        match record.ses.receipt.spam_verdict {
            SimpleEmailVerdict { status: Some(status) } => {
                match status.as_str() {
                    "PASS" | "GRAY" => (),
                    _ => {
                        delete_message(Arc::clone(&s3_client), &message_id).await;
                        continue;
                    },
                }
            },
            _ => tracing::warn!("No spamVerdict status!"),
        }

        match record.ses.receipt.virus_verdict {
            SimpleEmailVerdict { status: Some(status) } => {
                match status.as_str() {
                    "PASS" => (),
                    _ => {
                        delete_message(Arc::clone(&s3_client), &message_id).await;
                        continue;
                    },
                }
            },
            _ => tracing::warn!("No virusVerdict status!"),
        }

        let _message_id = message_id.clone();
        let appstate = Arc::clone(&appstate);
        let lambda_client = Arc::clone(&lambda_client);
        let s3_client = Arc::clone(&s3_client);
        tasks.push((tokio::spawn(async move {
            let object = s3_client
                                .get_object()
                                .bucket(&*SUPPORT_INBOX_BUCKET_NAME)
                                .key(&message_id)
                                .send()
                                .await
                                .map_err(|e| {
                                    format!("S3-error: {}", e.into_service_error())
                                })?;

            let Ok(bytes) = object.body.collect().await.map(|d| d.into_bytes()) else {
                return Err("Failed to collect object bytestream into bytes".to_string());
            };

            let binding = Into::<Vec<u8>>::into(bytes);
            let message = MessageParser::default().parse(&binding).ok_or("Failed to parse email")?;

            if message.text_body_count() == 0 {
                return Err("Email must have at least 1 text body".to_string());
            }

            let from = message.from().ok_or("No from field".to_string())?;
            let from = from.first().ok_or("No address in from field".to_string())?;
            let Some(ref from) = from.address else {
                return Err("No address in Addr".to_string());
            };

            let send_email_error = 
                |reasons: Vec<String>, instruction: String| async move {
                    tracing::info!("EmailError request: {:?}, {instruction}, from: {from}", reasons);
                    let template = SendIndividual {
                        template_name: "supportticketreceivefailure".to_string(),
                        template_data: json!({
                            "reasons": reasons.join("<br/>"),
                            "instruction": instruction,
                        }).to_string(),
                    };
                    let lambda_request = Request {
                        commands: Command::SendIndividual(template),
                        email: from.clone().into_owned(),
                    };
                    let _ = lambda_client
                                    .invoke()
                                    .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                                    .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                                    .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                                    .send()
                                    .await;
                };
            
            let subject = message.subject().ok_or("No header field to parse")?;
            let ticket_id;
            {
                let begin_ticket = subject.find('#').ok_or("No ticket id found in subject")?;
                let mut end_ticket = begin_ticket + 1;
                while end_ticket < subject.len() && subject.as_bytes()[end_ticket].is_ascii_digit() {
                    end_ticket += 1;
                }
                if end_ticket == begin_ticket + 1 {
                    return Err("No ticket id found in subject".to_string());
                }
                let m_ticket_id = &subject[begin_ticket + 1..end_ticket];
                if !m_ticket_id.chars().all(|c| c.is_digit(10)) {
                    return Err("Invalid ticket id".to_string());
                }
                ticket_id = m_ticket_id.parse::<i32>().map_err(|_| "Failed to parse ticket id to i32".to_string())?;
            }

            let body_text = message.body_text(0).ok_or("Failed to parse ticket because no text body was found".to_string())?;
            let text = extract_first_text_segment(&body_text).ok_or("Failed to extract first text segment".to_string())?;
            let text = text.censor().nfkc().collect::<String>();

            if text.len() > 2000 - 1 {
                send_email_error(vec![
                                     "- Ticket is too long (must be below 2000 characters)".to_string()
                    ], "Please attempt to resend a smaller email at a later time.".to_string()).await;
                return Err("Ticket is too long".to_string());
            }
            
            // Summarise text if possible
            let mut text_summary = match text.len() > 50 {
                true => summarize(text.as_str(), 0.3),
                false => text.clone(),
            };
            text_summary.truncate(100);

            // Now, DB query to check if this came from the right email
            enum TransactionResult {
                Success,
                NotFound,
                AlreadyClosed,
            }
            let mut conn = match appstate.postgres.get().await {
                Ok(conn) => conn,
                Err(err) => {
                    send_email_error(vec![
                                     "- Internal Server Error".to_string()
                    ], "Please attempt to resend your email at a later time.".to_string()).await;
                    return Err(format!("Failed to fetch Postgres connection due to {err}"))
                },
            };
            let result = conn.build_transaction()
                            .serializable()
                            .run::<_, diesel::result::Error, _>(|conn| async move {
                                let utc = Utc::now().naive_utc();
                                let ticket: SupportTicket = match supporttickets::table.filter(supporttickets::id.eq(ticket_id).and(supporttickets::email.eq(from)))
                                                .select(SupportTicket::as_select())
                                                .for_update()
                                                .first(conn)
                                                .await {
                                                    Ok(ticket) => ticket,
                                                    Err(err) => match err {
                                                        diesel::result::Error::NotFound => return Ok(TransactionResult::NotFound),
                                                        _ => return Err(err),
                                                    }
                                                };
                                if let SupportTicketState::Closed = ticket.state {
                                    return Ok(TransactionResult::AlreadyClosed);
                                }
                                let ticket_updated = diesel::update(supporttickets::table.filter(supporttickets::id.eq(ticket_id).and(supporttickets::email.eq(from))))
                                                .set((
                                                        supporttickets::summary.eq(text_summary),
                                                        supporttickets::lastchanged.eq(utc)
                                                ))
                                                .execute(conn)
                                                .await?;
                                if ticket_updated != 1 {
                                    return Err(diesel::result::Error::RollbackTransaction);
                                }
                                let ticket_message_added = diesel::insert_into(supportticketmessages::table)
                                    .values(&SupportTicketMessage {
                                            ticketid: ticket_id,
                                            message: &text,
                                            createdat: utc,
                                        })
                                    .execute(conn).await?;
                                if ticket_message_added != 1 {
                                    return Err(diesel::result::Error::RollbackTransaction);
                                }
                                Ok(TransactionResult::Success)
                            }.scope_boxed()).await.map_err(|err| {
                                        format!("Transaction error: {err}")
                                    })?;
            match result {
                TransactionResult::Success => Ok(()),
                TransactionResult::NotFound => {
                    send_email_error(vec![
                                     "- No ticket with matching ID".to_string(),
                                     "- No ticket with matching assigned email".to_string(),
                    ], "Please verify that you are using the same email address provided in the contact form, and include the ticket ID in the subject line (e.g., #123).".to_string()).await;
                    Err("No ticket found in database".to_string())
                },
                TransactionResult::AlreadyClosed => {
                    send_email_error(vec![
                                     "- Ticket has been closed".to_string(),
                    ], "The ticket has been closed, and further discussion is no longer possible. For any inquiries, please use the contact form on our website.".to_string()).await;
                    Err("Ticket has already been closed".to_string())
                },
            }
        }), _message_id));
    }
    for (task, message_id) in tasks.into_iter() {
        match task.await {
            Ok(e) => match e {
                Ok(_) => (),
                Err(err) => {
                    tracing::error!("TaskError: {err}");
                },
            },
            Err(err) => {
                tracing::error!("JoinError: {err}");
            },
        }
        delete_message(Arc::clone(&s3_client), &message_id).await;
    }
    Ok("CONTINUE")
}

#[tokio::main]
async fn main() -> Result<(), LambdaError> {
    tracing::init_default_subscriber();

    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let lambda_client = Arc::new(aws_sdk_lambda::Client::new(&config));
    let s3_client = Arc::new(aws_sdk_s3::Client::new(&config));

    let appstate = common_types_accounts::State::make_state().await?;

    lambda_runtime::run(service_fn(|event: LambdaEvent<SimpleEmailEvent>| async {
        handler(Arc::clone(&appstate), Arc::clone(&lambda_client), Arc::clone(&s3_client), event).await
    }))
    .await
}

