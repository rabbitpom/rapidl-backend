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
use diesel::sql_types::{BigInt, Integer};
use diesel::prelude::*;
use diesel::sql_query;
use diesel_async::RunQueryDsl;

use crate::{
    Schema::supporttickets,
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Middleware::validate_access_auth::AccessTokenDescription,
    DB::SupportTicket,
    db_schema::hooked_sql_types::{SupportWhoAreYou, SupportTicketState},
};

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
    ticket_claimed_by: String,
    #[serde(rename = "ticketStatus")]
    ticket_status: SupportTicketState,
    #[serde(rename = "ticketShortMessage")]
    ticket_short_message: String,
    #[serde(rename = "ticketOpenedAt")]
    ticket_opened_at: NaiveDateTime,
    #[serde(rename = "ticketLastChanged")]
    ticket_last_changed: NaiveDateTime,
}

impl TicketPayload {
    fn new(ticket: SupportTicket) -> Self {
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
        let claimed_by;
        {
            match (ticket.claimedby, ticket.claimedbyname) {
                (Some(claimedby_id), Some(claimedby_name)) => {
                    claimed_by = format!("{claimedby_name} ({claimedby_id})");
                },
                (_, _) => claimed_by = "None".to_string(),
            }
        }

        Self {
            ticket_id: ticket.id,
            ticket_name: ticket.name,
            ticket_wau: ticket.wau,
            ticket_status: ticket.state,
            ticket_email: email,
            ticket_claimed_by: claimed_by,
            ticket_short_message: ticket.summary,
            ticket_opened_at: ticket.createdat,
            ticket_last_changed: ticket.lastchanged,
        }
    }
}

#[derive(Serialize)]
pub struct GroupPayload {
    content: Vec<TicketPayload>,
    total_pages: Option<usize>,
}

mod db;
use db::Pagination;

// GET API endpoint
#[tracing::instrument(skip(access_token, appstate, pagination), fields(UserId=%access_token.user_id,request="/admin/support/list-ticket",page=%pagination.page))]
pub async fn request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(pagination): Query<Pagination>) -> Result<Json<GroupPayload>, ServerResponse> {
    if !access_token.has_support_privilege {
        return Err(status_response(StatusCode::UNAUTHORIZED, "Not Authorised"));
    }

    let tickets: Vec<SupportTicket>;
    let mut total_tickets = None;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        
        match pagination.get_claimed_only {
            true => {
                tickets = sql_query("SELECT id, name, summary, email, wau, state, claimedbyname, claimedby, createdat, lastchanged FROM (SELECT id, name, summary, email, wau, state, claimedbyname, claimedby, createdat, lastchanged, ROW_NUMBER() OVER (ORDER BY id) AS row_num FROM supporttickets WHERE claimedby = $1) AS subquery WHERE row_num BETWEEN (($2 - 1) * $3 + 1) AND ($2 * $3)")
                        .bind::<BigInt, _>(access_token.user_id)
                        .bind::<Integer, _>(pagination.page as i32)
                        .bind::<Integer, _>(10)
                        .load(&mut conn)
                        .await.map_err(|err| {
                            tracing::error!("Failed to query page {}, with page size, 10, due to {err}", pagination.page);
                            internal_server_error("Internal Service Error")
                        })?;

                if pagination.get_total_pages {
                    total_tickets = Some(
                        supporttickets::table.filter(supporttickets::claimedby.eq(&access_token.user_id))
                                    .count()
                                    .get_result::<i64>(&mut conn)
                                    .await.map_err(|err| {
                                        tracing::error!("Failed to query total page size due to {err}");
                                        internal_server_error("Internal Service Error")
                                    })? as usize
                    );
                }
            },
            false => {
                tickets = sql_query("SELECT id, name, summary, email, wau, state, claimedbyname, claimedby, createdat, lastchanged FROM (SELECT id, name, summary, email, wau, state, claimedbyname, claimedby, createdat, lastchanged, ROW_NUMBER() OVER (ORDER BY id) AS row_num FROM supporttickets) AS subquery WHERE row_num BETWEEN (($1 - 1) * $2 + 1) AND ($1 * $2)")
                        .bind::<Integer, _>(pagination.page as i32)
                        .bind::<Integer, _>(10)
                        .load(&mut conn)
                        .await.map_err(|err| {
                            tracing::error!("Failed to query page {}, with page size, 10, due to {err}", pagination.page);
                            internal_server_error("Internal Service Error")
                        })?;

                if pagination.get_total_pages {
                    total_tickets = Some(
                        supporttickets::table.count()
                                    .get_result::<i64>(&mut conn)
                                    .await.map_err(|err| {
                                        tracing::error!("Failed to query total page size due to {err}");
                                        internal_server_error("Internal Service Error")
                                    })? as usize
                    );
                }
            },
        }
    }
    
    let tickets_payload = tickets.into_iter().map(|ticket| {
        TicketPayload::new(ticket)
    }).collect::<Vec<TicketPayload>>();

    Ok(Json(GroupPayload {
        total_pages: match total_tickets {
            None => None,
            Some(total_tickets) => {
                Some((total_tickets as f64 / (10) as f64).ceil() as usize)
            }
        },
        content: tickets_payload,
    }))
}

