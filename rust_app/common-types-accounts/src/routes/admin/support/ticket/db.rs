use serde::Deserialize;
use garde::Validate;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use crate::Schema::supportticketmessages;
use db_schema::hooked_sql_types::SupportTicketState;

#[derive(Debug, Copy, Clone, Deserialize)]
pub enum PutTicketMode {
    Claim,
    Unclaim,
    Close,
}

impl Into<SupportTicketState> for PutTicketMode {
    fn into(self) -> SupportTicketState {
        match self {
            PutTicketMode::Claim => SupportTicketState::Claimed,
            PutTicketMode::Unclaim => SupportTicketState::Unclaimed,
            PutTicketMode::Close => SupportTicketState::Closed,
        }
    }
}

impl ::std::fmt::Display for PutTicketMode {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Deserialize)]
pub struct PutTicketRequest {
    #[serde(rename = "ticketId")]
    pub ticket_id: i32,
    pub mode: PutTicketMode,
}

#[derive(Deserialize)]
pub struct TicketRequest {
    #[serde(rename = "ticketId")]
    pub ticket_id: i32,
}

#[derive(Deserialize, Debug, Validate)]
pub struct PostMessagePayload {
    #[serde(rename = "ticketId")]
    #[garde(skip)]
    pub ticket_id: i32,
    #[garde(length(min=20, max=500))] // replies are subject to email service limits, but i set 500
                              // here to ensure initial messages are short
    pub message: String,
}

#[derive(Insertable)]
#[diesel(table_name = supportticketmessages)]
#[allow(non_snake_case)]
pub struct InsertableSupportTicketMessage<'a> {
    pub ticketid: i32,
    pub message: &'a str,
    pub createdat: NaiveDateTime,
    pub isteam: bool,
}
