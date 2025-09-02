pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "generation_status"))]
    pub struct GenerationStatusMapping;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "supportwhoareyou"))]
    pub struct SupportWhoAreYouMapping;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "supportticketstate"))]
    pub struct SupportTicketStateMapping;
}

pub mod hooked_sql_types {
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Clone, diesel_derive_enum::DbEnum, serde::Serialize)]
    #[ExistingTypePath = "crate::sql_types::GenerationStatusMapping"]
    pub enum GenerationStatus {
        Working,
        Success,
        Failed,
        Deleting,
        Waiting,
    }

    #[derive(Deserialize, Debug, PartialEq, Clone, diesel_derive_enum::DbEnum, serde::Serialize)]
    #[ExistingTypePath = "crate::sql_types::SupportWhoAreYouMapping"]
    pub enum SupportWhoAreYou {
        Student,
        Teacher,
        Company,
        Organisation,
        Unknown,
    }

    #[derive(Debug, PartialEq, Clone, diesel_derive_enum::DbEnum, serde::Serialize)]
    #[ExistingTypePath = "crate::sql_types::SupportTicketStateMapping"]
    pub enum SupportTicketState {
        Unclaimed,
        Claimed,
        Closed,
    }

}

diesel::table! {
    allocatedcredits (creditid) {
        creditid -> Int4,
        userid -> Int8,
        credits -> Int4,
        expireat -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::GenerationStatusMapping;

    generation (id) {
        userid -> Int8,
        id -> Int8,
        status -> GenerationStatusMapping,
        createdat -> Timestamp,
        finishedon -> Nullable<Timestamp>,
        jobid -> Uuid,
        displayname -> Text,
        options -> Text,
        category -> Varchar,
        creditsused -> SmallInt,
    }
}

diesel::table! {
    users (userid) {
        userid -> Int8,
        #[max_length = 16]
        username -> Varchar,
        #[max_length = 320]
        email -> Varchar,
        emailverified -> Bool,
        bcryptpass -> Bytea,
        createdat -> Timestamp,
        supportprivilege -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    supportticketmessages (id) {
        id -> Int4,
        ticketid -> Int4,
        message -> Text,
        createdat -> Timestamp,
        isteam -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    problematicemails (hash) {
        hash -> Text,
        count -> Int4,
        nextreset -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SupportWhoAreYouMapping;
    use super::sql_types::SupportTicketStateMapping;

    supporttickets (id) {
        id -> Int4,
        name -> Text,
        summary -> Text,
        #[max_length = 320]
        email -> Varchar,
        wau -> SupportWhoAreYouMapping,
        state -> SupportTicketStateMapping,
        claimedbyname -> Nullable<Text>,
        claimedby -> Nullable<Int8>,
        createdat -> Timestamp,
        lastchanged -> Timestamp,
    }
}

diesel::joinable!(allocatedcredits -> users (userid));
diesel::joinable!(generation -> users (userid));
diesel::joinable!(supportticketmessages -> supporttickets (ticketid));
diesel::joinable!(supporttickets -> users (claimedby));

diesel::allow_tables_to_appear_in_same_query!(
    allocatedcredits,
    generation,
    supportticketmessages,
    supporttickets,
    users,
);
