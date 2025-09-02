use ::std::sync::Arc;
use ::tokio::sync::Mutex;
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
use diesel_async::RunQueryDsl;
use garde::Validate;
use diesel_async::scoped_futures::ScopedFutureExt;
use chrono::Utc;
use rustrict::CensorStr;
use summarizer::summarize;
use unicode_normalization::UnicodeNormalization;
use serde_json::json;
use common_types::SESContacts::{
    Request,
    SendIndividual,
    Command,
};

use crate::{
    Schema::{users, supporttickets, supportticketmessages},
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Middleware::validate_access_auth::AccessTokenDescription,
    DB::{SupportTicket, SupportTicketMessage, UserQueryResult},
    db_schema::hooked_sql_types::{SupportWhoAreYou, SupportTicketState},
    Constants,
};

pub mod db;
use db::{TicketRequest, PutTicketRequest, PutTicketMode, PostMessagePayload, InsertableSupportTicketMessage};

#[derive(Serialize)]
pub struct TicketMessage {
    #[serde(rename = "messageId")]
    message_id: i32,
    message: String,
    #[serde(rename = "createdAt")]
    created_at: NaiveDateTime,
    #[serde(rename = "isTeam")]
    is_team: bool,
}

#[derive(Serialize)]
pub struct TicketPayload {
    #[serde(rename = "ticketId")]
    ticket_id: i32,
    #[serde(rename = "ticketName")]
    ticket_name: String,
    #[serde(rename = "ticketWAU")]
    ticket_wau: SupportWhoAreYou,
    #[serde(rename = "ticketEmail")]
    ticket_email: String,
    #[serde(rename = "ticketClaimedBy")]
    ticket_claimed_by: Option<i64>,
    #[serde(rename = "ticketClaimedByName")]
    ticket_claimed_by_name: Option<String>,
    #[serde(rename = "ticketStatus")]
    ticket_status: SupportTicketState,
    #[serde(rename = "ticketOpenedAt")]
    ticket_opened_at: NaiveDateTime,
    #[serde(rename = "ticketLastChanged")]
    ticket_last_changed: NaiveDateTime,
    #[serde(rename = "ticketMessages")]
    ticket_messages: Vec<TicketMessage>,
}

impl Into<TicketMessage> for SupportTicketMessage {
    fn into(self) -> TicketMessage {
        TicketMessage {
            message_id: self.id,
            message: self.message,
            created_at: self.createdat,
            is_team: self.isteam,
        }
    }
}

impl TicketPayload {
    fn new(ticket: SupportTicket, messages: Vec<SupportTicketMessage>) -> Self {
        let email;
        {
            email = ticket.email.find('@')
                                .map(|pos| {
                                    if pos > 3 {
                                        format!("{}***{}", &ticket.email[..3], &ticket.email[pos..])
                                    } else {
                                        format!("***{}", &ticket.email[pos..])
                                    }
                                })
                                .unwrap_or_else(|| ticket.email);
        }
        Self {
            ticket_id: ticket.id,
            ticket_name: ticket.name,
            ticket_wau: ticket.wau,
            ticket_email: email,
            ticket_claimed_by: ticket.claimedby,
            ticket_claimed_by_name: ticket.claimedbyname,
            ticket_status: ticket.state,
            ticket_opened_at: ticket.createdat,
            ticket_last_changed: ticket.lastchanged,
            ticket_messages: messages.into_iter().map(|v| v.into()).collect(),
        }
    }
}

// GET API endpoint
#[tracing::instrument(skip(access_token, appstate, ticket_request), fields(UserId=%access_token.user_id,request="GET /admin/support/ticket",ticket_id=%ticket_request.ticket_id))]
pub async fn get_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(ticket_request): Query<TicketRequest>) -> Result<Json<TicketPayload>, ServerResponse> {
    if !access_token.has_support_privilege {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Not Authorised"));
    }
    let mut conn = appstate.postgres.get().await.map_err(|err| {
        tracing::error!("Failed to fetch Postgres connection, {err}");
        internal_server_error("Internal Service Error")
    })?;

    let ticket = match supporttickets::table.filter(supporttickets::id.eq(ticket_request.ticket_id))
                                    .select(SupportTicket::as_select())
                                    .first(&mut conn)
                                    .await {
                                        Ok(ticket) => ticket,
                                        Err(err) => match err {
                                            diesel::result::Error::NotFound => return Err(status_response(StatusCode::NOT_FOUND, "No such ticket")),
                                            _ => {
                                                tracing::error!("Failed to fetch ticket record due to {err}");
                                                return Err(internal_server_error("Internal Service Error"));
                                            },
                                        },
                                    };

    let ticket_messages = supportticketmessages::table.filter(supportticketmessages::ticketid.eq(ticket_request.ticket_id)) 
                                    .select(SupportTicketMessage::as_select())
                                    .load(&mut conn)
                                    .await
                                    .map_err(|err| {
                                        tracing::error!("Failed to fetch ticket messages for {} due to {err}", ticket_request.ticket_id);
                                        internal_server_error("Internal Service Error")
                                    })?;

    Ok(Json(TicketPayload::new(ticket, ticket_messages)))
}

// DELETE API endpoint
#[tracing::instrument(skip(access_token, appstate, ticket_request), fields(UserId=%access_token.user_id,request="DELETE /admin/support/ticket",ticket_id=%ticket_request.ticket_id))]
pub async fn delete_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(ticket_request): Query<TicketRequest>) -> Result<(), ServerResponse> {
    if !access_token.has_support_privilege {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Not Authorised"));
    }
    let mut conn = appstate.postgres.get().await.map_err(|err| {
        tracing::error!("Failed to fetch Postgres connection, {err}");
        internal_server_error("Internal Service Error")
    })?;
    let _ = conn.build_transaction()
                        .serializable()
                        .run::<_, diesel::result::Error, _>(|conn| async move {
                            let ticket = supporttickets::table.filter(supporttickets::id.eq(ticket_request.ticket_id))
                                                                .select(SupportTicket::as_select())
                                                                .for_update()
                                                                .first(conn)
                                                                .await?;
                            let Some(claimedby) = ticket.claimedby else {
                                tracing::error!("Ticket {} has invalid state : attempted to delete ticket but it has no claimedby field", ticket_request.ticket_id);
                                return Err(diesel::result::Error::RollbackTransaction);
                            };
                            if claimedby != access_token.user_id {
                                return Err(diesel::result::Error::RollbackTransaction);
                            }
                            match ticket.state {
                                SupportTicketState::Closed => (),
                                _ => {
                                    tracing::error!("Failed to delete ticket because it's not in closed state");
                                    return Err(diesel::result::Error::RollbackTransaction);
                                },
                            }
                            let _ = diesel::delete(supporttickets::table.filter(supporttickets::id.eq(ticket_request.ticket_id))).execute(conn).await?;
                            Ok::<(),_>(())
                        }.scope_boxed())
                        .await
                        .map_err(|err| {
                            tracing::error!("Transaction error: {err}");
                            internal_server_error("Internal Service Error")
                        })?;
    Ok(())
}


// PUT API endpoint
#[tracing::instrument(skip(access_token, appstate, ticket_request), fields(UserId=%access_token.user_id,request="PUT /admin/support/ticket",ticket_id=%ticket_request.ticket_id,mode=%ticket_request.mode))]
pub async fn put_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(ticket_request): Query<PutTicketRequest>) -> Result<(), ServerResponse> {
    if !access_token.has_support_privilege {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Not Authorised"));
    }

    struct TransactionSuccess {
        target: String,
        name: String,
        email: String,
    }

    enum TransactionCommand {
        None,
        Success(TransactionSuccess),
        TicketIsClosed,
        TicketMustBeClaimedFirst,
        TicketClaimedBySomeoneElse,
        InvalidTicketState,
    }

    let mut conn = appstate.postgres.get().await.map_err(|err| {
        tracing::error!("Failed to fetch Postgres connection, {err}");
        internal_server_error("Internal Service Error")
    })?;
    let user: UserQueryResult = users::table.filter(users::userid.eq(&access_token.user_id)).first(&mut conn).await.map_err(|err| {
        tracing::info!("No matching UserId, {err}");
        status_response(StatusCode::BAD_REQUEST, "No matching UserId")
    })?;

    // an extra check ig
    if !user.supportprivilege {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Not Authorised"));
    }

    let transaction_command = Arc::new(Mutex::new(TransactionCommand::None));
    {
        let transaction_command = Arc::clone(&transaction_command);
        let result = conn.build_transaction()
                        .serializable()
                        .run::<_, diesel::result::Error, _>(|conn| async move {
                            let utc = Utc::now().naive_utc();
                            let ticket = supporttickets::table.filter(supporttickets::id.eq(ticket_request.ticket_id))
                                                                .select(SupportTicket::as_select())
                                                                .for_update()
                                                                .first(conn)
                                                                .await?;
                            match (ticket.state, &ticket_request.mode) {
                                (SupportTicketState::Unclaimed, PutTicketMode::Unclaim) => {
                                    // by default transaction command is none
                                    return Ok(());
                                },
                                (SupportTicketState::Unclaimed, PutTicketMode::Claim) => (),
                                (SupportTicketState::Claimed, PutTicketMode::Claim) => match ticket.claimedby {
                                    Some(claimedby) => {
                                        if claimedby == access_token.user_id {
                                            // by default transaction command is none
                                            return Ok(());
                                        }
                                        *transaction_command.lock().await = TransactionCommand::TicketClaimedBySomeoneElse;
                                        return Ok(());
                                    },
                                    None => {
                                        tracing::error!("Ticket {} has invalid state : attempted to claim a claimed ticket but it has no claimedby field", ticket_request.ticket_id);
                                        *transaction_command.lock().await = TransactionCommand::InvalidTicketState;
                                        return Ok(());
                                    },
                                },
                                (SupportTicketState::Claimed, PutTicketMode::Unclaim) => match ticket.claimedby {
                                    Some(claimedby) => {
                                        if claimedby != access_token.user_id {
                                            *transaction_command.lock().await = TransactionCommand::TicketClaimedBySomeoneElse;
                                            return Ok(());
                                        }
                                    },
                                    None => {
                                        tracing::error!("Ticket {} has invalid state : attempted to unclaim a claimed ticket but it has no claimedby field", ticket_request.ticket_id);
                                        *transaction_command.lock().await = TransactionCommand::InvalidTicketState;
                                        return Ok(());

                                    },
                                },
                                (SupportTicketState::Claimed, PutTicketMode::Close) => match ticket.claimedby {
                                    Some(claimedby) => {
                                        if claimedby != access_token.user_id {
                                            *transaction_command.lock().await = TransactionCommand::TicketClaimedBySomeoneElse;
                                            return Ok(());
                                        }
                                    },
                                    None => {
                                        tracing::error!("Ticket {} has invalid state : attempted to close a claimed ticket but it has no claimedby field", ticket_request.ticket_id);
                                        *transaction_command.lock().await = TransactionCommand::InvalidTicketState;
                                        return Ok(());

                                    },
                                },

                                (SupportTicketState::Unclaimed, PutTicketMode::Close) => {
                                    *transaction_command.lock().await = TransactionCommand::TicketMustBeClaimedFirst;
                                    return Ok(());

                                },
                                (SupportTicketState::Closed, PutTicketMode::Close) => {
                                    // by default transaction command is none
                                    return Ok(());
                                },
                                (SupportTicketState::Closed, _) => {
                                    *transaction_command.lock().await = TransactionCommand::TicketIsClosed;
                                    return Ok(());
                                },
                            }

                            match &ticket_request.mode {
                                PutTicketMode::Claim | PutTicketMode::Close => {
                                    let ticket_updated = diesel::update(supporttickets::table.filter(supporttickets::id.eq(ticket_request.ticket_id)))
                                        .set((
                                                supporttickets::claimedby.eq(access_token.user_id),
                                                supporttickets::claimedbyname.eq(user.username),
                                                supporttickets::state.eq(Into::<SupportTicketState>::into(ticket_request.mode)),
                                                supporttickets::lastchanged.eq(utc)
                                        ))
                                        .execute(conn)
                                        .await?;
                                    if ticket_updated != 1 {
                                        tracing::error!("Expected 1 ticket record to be updated, instead got {ticket_updated}, so rolled back");
                                        *transaction_command.lock().await = TransactionCommand::InvalidTicketState;
                                        return Err(diesel::result::Error::RollbackTransaction);
                                    }
                                },
                                PutTicketMode::Unclaim => {
                                    let ticket_updated = diesel::update(supporttickets::table.filter(supporttickets::id.eq(ticket_request.ticket_id)))
                                        .set((
                                                supporttickets::claimedby.eq(None::<i64>),
                                                supporttickets::claimedbyname.eq(None::<String>),
                                                supporttickets::state.eq(Into::<SupportTicketState>::into(ticket_request.mode)),
                                                supporttickets::lastchanged.eq(utc)
                                        ))
                                        .execute(conn)
                                        .await?;
                                    if ticket_updated != 1 {
                                        tracing::error!("Expected 1 ticket record to be updated, instead got {ticket_updated}, so rolled back");
                                        *transaction_command.lock().await = TransactionCommand::InvalidTicketState;
                                        return Err(diesel::result::Error::RollbackTransaction);
                                    }

                                },
                            }

                            *transaction_command.lock().await = TransactionCommand::Success(TransactionSuccess {
                                target: ticket.name,
                                name: ticket.claimedbyname.unwrap_or("".to_string()),
                                email: ticket.email,
                            });
                            Ok::<(),_>(())
                        }.scope_boxed()).await;
        if let Err(err) = result {
            match err {
                diesel::result::Error::RollbackTransaction => (),
                _ => {
                    tracing::error!("Transaction error: {err}");
                    return Err(internal_server_error("Internal Service Error"));
                },
            }
        }
    }

    let command = &*transaction_command.lock().await;
    match command {
        TransactionCommand::None => Ok(()),
        TransactionCommand::Success(info) => {
            if let PutTicketMode::Close = ticket_request.mode {
                let template = SendIndividual {
                    template_name: "supportticketclosed".to_string(),
                    template_data: json!({
                        "ticketid": format!("#{}", ticket_request.ticket_id),
                        "supportname": &info.name,
                        "name": &info.target,
                    }).to_string(),
                };
                let lambda_request = Request {
                    commands: Command::SendIndividualCustomReplyTo(template, "support".to_string()),
                    email: info.email.clone(),
                };
                let _ = appstate.lambda_client
                                        .invoke()
                                        .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                                        .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                                        .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                                        .send()
                                        .await;
            }
            Ok(())
        }
        TransactionCommand::TicketIsClosed => Err(status_response(StatusCode::LOCKED, "Ticket is closed and cannot be modified")),
        TransactionCommand::TicketMustBeClaimedFirst => Err(status_response(StatusCode::BAD_REQUEST, "Ticket must be claimed first")),
        TransactionCommand::TicketClaimedBySomeoneElse => Err(status_response(StatusCode::CONFLICT, "Ticket has already been claimed by someone else")),
        TransactionCommand::InvalidTicketState => Err(status_response(StatusCode::CONFLICT, "Ticket has invalid state, try again later or contact developer")),
    }
}

// POST API endpoint for message
#[tracing::instrument(skip(access_token, appstate, request), fields(UserId=%access_token.user_id,request="POST /admin/support/ticket/message",ticket_id=%request.ticket_id))]
pub async fn post_message_request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Json(mut request): Json<PostMessagePayload>) -> Result<(), ServerResponse> {
    if !access_token.has_support_privilege {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Not Authorised"));
    }
    let validation_result = request.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    request.message = request.message.nfkc().collect();
    if request.message.is_inappropriate() {
        return Err(status_response(StatusCode::BAD_REQUEST, "Message is inappropriate, please write a different message"));
    }
    let mut message_summary = match request.message.len() > 50 {
        true => summarize(request.message.as_str(), 0.3),
        false => request.message.clone(),
    };
    message_summary.truncate(100);

    struct TransactionSuccess {
        target: String,
        name: String,
        message: String,
        email: String,
    }
    enum TransactionCommand {
        None,
        TicketIsClosed,
        TicketMustBeClaimed,
        UnexpectedUpdatedRows,
        Success(TransactionSuccess),
    }

    let transaction_command = Arc::new(Mutex::new(TransactionCommand::None));
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;

        let transaction_command = Arc::clone(&transaction_command);
        let result = conn.build_transaction()
                    .serializable()
                    .run::<_, diesel::result::Error, _>(|conn| async move {
                        let utc = Utc::now().naive_utc();
                        let ticket = supporttickets::table.filter(supporttickets::id.eq(request.ticket_id))
                                                            .select(SupportTicket::as_select())
                                                            .for_update()
                                                            .first(conn)
                                                            .await?;

                        if let SupportTicketState::Closed = ticket.state {
                            *transaction_command.lock().await = TransactionCommand::TicketIsClosed;
                            return Err(diesel::result::Error::RollbackTransaction);
                        }
                        if let SupportTicketState::Unclaimed = ticket.state {
                            *transaction_command.lock().await = TransactionCommand::TicketMustBeClaimed;
                            return Err(diesel::result::Error::RollbackTransaction);
                        }
                        let (Some(claimedby), Some(claimedbyname)) = (ticket.claimedby, ticket.claimedbyname) else {
                            *transaction_command.lock().await = TransactionCommand::TicketMustBeClaimed;
                            return Err(diesel::result::Error::RollbackTransaction);
                        };
                        if claimedby != access_token.user_id {
                            *transaction_command.lock().await = TransactionCommand::TicketMustBeClaimed;
                            return Err(diesel::result::Error::RollbackTransaction);
                        }
                        let ticket_updated = diesel::update(supporttickets::table.filter(supporttickets::id.eq(request.ticket_id)))
                                        .set((
                                                supporttickets::summary.eq(message_summary),
                                                supporttickets::lastchanged.eq(utc)
                                        ))
                                        .execute(conn)
                                        .await?;
                        if ticket_updated != 1 {
                            *transaction_command.lock().await = TransactionCommand::UnexpectedUpdatedRows;
                            return Err(diesel::result::Error::RollbackTransaction);
                        }
                        let ticket_message_added = diesel::insert_into(supportticketmessages::table)
                            .values(&InsertableSupportTicketMessage {
                                    ticketid: request.ticket_id,
                                    message: &request.message,
                                    createdat: utc,
                                    isteam: true,
                                })
                            .execute(conn).await?;
                        if ticket_message_added != 1 {
                            *transaction_command.lock().await = TransactionCommand::UnexpectedUpdatedRows;
                            return Err(diesel::result::Error::RollbackTransaction);
                        }

                        *transaction_command.lock().await = TransactionCommand::Success(TransactionSuccess {
                            target: ticket.name,
                            name: claimedbyname,
                            message: request.message,
                            email: ticket.email,
                        });

                        Ok::<(),_>(())
                    }.scope_boxed()).await;

        match result {
            Ok(_) => (),
            Err(err) => match err {
                diesel::result::Error::RollbackTransaction => (),
                _ => {
                    tracing::error!("Transaction error: {err}");
                    return Err(internal_server_error("Internal Service Error"));
                },
            },
        }
    }
    let command = &*transaction_command.lock().await;
    match command {
        TransactionCommand::None => Err(internal_server_error("None")),
        TransactionCommand::UnexpectedUpdatedRows => {
            tracing::error!("Failed to process ticket message because received unexpected number of updated rows");
            Err(internal_server_error("Internal Service Error"))
        },
        TransactionCommand::TicketIsClosed => Err(status_response(StatusCode::BAD_REQUEST, "No further operations to ticket is possible because it is closed")),
        TransactionCommand::TicketMustBeClaimed => Err(status_response(StatusCode::BAD_REQUEST, "Ticket must be in claimed state or must be claimed by sender")),
        TransactionCommand::Success(info) => {
            let template = SendIndividual {
                template_name: "supportticket".to_string(),
                template_data: json!({
                    "ticketid": format!("#{}", request.ticket_id),
                    "message": ammonia::clean_text(&info.message),
                    "supportname": &info.name,
                    "name": &info.target,
                }).to_string(),
            };
            let lambda_request = Request {
                commands: Command::SendIndividualCustomReplyTo(template, "support".to_string()),
                email: info.email.clone(),
            };
            let _ = appstate.lambda_client
                                    .invoke()
                                    .function_name(&*Constants::LAMBDA_EMAIL_ARN)
                                    .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
                                    .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_string(&lambda_request).unwrap()))
                                    .send()
                                    .await;
            Ok(())
        },
    }
}
