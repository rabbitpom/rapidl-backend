use serde::Deserialize;
use garde::Validate;
use crate::db_schema::hooked_sql_types::{SupportWhoAreYou, SupportTicketState};
use diesel::prelude::*;
use crate::Schema::{supporttickets, supportticketmessages};
use chrono::NaiveDateTime;

#[derive(Deserialize, Debug, Validate)]
pub struct RequestPayload {
    #[garde(skip)]
    pub whoami: SupportWhoAreYou,
    #[garde(email, length(max=320))]
    pub email: String,
    #[garde(length(min=3, max=16))]
    pub name: String,
    #[garde(length(min=20, max=500))] // replies are subject to email service limits, but i set 500
                              // here to ensure initial messages are short
    pub message: String,
}

#[derive(Insertable)]
#[diesel(table_name = supporttickets)]
#[allow(non_snake_case)]
pub struct SupportTicket<'a> {
    pub name: &'a str,
    pub summary: &'a str,
    pub email: &'a str,
    pub wau: SupportWhoAreYou,
    pub state: SupportTicketState,
    pub claimedby: Option<i64>,
    pub claimedbyname: Option<String>,
    pub createdat: NaiveDateTime,
    pub lastchanged: NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = supportticketmessages)]
#[allow(non_snake_case)]
pub struct SupportTicketMessage<'a> {
    pub ticketid: i32,
    pub message: &'a str,
    pub createdat: NaiveDateTime,
    pub isteam: bool,
}
