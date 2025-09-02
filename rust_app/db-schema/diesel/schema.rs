// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "generation_status"))]
    pub struct GenerationStatus;

    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "supportticketstate"))]
    pub struct Supportticketstate;

    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "supportwhoareyou"))]
    pub struct Supportwhoareyou;
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
    use super::sql_types::GenerationStatus;

    generation (id) {
        userid -> Nullable<Int8>,
        id -> Int8,
        status -> Nullable<GenerationStatus>,
        createdat -> Timestamp,
        finishedon -> Nullable<Timestamp>,
        jobid -> Nullable<Uuid>,
        creditsused -> Nullable<Int2>,
        category -> Nullable<Varchar>,
        options -> Nullable<Text>,
        displayname -> Nullable<Text>,
    }
}

diesel::table! {
    supportticketmessages (id) {
        id -> Int4,
        ticketid -> Int4,
        message -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Supportwhoareyou;
    use super::sql_types::Supportticketstate;

    supporttickets (id) {
        id -> Int4,
        name -> Text,
        #[max_length = 320]
        email -> Varchar,
        wau -> Nullable<Supportwhoareyou>,
        state -> Supportticketstate,
        claimedbyname -> Nullable<Text>,
        claimedby -> Nullable<Int8>,
    }
}

diesel::table! {
    users (userid) {
        userid -> Int8,
        #[max_length = 16]
        username -> Varchar,
        #[max_length = 320]
        email -> Varchar,
        emailverified -> Nullable<Bool>,
        bcryptpass -> Nullable<Bytea>,
        createdat -> Nullable<Timestamp>,
        supportprivilege -> Nullable<Bool>,
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
