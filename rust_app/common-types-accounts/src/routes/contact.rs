use ::std::sync::{Arc, Mutex};
use axum::{
    extract::{
        Extension,
        State,
    },
    http::StatusCode,
    Json
};
use chrono::Utc;
use garde::Validate;
use rustrict::CensorStr;
use diesel_async::RunQueryDsl;
use summarizer::summarize;
use sha2::{Sha256, Digest};
use diesel::prelude::*;
use deadpool_redis::redis::cmd;
use diesel_async::scoped_futures::ScopedFutureExt;
use common_types::SESContacts::{
    Request,
    SendIndividual,
    Command,
};
use serde_json::json;
use unicode_normalization::UnicodeNormalization;

use crate::{
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Email::verify_email,
    Middleware::request_describer::RequestDescription,
    Schema::{supporttickets, supportticketmessages},
    db_schema::hooked_sql_types::SupportTicketState,
    Constants,
};

mod db;
use db::{RequestPayload, SupportTicket, SupportTicketMessage};

// POST API endpoint
#[tracing::instrument(skip(request_info, appstate), fields(request="/support/contact"))]
pub async fn request(Extension(request_info): Extension<RequestDescription>, State(appstate): State<AppState>, Json(mut user_request): Json<RequestPayload>) -> Result<(), ServerResponse> {
    let validation_result = user_request.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }
    if !verify_email(Arc::clone(&appstate), &user_request.email).await {
        return Err(status_response(StatusCode::BAD_REQUEST, "Invalid email"))
    }
    if user_request.name.is_inappropriate() {
        return Err(status_response(StatusCode::BAD_REQUEST, "Name is inappropriate, please pick a different name"));
    }
    user_request.message = user_request.message.nfkc().collect();
    if user_request.message.is_inappropriate() {
        return Err(status_response(StatusCode::BAD_REQUEST, "Message is inappropriate, please write a different message"));
    }
    let mut message_summary = match user_request.message.len() > 50 {
        true => summarize(user_request.message.as_str(), 0.3),
        false => user_request.message.clone(),
    };
    message_summary.truncate(100);

    // Add some "consistently random" data to IP and hash it
    // since IP is easy to brute force we have to add additional
    // gibberish
    let ip_identifier;
    {
        // The day + month + year + rapidl
        let mangled_ip = format!("{}{}rapidl-nonce!#?", request_info.ip, {
            Utc::now().format("%d/%m/%Y").to_string()
        });
        let mut hasher = Sha256::new();
        hasher.update(mangled_ip);
        let hashed_ip = hasher.finalize();
        ip_identifier = hex::encode(hashed_ip);
    }

    // We also check againts email
    let email_identifier;
    {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}rapidl-nonce!#?", user_request.email));
        email_identifier = hex::encode(hasher.finalize());
    }

    // Check for any request "cooldown" for ip
    {
        let mut redis_conn = appstate.redis.get().await.map_err(|err|{
            tracing::error!("Failed to fetch Redis connection, {err}");
            internal_server_error("Internal Service Error")
        })?;

        /* Check redis cache if this email has already been served in the last
         * SEND_CONTACT_US_COOLDOWN */
        let redis_key = format!("contact:{}", email_identifier);
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
                .arg(&[&redis_key, "true", "EX", &(*Constants::SEND_CONTACT_US_COOLDOWN).to_string()])
                .query_async::<_, ()>(&mut redis_conn)
                .await
            {
                tracing::error!("Redis set command failed, {:?}", err);
                return Err(internal_server_error("Internal Service Error"))
            }
        }

        /* Check redis cache if this request has already been served in the last
         * SEND_CONTACT_US_COOLDOWN */
        let redis_key = format!("contact:{}", ip_identifier);
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
                .arg(&[&redis_key, "true", "EX", &(*Constants::SEND_CONTACT_US_COOLDOWN).to_string()])
                .query_async::<_, ()>(&mut redis_conn)
                .await
            {
                tracing::error!("Redis set command failed, {:?}", err);
                return Err(internal_server_error("Internal Service Error"))
            }
        }
    }

    let created_ticket_id = Arc::new(Mutex::new(None));
    {
        let email = user_request.email.clone();
        let message = user_request.message.clone();

        let created_ticket_id = Arc::clone(&created_ticket_id);
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;

        // Check if they haven't created more than ALLOWED_TICKETS_OPEN_AT_ONCE unresolved tickets
        let tickets: i64 = supporttickets::table.filter(supporttickets::email.eq(&email).and(supporttickets::state.ne(SupportTicketState::Closed)))
                                                .count()
                                                .get_result(&mut conn)
                                                .await
                                                .map_err(|err| {
                                                    tracing::error!("Failed to count tickets, {err}");
                                                    internal_server_error("Internal Service Error")
                                                })?;
        if tickets >= *Constants::ALLOWED_TICKETS_OPEN_AT_ONCE {
            return Err(status_response(StatusCode::TOO_MANY_REQUESTS, "You already have 3 active tickets, please wait for them to be resolved before submitting another contact request"));
        }

        // Make a new ticket, and ticket message
        let _ = conn.build_transaction()
                .repeatable_read()
                .run::<_, diesel::result::Error, _>(|conn| async move {
                    let utc = Utc::now().naive_utc();
                    let ticketid = diesel::insert_into(supporttickets::table)
                        .values(&SupportTicket {
                                name: &user_request.name,
                                email: &email,
                                wau: user_request.whoami,
                                summary: &message_summary,
                                state: SupportTicketState::Unclaimed,
                                claimedby: None,
                                claimedbyname: None,
                                createdat: utc,
                                lastchanged: utc,
                            })
                        .on_conflict_do_nothing()
                        .returning(supporttickets::id)
                        .get_result::<i32>(conn).await?;
                    
                    let ticket_message_added = diesel::insert_into(supportticketmessages::table)
                        .values(&SupportTicketMessage {
                                ticketid,
                                message: &message,
                                createdat: utc,
                                isteam: false,
                            })
                        .execute(conn).await?;

                    if ticket_message_added != 1 {
                        return Err(diesel::result::Error::RollbackTransaction);
                    }

                    *created_ticket_id.lock().unwrap() = Some(ticketid);

                    Ok::<(),_>(())
                }.scope_boxed()).await.map_err(|err| {
                            tracing::error!("Transaction error: {err}");
                             internal_server_error("Internal Service Error")
                        })?;
    }

    let ticketid = match *created_ticket_id.lock().unwrap() {
        Some(ticketid) => ticketid,
        None => {
            return Err(internal_server_error("No ticket ID was created"));
        },
    };

    // Finally, email the user to let them know we got their request
    let template = SendIndividual {
        template_name: "supportticketbegin".to_string(),
        template_data: json!({
            "ticketid": format!("#{ticketid}"),
            "message": ammonia::clean_text(&user_request.message),
        }).to_string(),
    };
    let lambda_request = Request {
        commands: Command::SendIndividualCustomReplyTo(template, "support".to_string()),
        email: user_request.email,
    };
    let _ = appstate.lambda_client
                            .invoke()
                            .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                            .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                            .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                            .send()
                            .await;

    Ok(())
}

