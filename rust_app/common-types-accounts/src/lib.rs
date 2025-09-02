use common_types;
use db_schema;

pub type E = Box<dyn ::std::error::Error + Send + Sync + 'static>;

mod routes;
mod middleware;

#[allow(non_snake_case)]
pub mod Routes {
    pub use crate::routes::*;
}

#[allow(non_snake_case)]
pub mod Middleware {
    pub use crate::middleware::*;
}

#[allow(non_snake_case)]
pub mod Schema {
    pub use crate::db_schema::*;
}

#[allow(non_snake_case)]
pub mod Credits {
    use deadpool_redis::{
        redis::{cmd, pipe},
        Connection as RedisConnection,
    };
    use chrono::{Utc, TimeDelta, NaiveDateTime, DateTime};
    use diesel::prelude::*;
    use diesel::dsl::{min, sum};
    use diesel_async::{
        RunQueryDsl,
        scoped_futures::ScopedFutureExt
    };

    use crate::{
        State::AppState,
        DB::UserCreditsQueryResult,
        Schema::allocatedcredits,
        Routes::verify::db::InsertableAllocatedCredits,
    };

    type PostgresConnection = diesel_async::pooled_connection::deadpool::Object<diesel_async::AsyncPgConnection>;

    #[derive(Debug)]
    pub enum FetchError {
        FailedToObtainRedisConnection,
        FailedToObtainDatabaseConnection,
        FailedToQueryRedis,
        DatabaseQueryFailure,
        FailedToSetPipeline,
    }

    async fn query_credits_result_with_conn(utc: NaiveDateTime, user_id: i64, mut redis_conn: RedisConnection, mut postgres_conn: PostgresConnection) -> Result<(i64, NaiveDateTime, RedisConnection, PostgresConnection), FetchError> {
        let record;
        {
            record = allocatedcredits::table.select(
                    (
                        sum(allocatedcredits::credits),
                        min(allocatedcredits::expireat),
                    )
                )
                .filter(allocatedcredits::userid.eq(user_id).and(allocatedcredits::expireat.gt(utc)))
                .first(&mut postgres_conn).await.or(Err(FetchError::DatabaseQueryFailure))?;
        }
        let UserCreditsQueryResult(Some(credits), Some(expire)) = record else {
            return Ok((0, NaiveDateTime::default(), redis_conn, postgres_conn))
        };

        // Convert MySQL TIMESTAMP to an i64
        let expire_secs = expire.and_utc().timestamp();
        let mut redis_expire_seconds = expire_secs - utc.and_utc().timestamp();
        if redis_expire_seconds < 0 {
            redis_expire_seconds = 1;
        }

        let credits_key = format!("user:{user_id}:cred:t");
        let expire_key = format!("user:{user_id}:cred:e");
        let pipe_result = pipe()
            .cmd("SET").arg(&[&credits_key, &credits.to_string(), "EX", &redis_expire_seconds.to_string()]).ignore()
            .cmd("SET").arg(&[&expire_key, &expire_secs.to_string(), "EX", &redis_expire_seconds.to_string()]).ignore()
            .query_async::<_, ()>(&mut redis_conn).await;

        if let Err(err) = pipe_result {
            tracing::warn!("Failed to set multiple keys through pipeline, {err}");
            return Err(FetchError::FailedToSetPipeline)
        }

        Ok((credits, expire, redis_conn, postgres_conn))

    }

    async fn query_credits_result(utc: NaiveDateTime, appstate: &AppState, user_id: i64, redis_conn: RedisConnection) -> Result<(i64, NaiveDateTime), FetchError> {
        let postgres_conn = appstate.postgres.get().await.map_err(|err|{
            tracing::error!("Failed to fetch Postgres conection, {err}");
            FetchError::FailedToObtainDatabaseConnection
        })?;
        match query_credits_result_with_conn(utc, user_id, redis_conn, postgres_conn).await {
            Err(err) => Err(err),
            Ok((credits, expire, _, _)) => {
                Ok((credits, expire))
            }
        }
    }

    pub enum IncrementTotalCreditsError {
        RedisConnectionOpenFailure,
        PostgresConnectionOpenFailure,
        NotEnoughCredits,
        PostgresOperationFailure,
        RedisOperationFailure,
        PostgresTransactionFailure,
        BadData,
    }
    impl ::std::fmt::Display for IncrementTotalCreditsError {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
            match *self {
                IncrementTotalCreditsError::RedisConnectionOpenFailure => {
                    write!(f, "Failed to open Redis connection")
                }
                IncrementTotalCreditsError::PostgresConnectionOpenFailure => {
                    write!(f, "Failed to open Postgres connection")
                }
                IncrementTotalCreditsError::NotEnoughCredits => {
                    write!(f, "Not enough credits")
                }
                IncrementTotalCreditsError::PostgresOperationFailure => {
                    write!(f, "Postgres operation failed")
                }
                IncrementTotalCreditsError::RedisOperationFailure => {
                    write!(f, "Redis operation failed")
                }
                IncrementTotalCreditsError::PostgresTransactionFailure => {
                    write!(f, "Postgres transaction failed")
                }
                IncrementTotalCreditsError::BadData => {
                    write!(f, "Bad data")
                }
            }
        }
    }
    pub async fn increment_total_credits(appstate: AppState, user_id: i64, amount: i32, duration: i64, redis_conn: Option<RedisConnection>, postgres_conn: Option<PostgresConnection>) -> Result<(), IncrementTotalCreditsError> {
        let mut redis_conn = match redis_conn {
            Some(conn) => conn,
            None => {
                let redis_conn = appstate.redis.get().await.map_err(|err| {
                    tracing::error!("Failed to fetch Redis connection: {}", err);
                    IncrementTotalCreditsError::RedisConnectionOpenFailure
                })?;
                redis_conn
            }
        };
        let mut postgres_conn = match postgres_conn {
            Some(conn) => conn,
            None => {
                let postgres_conn = appstate.postgres.get().await.map_err(|err| {
                    tracing::error!("Failed to fetch Postgres connection: {}", err);
                    IncrementTotalCreditsError::PostgresConnectionOpenFailure
                })?;
                postgres_conn
            }
        };
        let expireat = Utc::now().checked_add_signed(TimeDelta::new(duration,0).unwrap()).unwrap().naive_utc();
        {
            let _ = diesel::insert_into(allocatedcredits::table)
                        .values(&InsertableAllocatedCredits {
                            credits: amount,
                            userid: user_id,
                            expireat,
                        })
                        .execute(&mut postgres_conn)
                        .await.map_err(|err| {
                            tracing::error!("Increment credits Postgres failure: {}", err);
                            IncrementTotalCreditsError::PostgresOperationFailure
                        })?;
        }
        let credits_key = format!("user:{user_id}:cred:t");
        let expire_key = format!("user:{user_id}:cred:e");
        let pipe_result = pipe()
            .cmd("DEL").arg(&[&credits_key]).ignore()
            .cmd("DEL").arg(&[&expire_key]).ignore()
            .query_async::<_, ()>(&mut redis_conn).await;
        if let Err(err) = pipe_result {
            tracing::error!("Increment credits Redis failure: {}", err);
            return Err(IncrementTotalCreditsError::RedisOperationFailure)
        }
        Ok(())
    }

    pub async fn decrement_total_credits(appstate: AppState, user_id: i64, amount: i32, redis_conn: Option<RedisConnection>, postgres_conn: Option<PostgresConnection>) -> Result<(i64, NaiveDateTime), IncrementTotalCreditsError> {
        let redis_conn = match redis_conn {
            Some(conn) => conn,
            None => {
                let redis_conn = appstate.redis.get().await.map_err(|err| {
                    tracing::error!("Failed to fetch Redis connection: {}", err);
                    IncrementTotalCreditsError::RedisConnectionOpenFailure
                })?;
                redis_conn
            }
        };
        let postgres_conn = match postgres_conn {
            Some(conn) => conn,
            None => {
                let postgres_conn = appstate.postgres.get().await.map_err(|err| {
                    tracing::error!("Failed to fetch Postgres connection: {}", err);
                    IncrementTotalCreditsError::PostgresConnectionOpenFailure
                })?;
                postgres_conn
            }
        };
        let ( total_credits, _, mut redis_conn, mut postgres_conn ) = get_total_credits_with_conn(user_id, redis_conn, postgres_conn).await.unwrap();
        if total_credits < amount as i64 {
            return Err(IncrementTotalCreditsError::NotEnoughCredits)
        }
        let mut next_expire_at = None;
        let success = postgres_conn.build_transaction()
                    .read_write()
                    .serializable()
                    .run::<_, diesel::result::Error, _>(|conn| async move {
                        let utc = Utc::now().naive_utc();
                        let credits = allocatedcredits::table
                                            .filter(allocatedcredits::userid.eq(user_id).and(allocatedcredits::expireat.gt(utc)))
                                            .order(allocatedcredits::expireat.asc())
                                            .for_update()
                                            .load::<( i32, i64, i32, NaiveDateTime )>(conn)
                                            .await?;
                        // WARNING: It is not safe to assume total_credits >= amount so we still
                        // repeat checks here (just in individual "chunks")
                        enum Control {
                            DELETE(i32),
                            UPDATE(i32,i32),
                        }
                        let mut drain = amount;
                        let mut to_update = Vec::new();
                        for credit_record in credits.into_iter() {
                            let creditid = credit_record.0;
                            let _userid = credit_record.1;
                            let credits = credit_record.2;
                            let expireat = credit_record.3;
                            if credits < drain {
                                drain -= credits;
                                to_update.push(Control::DELETE(creditid));
                            } else if credits == drain {
                                drain = 0;
                                to_update.push(Control::DELETE(creditid));
                                break;
                            } else {
                                to_update.push(Control::UPDATE(creditid,credits - drain));
                                drain = 0;
                                next_expire_at = Some(expireat.and_utc().timestamp());   // WARNING: This
                                                                                         // is okay to do
                                                                                         // because we
                                                                                         // queried the
                                                                                         // credits in
                                                                                         // order of
                                                                                         // expireat
                                break;
                            }
                        }
                        if drain > 0 {
                            // Should not be reachable but if it is reached we'll exit out from
                            // this operation
                            return Ok::<bool,_>(false)
                        }
                        // Okay, everything confirmed, lets now update each credit record
                        for control in to_update.into_iter() {
                            match control {
                                Control::DELETE(creditid) => {
                                    // The amount deleted should be 1 but it doesn't matter
                                    let _ = diesel::delete(allocatedcredits::table.filter(allocatedcredits::creditid.eq(creditid)))
                                                .execute(conn)
                                                .await?;
                                },
                                Control::UPDATE(creditid,credits) => {
                                    let _ = diesel::update(allocatedcredits::table.filter(allocatedcredits::creditid.eq(creditid)))
                                                .set(allocatedcredits::credits.eq(credits))
                                                .execute(conn)
                                                .await?;
                                },
                            }
                        }
                        Ok::<bool,_>(true)
                    }.scope_boxed()).await.map_err(|err| {
                        tracing::error!("Decrement credits Postgres transaction failure: {}", err);
                        IncrementTotalCreditsError::PostgresTransactionFailure
                    })?;
        if !success {
            return Err(IncrementTotalCreditsError::BadData);
        }
        let credits_key = format!("user:{user_id}:cred:t");
        let expire_key = format!("user:{user_id}:cred:e");
        let pipe_result = pipe()
            .cmd("DEL").arg(&[&credits_key]).ignore()
            .cmd("DEL").arg(&[&expire_key]).ignore()
            .query_async::<_, ()>(&mut redis_conn).await;
        if let Err(err) = pipe_result {
            tracing::error!("Decrement credits Redis failure: {}", err);
            return Err(IncrementTotalCreditsError::RedisOperationFailure)
        }
        return Ok((total_credits - amount as i64, DateTime::from_timestamp(next_expire_at.unwrap_or(1), 0).unwrap().naive_utc()));
    }

    pub async fn get_total_credits_with_conn(user_id: i64, mut redis_conn: RedisConnection, postgres_conn: PostgresConnection) -> Result<(i64, NaiveDateTime, RedisConnection, PostgresConnection), FetchError> {
        let utc = Utc::now().naive_utc();
        let utc_now = utc.and_utc().timestamp();
        let credits_key = format!("user:{user_id}:cred:t");
        let expire_key = format!("user:{user_id}:cred:e");
        let (cached_expire, cached_credits) = match cmd("MGET").arg(&[&expire_key, &credits_key]).query_async::<_, (Option<i64>, Option<i64>)>(&mut redis_conn).await {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("Redis GET command failed, {:?}", err);
                return Err(FetchError::FailedToQueryRedis);
            }
        };
        let Some(cached_expire) = cached_expire else {
            return query_credits_result_with_conn(utc, user_id, redis_conn, postgres_conn).await;
        };
        let Some(cached_credits) = cached_credits else {
            return query_credits_result_with_conn(utc, user_id, redis_conn, postgres_conn).await;
        };
        if cached_expire > utc_now {
            return Ok((cached_credits, DateTime::from_timestamp(cached_expire, 0).unwrap().naive_utc(), redis_conn, postgres_conn));
        }
        query_credits_result_with_conn(utc, user_id, redis_conn, postgres_conn).await
    }

    pub async fn get_total_credits(appstate: &AppState, user_id: i64) -> Result<(i64, NaiveDateTime), FetchError> {
        let mut redis_conn = appstate.redis.get().await.map_err(|err|{
            tracing::error!("Failed to fetch Redis connection, {err}");
            FetchError::FailedToObtainRedisConnection
        })?;

        let utc = Utc::now().naive_utc();
        let utc_now = utc.and_utc().timestamp();
        let credits_key = format!("user:{user_id}:cred:t");
        let expire_key = format!("user:{user_id}:cred:e");
        let (cached_expire, cached_credits) = match cmd("MGET").arg(&[&expire_key, &credits_key]).query_async::<_, (Option<i64>, Option<i64>)>(&mut redis_conn).await {
            Ok(x) => x,
            Err(err) => {
                tracing::error!("Redis GET command failed, {:?}", err);
                return Err(FetchError::FailedToQueryRedis);
            }
        };
        let Some(cached_expire) = cached_expire else {
            return query_credits_result(utc, appstate, user_id, redis_conn).await;
        };
        let Some(cached_credits) = cached_credits else {
            return query_credits_result(utc, appstate, user_id, redis_conn).await;
        };
        if cached_expire > utc_now {
            return Ok((cached_credits, DateTime::from_timestamp(cached_expire, 0).unwrap().naive_utc()));
        }
        query_credits_result(utc, appstate, user_id, redis_conn).await
    }

}

#[allow(non_snake_case)]
pub mod Response {
    use axum::http::StatusCode;

    pub type ServerResponse = (StatusCode, String);

    pub fn status_response<E: ToString>(status: StatusCode, error: E) -> ServerResponse {
        (status, error.to_string())
    }

    pub fn internal_server_error<E: ToString>(err: E) -> ServerResponse {
        status_response(StatusCode::INTERNAL_SERVER_ERROR, err)
    }
}

#[allow(non_snake_case)]
pub mod Auth {
    use ::std::collections::BTreeMap;
    use chrono::{DateTime, Utc};
    use uuid::Uuid;
    use jwt::{SignWithKey, VerifyWithKey};
    use thiserror::Error;

    #[allow(non_camel_case_types)]
    pub struct IGNORE_SET_AUTH_TO_HEADERS;

    pub struct TokenPackage {
        pub utc: i64,
        pub refresh_id: Uuid,
        pub refresh_token: String,
        pub access_token: String,
        pub refresh_expire_format: String,
        pub access_expire_format: String,
    }

    #[derive(Error, Debug)]
    pub enum TokenGenerationError {
        #[error("failed to sign refresh token")]
        SigningFailureRefreshJWTToken,
        #[error("failed to sign access token")]
        SigningFailureAccessJWTToken,
    }

    // merely passed around through code but not exposed directly through API
    pub struct TokenData {
        pub userid: i64,
        pub has_support_privilege: bool,
    }

    pub fn is_timestamp_expired(compare: i64) -> bool {
        Utc::now().timestamp() > compare
    }

    pub fn is_valid_signed_token(token: &str) -> Result<BTreeMap<String, String>, jwt::error::Error> {
        token.verify_with_key(&*crate::Constants::JWT_KEY)
    }

    fn timestamp_to_rfc7231(timestamp: i64) -> String {
        let expiration_time = DateTime::<Utc>::from_timestamp(timestamp, 0).expect("invalid timestamp");
        expiration_time.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
    }

    pub fn gen_refresh_and_access_tokens(ipv6: String, token_data: &TokenData) -> Result<TokenPackage, TokenGenerationError> {
        let jwt_key = &*crate::Constants::JWT_KEY;

        let utc_now = Utc::now();
        let utc_time_now = utc_now.timestamp();
        let refresh_token_expire_utc = utc_time_now + *crate::Constants::REFRESH_TOKEN_EXPIRES_SEC;
        let access_token_expire_utc = utc_time_now + *crate::Constants::ACCESS_TOKEN_EXPIRES_SEC;
        let refresh_token_expire_utc_format = timestamp_to_rfc7231(refresh_token_expire_utc);
        // WARNING: Access token has same expire timestamp (in Cookie metadata, not actual Cookie
        // payload). This is so other middleware can compare access token and refresh tokens, otherwise
        // browser will delete access tokens and there would be nothing else to compare!
        let access_token_expire_utc_format = refresh_token_expire_utc_format.clone();//timestamp_to_rfc7231(access_token_expire_utc);

        // Create refresh token
        let refresh_token_id = Uuid::new_v4();
        let mut refresh_token_claims = BTreeMap::new();
        refresh_token_claims.insert("userId", token_data.userid.to_string());
        refresh_token_claims.insert("id", refresh_token_id.to_string());
        refresh_token_claims.insert("ip", ipv6.clone());
        refresh_token_claims.insert("rtk-expire", refresh_token_expire_utc.to_string());
        // You can only refresh if the access token is rejected, and thats only when
        // it has expired (or if IP has changed). Subtract by some constant, just
        // for good measure.
        refresh_token_claims.insert("atk-expire", (access_token_expire_utc - 1).to_string());

        let refresh_jwt_token = refresh_token_claims.sign_with_key(jwt_key).map_err(|err| {
            tracing::error!("Failed to sign refresh JWT token, err: {}", err);
            TokenGenerationError::SigningFailureRefreshJWTToken
        })?;

        // Create access token
        let mut access_token_claims = BTreeMap::new();
        access_token_claims.insert("userId", token_data.userid.to_string());
        access_token_claims.insert("ip", ipv6);
        access_token_claims.insert("expire", access_token_expire_utc.to_string());

        if token_data.has_support_privilege {
            access_token_claims.insert("supportprivilege", "1".to_string()); // 1 for true, just uses up less
                                                                             // data am i right?!
                                                                             // also it doesnt mean
                                                                             // anything really,
                                                                             // there just has to
                                                                             // be a value
        }

        let access_jwt_token = access_token_claims.sign_with_key(jwt_key).map_err(|err| {
            tracing::error!("Failed to sign access JWT token, err: {}", err);
            TokenGenerationError::SigningFailureAccessJWTToken
        })?;

        Ok( TokenPackage {
            utc: utc_time_now,
            refresh_id: refresh_token_id,
            refresh_token: refresh_jwt_token,
            access_token: access_jwt_token,
            refresh_expire_format: refresh_token_expire_utc_format,
            access_expire_format: access_token_expire_utc_format,
        })
    }
}

#[allow(non_snake_case)]
pub mod DB {
    use diesel::prelude::*;
    use crate::db_schema::{hooked_sql_types::{SupportTicketState, SupportWhoAreYou}, supporttickets, supportticketmessages};
    use chrono::naive::NaiveDateTime;

    #[derive(Queryable, Debug)]
    #[diesel(table_name = users)]
    #[allow(non_snake_case)]
    pub struct UserQueryResult {
        pub userid: i64,
        pub username: String,
        pub email: String,
        pub emailverified: bool,
        pub bcryptpass: Vec<u8>,
        pub createdat: NaiveDateTime, 
        pub supportprivilege: bool,
    }

    #[derive(Queryable, Debug)]
    #[allow(non_snake_case)]
    pub struct UserCreditsQueryResult(pub Option<i64>, pub Option<chrono::NaiveDateTime>);

    #[derive(Queryable, Selectable, Debug)]
    #[diesel(table_name = supportticketmessages)]
    pub struct SupportTicketMessage {
        pub id: i32,
        pub ticketid: i32,
        pub message: String,
        pub createdat: NaiveDateTime,
        pub isteam: bool,
    }

    #[derive(QueryableByName, Selectable, Queryable, Debug)]
    #[diesel(table_name = supporttickets)]
    pub struct SupportTicket {
        pub id: i32,
        pub name: String,
        pub summary: String,
        pub email: String,
        pub wau: SupportWhoAreYou,
        pub state: SupportTicketState,
        pub claimedby: Option<i64>,
        pub claimedbyname: Option<String>,
        pub createdat: NaiveDateTime, 
        pub lastchanged: NaiveDateTime, 
    }
}

#[allow(non_snake_case)]
pub mod Email {
    use ::std::sync::Arc;
    use crate::State::{self, AppState}; 
    use deadpool_redis::redis::cmd;
    use base64::prelude::*;
    use trust_dns_resolver::TokioAsyncResolver;
    use super::Constants;
    use super::db_schema::problematicemails;
    use super::common_types::SESEmailBlock::EmailBlock;
    use sha2::{Sha256, Digest};
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    pub async fn is_safe_to_send_to(appstate: Arc<State::InternalAppState>, email: &str) -> bool {
        let email_identifier;
        {
            let mut hasher = Sha256::new();
            hasher.update(format!("{}rapidl-nonce!#?", email));
            email_identifier = hex::encode(hasher.finalize());
        }
        let Ok(mut conn) = appstate.postgres.get().await else {
            return false;
        };
        let result = problematicemails::table.filter(problematicemails::hash.eq(&email_identifier))
                                            .select(EmailBlock::as_select())
                                            .first(&mut conn)
                                            .await;
        match result {
            Ok(emailblock) => {
                let utc = chrono::Utc::now().naive_utc();
                if utc >= emailblock.nextreset {
                    let _ = diesel::update(problematicemails::table.filter(problematicemails::hash.eq(&email_identifier)))
                                    .set(problematicemails::count.eq(0))
                                    .execute(&mut conn)
                                    .await;
                    true
                } else {
                    return emailblock.count <= *Constants::SKIP_EMAIL_IF_BLOCK_COUNT_ABOVE;
                }
            },
            Err(err) => match err {
                diesel::result::Error::NotFound => true,
                _ => false,
            },
        }
    }

    pub async fn verify_email(appstate: AppState, email: &str) -> bool {
        if !dispo::is_valid(email) {
            return false;
        }
        let email_parts = email.split('@');
        let Some(domain) = email_parts.last() else { return false };
        let b64_domain = BASE64_STANDARD.encode(&domain);
        let Ok(mut redis_conn) = appstate.redis.get().await else {
            return false;
        };
        let previous_verified = match cmd("GET").arg(&[&b64_domain]).query_async::<_, Option<String>>(&mut redis_conn).await {
            Ok(x) => x,
            Err(_) => return false,
        };
        match previous_verified {
            Some(previous_verified) => {
                if previous_verified == "t" {
                    return true;
                }
                false
            },
            None => {
                let valid = check_domain(&appstate.dns_resolver, domain).await;
                if let Err(_) = cmd("SET")
                    .arg(&[b64_domain.as_ref(), if valid { "t" } else { "f" } , "EX", "259200"])
                    .query_async::<_, ()>(&mut redis_conn)
                    .await
                {
                    return false;
                }
                valid
            },
        }
    }

    async fn check_domain(resolver: &TokioAsyncResolver, domain: &str) -> bool {
        let mut has_mx = false;
        let mut has_spf = false;
        let mut has_dmarc = false;

        if let Ok(mx_response) = resolver.mx_lookup(domain).await {
            if mx_response.iter().peekable().peek().is_some() {
                has_mx = true;
            }
        } else {
            return false;
        }

        if let Ok(txt_response) = resolver.txt_lookup(domain).await {
            for record in txt_response {
                if record.to_string().starts_with("v=spf1") {
                    has_spf = true;
                    break;
                }
            }
        } else {
            return false;
        }

        if let Ok(dmarc_records) = resolver.txt_lookup(String::from("_dmarc.") + domain).await {
            for record in dmarc_records {
                if record.to_string().starts_with("v=DMARC1") {
                    has_dmarc = true;
                    break;
                }
            }
        } else {
            return false;
        }

        return has_mx && has_spf && has_dmarc;
    }
}

#[allow(non_snake_case)]
pub mod MinimalState {
    use ::std::sync::Arc;
    use diesel_async::pooled_connection::deadpool::Pool as PostgresPool;
    use deadpool_redis::Pool as RedisPool;
    use diesel_async::{
        pooled_connection::{
            ManagerConfig,
            AsyncDieselConnectionManager,
            deadpool::Pool,
        },
        AsyncPgConnection,
    };
    use deadpool_redis::{
        Config as RedisConfig, 
        Runtime as RedisRuntime,
        ConnectionInfo as RedisPoolConnectionInfo,
        ConnectionAddr as RedisConnectionAddr,
        RedisConnectionInfo,
    };
    use crate::Constants::*;

    pub struct InternalAppState {
        pub postgres: PostgresPool<AsyncPgConnection>,
        pub redis: RedisPool,
    }
    pub type AppState = Arc<InternalAppState>;

    pub async fn make_state() -> Result<AppState, crate::E> {
        // Create our connection pool
        tracing::info!("Setting up Postgres connection pool");
        let mut config = ManagerConfig::default();
        config.custom_setup = Box::new(super::State::establish_connection);
        let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(&*DATABASE_URL, config);
        let pool = Pool::builder(config).build()?;

        tracing::info!("Setting up secure Redis connection pool");
        let redisconnectioninfo = RedisPoolConnectionInfo {
            addr: RedisConnectionAddr::TcpTls{
                host: REDIS_SESSION_DATABASE_HOST.clone(),
                port: *REDIS_SESSION_DATABASE_PORT,
                insecure: false, 
            },
            redis: RedisConnectionInfo {
                db: 0,
                username: Some(REDIS_SESSION_DATABASE_USER.clone()),
                password: Some(REDIS_SESSION_DATABASE_PASS.clone()),
            }
        };
        let redisconfig = RedisConfig::from_connection_info(redisconnectioninfo);
        let redispool = redisconfig.create_pool(Some(RedisRuntime::Tokio1)).unwrap();

        // Create AppState
        tracing::info!("Creating AppState");
        Ok(Arc::new(InternalAppState {
            postgres: pool,
            redis: redispool,
        }))
    }
}

#[allow(non_snake_case)]
pub mod State {
    use ::std::sync::Arc;
    use aws_config::BehaviorVersion;
    use diesel::{ConnectionError, ConnectionResult};
    use diesel_async::pooled_connection::deadpool::Pool as PostgresPool;
    use deadpool_redis::Pool as RedisPool;
    use reqwest::Client;
    use trust_dns_resolver::TokioAsyncResolver;
    use trust_dns_resolver::config::*;
    use diesel_async::{
        pooled_connection::{
            ManagerConfig,
            AsyncDieselConnectionManager,
            deadpool::Pool,
        },
        AsyncPgConnection,
    };
    use deadpool_redis::{
        Config as RedisConfig, 
        Runtime as RedisRuntime,
        ConnectionInfo as RedisPoolConnectionInfo,
        ConnectionAddr as RedisConnectionAddr,
        RedisConnectionInfo,
    };
    use futures_util::{future::BoxFuture, FutureExt};
    use crate::Constants::*;

    pub struct InternalAppState {
        pub postgres: PostgresPool<AsyncPgConnection>,
        pub redis: RedisPool,
        pub http_client: Client,
        pub lambda_client: aws_sdk_lambda::Client,
        pub sqs_client: aws_sdk_sqs::Client,
        pub s3_client: aws_sdk_s3::Client,
        pub dns_resolver: TokioAsyncResolver,
    }
    pub type AppState = Arc<InternalAppState>;

    pub async fn make_state() -> Result<AppState, crate::E> {
        // Create our connection pool
        tracing::info!("Setting up Postgres connection pool");
        let mut config = ManagerConfig::default();
        config.custom_setup = Box::new(establish_connection);
        let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(&*DATABASE_URL, config);
        let pool = Pool::builder(config).build()?;

        tracing::info!("Setting up secure Redis connection pool");
        let redisconnectioninfo = RedisPoolConnectionInfo {
            addr: RedisConnectionAddr::TcpTls{
                host: REDIS_SESSION_DATABASE_HOST.clone(),
                port: *REDIS_SESSION_DATABASE_PORT,
                insecure: false, 
            },
            redis: RedisConnectionInfo {
                db: 0,
                username: Some(REDIS_SESSION_DATABASE_USER.clone()),
                password: Some(REDIS_SESSION_DATABASE_PASS.clone()),
            }
        };
        let redisconfig = RedisConfig::from_connection_info(redisconnectioninfo);
        let redispool = redisconfig.create_pool(Some(RedisRuntime::Tokio1)).unwrap();

        /* Create AWS clients */
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let lambda_client = aws_sdk_lambda::Client::new(&config);
        let sqs_client = aws_sdk_sqs::Client::new(&config);
        let s3_client = aws_sdk_s3::Client::new(&config);

        /* Create DNS reoslver */
        let resolver = TokioAsyncResolver::tokio(ResolverConfig::cloudflare_tls(), ResolverOpts::default());

        // Create AppState
        tracing::info!("Creating AppState");
        Ok(Arc::new(InternalAppState {
            postgres: pool,
            redis: redispool,
            http_client: reqwest::Client::new(),
            lambda_client,
            sqs_client,
            s3_client,
            dns_resolver: resolver,
        }))
    }
    pub fn establish_connection(config: &str) -> BoxFuture<ConnectionResult<AsyncPgConnection>> {
        let fut = async {
            // We first set up the way we want rustls to work.
            let rustls_config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_certs())
                .with_no_client_auth();
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(rustls_config);
            let (client, conn) = tokio_postgres::connect(config, tls)
                .await
                .map_err(|e| ConnectionError::BadConnection(e.to_string()))?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    eprintln!("Database connection: {e}");
                }
            });
            AsyncPgConnection::try_from(client).await
        };
        fut.boxed()
    }

    pub fn root_certs() -> rustls::RootCertStore {
        let mut roots = rustls::RootCertStore::empty();
        let certs = rustls_native_certs::load_native_certs().expect("Certs not loadable!");
        let certs: Vec<_> = certs.into_iter().map(|cert| cert.0).collect();
        roots.add_parsable_certificates(&certs);
        roots
    }
}

#[allow(non_snake_case)]
pub mod Constants {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use lazy_static::lazy_static;
    
    // The number of passes for the bcrypt algorithm
    pub const HASH_COST: u32 = 9;

    // WARNING: These are global variables that get 
    // initialised at the entry point, and should not
    // be written to after
    lazy_static!{
        pub static ref GENERATE_QUEUE_URL: String = {
            dotenvy::var("GENERATE_QUEUE_URL").expect("No environment variable for GENERATE_QUEUE_URL").to_owned()
        };
        pub static ref DEVELOPMENT_MODE: bool = {
            dotenvy::var("DEVELOPMENT_MODE").unwrap_or("false".to_owned()).parse().expect("Failed to parse DEVELOPMENT_MODE")
        };
        pub static ref LAMBDA_EMAIL_ARN: String = {
            dotenvy::var("LAMBDA_EMAIL_ARN").expect("No environment variable for LAMBDA_EMAIL_ARN").to_owned()
        };
        pub static ref ORIGIN_URL: String = {
            dotenvy::var("ORIGIN_URL").expect("No environment variable for ORIGIN_URL").to_owned()
        };
        pub static ref DATABASE_URL: String = {
            dotenvy::var("DATABASE_URL").expect("No environment variable for DATABASE_URL").to_owned()
        };
        pub static ref REDIS_SESSION_DATABASE_HOST: String = {
           dotenvy::var("REDIS_SESSION_DATABASE_HOST").expect("No environment variable for REDIS_SESSION_DATABASE_HOST").to_owned()
        };
        pub static ref REDIS_SESSION_DATABASE_PORT: u16 = {
            dotenvy::var("REDIS_SESSION_DATABASE_PORT").expect("No environment variable for REDIS_SESSION_DATABASE_PORT").to_owned().parse().expect("Failed to parse REDIS_SESSION_DATABASE_PORT")
        };
        pub static ref REDIS_SESSION_DATABASE_USER: String = {
            dotenvy::var("REDIS_SESSION_DATABASE_USER").expect("No environment variable for REDIS_SESSION_DATABASE_USER").to_owned()
        };
        pub static ref REDIS_SESSION_DATABASE_PASS: String = {
            dotenvy::var("REDIS_SESSION_DATABASE_PASS").expect("No environment variable for REDIS_SESSION_DATABASE_PASS").to_owned()
        };
        pub static ref GOOGLE_TICK_RECAPTCHA_SECRET_KEY: String = {
            dotenvy::var("GOOGLE_TICK_RECAPTCHA_SECRET_KEY").expect("No environment variable for GOOGLE_TICK_RECAPTCHA_SECRET_KEY").to_owned()
        };
        pub static ref GOOGLE_INVISIBLE_RECAPTCHA_SECRET_KEY: String = {
            dotenvy::var("GOOGLE_INVISIBLE_RECAPTCHA_SECRET_KEY").expect("No environment variable for GOOGLE_INVISIBLE_RECAPTCHA_SECRET_KEY").to_owned()
        };
        pub static ref JWT_KEY: Hmac<Sha256> = {
            let raw_key = dotenvy::var("JWT_KEY").expect("No environment variable for JWT_KEY").to_owned();
            Hmac::new_from_slice(raw_key.as_bytes()).expect("Failed to generate HMAC for JWT_KEY")
        };
        pub static ref SUBSCRPTION_NEWSLETTER_COOLDOWN: i64 = {
            let maybe = dotenvy::var("SUBSCRPTION_NEWSLETTER_COOLDOWN");
            let mut time = 60 * 60;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom SUBSCRPTION_NEWSLETTER_COOLDOWN: {time}");
                    } else {
                        tracing::info!("Failed to parse SUBSCRPTION_NEWSLETTER_COOLDOWN, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref SEND_VERIFICATION_COOLDOWN: i64 = {
            let maybe = dotenvy::var("SEND_VERIFICATION_COOLDOWN");
            let mut time = 60 * 2;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom SEND_VERIFICATION_COOLDOWN: {time}");
                    } else {
                        tracing::info!("Failed to parse SEND_VERIFICATION_COOLDOWN, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };

        pub static ref SEND_CONTACT_US_COOLDOWN: i64 = {
            let maybe = dotenvy::var("SEND_CONTACT_US_COOLDOWN");
            let mut time = 60 * 10;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom SEND_CONTACT_US_COOLDOWN: {time}");
                    } else {
                        tracing::info!("Failed to parse SEND_CONTACT_US_COOLDOWN, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };

        pub static ref REFRESH_TOKEN_EXPIRES_SEC: i64 = {
            let maybe = dotenvy::var("REFRESH_TOKEN_EXPIRES_SEC");
            let mut time = 60 * 60 * 24 * 3;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom REFRESH_TOKEN_EXPIRES_SEC: {time}");
                    } else {
                        tracing::info!("Failed to parse REFRESH_TOKEN_EXPIRES_SEC, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref ACCESS_TOKEN_EXPIRES_SEC: i64 = {
            let maybe = dotenvy::var("ACCESS_TOKEN_EXPIRES_SEC");
            let mut time = 60 * 5;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom ACCESS_TOKEN_EXPIRES_SEC: {time}");
                    } else {
                        tracing::info!("Failed to parse ACCESS_TOKEN_EXPIRES_SEC, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref STANDARD_CREDITS_EXPIRE_AFTER_SECS: i64 = {
            let maybe = dotenvy::var("STANDARD_CREDITS_EXPIRE_AFTER_SECS");
            let mut time = 60 * 60 * 24 * 7 * 3;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom STANDARD_CREDITS_EXPIRE_AFTER_SECS: {time}");
                    } else {
                        tracing::info!("Failed to parse STANDARD_CREDITS_EXPIRE_AFTER_SECS, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref FREE_CREDITS_ON_VERIFY_EXPIRE_AFTER_SECS: i64 = {
            let maybe = dotenvy::var("FREE_CREDITS_ON_VERIFY_EXPIRE_AFTER_SECS");
            let mut time = 60 * 60 * 24 * 3;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom FREE_CREDITS_ON_VERIFY_EXPIRE_AFTER_SECS: {time}");
                    } else {
                        tracing::info!("Failed to parse FREE_CREDITS_ON_VERIFY_EXPIRE_AFTER_SECS, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref FREE_CREDITS_ON_VERIFY: i32 = {
            let maybe = dotenvy::var("FREE_CREDITS_ON_VERIFY");
            let mut time = 4;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom FREE_CREDITS_ON_VERIFY: {time}");
                    } else {
                        tracing::info!("Failed to parse FREE_CREDITS_ON_VERIFY, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref GENERATED_BUCKET_NAME: String = {
            dotenvy::var("GENERATED_BUCKET_NAME").expect("No environment variable for GENERATED_BUCKET_NAME").to_owned()
        };
        pub static ref COMPLAINT_BOUNCE_NEXT_RESET: i64 = {
            let maybe = dotenvy::var("COMPLAINT_BOUNCE_NEXT_RESET");
            let mut time = 604800;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom COMPLAINT_BOUNCE_NEXT_RESET: {time}");
                    } else {
                        tracing::info!("Failed to parse COMPLAINT_BOUNCE_NEXT_RESET, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref SKIP_EMAIL_IF_BLOCK_COUNT_ABOVE: i32 = {
            let maybe = dotenvy::var("SKIP_EMAIL_IF_BLOCK_COUNT_ABOVE");
            let mut time = 1;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom SKIP_EMAIL_IF_BLOCK_COUNT_ABOVE: {time}");
                    } else {
                        tracing::info!("Failed to parse SKIP_EMAIL_IF_BLOCK_COUNT_ABOVE, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };
        pub static ref ALLOWED_TICKETS_OPEN_AT_ONCE: i64 = {
            let maybe = dotenvy::var("ALLOWED_TICKETS_OPEN_AT_ONCE");
            let mut time = 2;
            match maybe {
                Ok(secs) => {
                    if let Ok(new_secs) = secs.parse() {
                        time = new_secs;
                        tracing::info!("Using custom ALOWED_TICKETS_OPEN_AT_ONCE: {time}");
                    } else {
                        tracing::info!("Failed to parse ALLOWED_TICKETS_OPEN_AT_ONCE, using default, {time}");
                    }
                }
                _ => ()
            }
            time
        };

    }
}
