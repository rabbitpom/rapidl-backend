use axum::{
    extract::{
        Extension,
        State,
        Query,
    },
    http::StatusCode,
    Json
};
use garde::Validate;
use serde::Serialize;
use diesel::sql_types::{BigInt, Integer};
use diesel::prelude::*;
use diesel::sql_query;
use diesel_async::RunQueryDsl;

use crate::{
    Schema::generation,
    Response::{ServerResponse, internal_server_error, status_response},
    State::AppState, 
    Middleware::validate_access_auth::AccessTokenDescription,
};

#[derive(Serialize)]
pub struct GroupPayload {
    content: Vec<GenerationQueryable>,
    total_pages: Option<usize>,
}

mod db;
use db::{Pagination, GenerationQueryable};

// GET API endpoint
#[tracing::instrument(skip(access_token, appstate, pagination), fields(UserId=%access_token.user_id,request="/generated/list",page=%pagination.page,page_size=%pagination.page_size))]
pub async fn request(Extension(access_token): Extension<AccessTokenDescription>, State(appstate): State<AppState>, Query(pagination): Query<Pagination>) -> Result<Json<GroupPayload>, ServerResponse> {
    let validation_result = pagination.validate(&());
    if let Err(err) = validation_result {
        tracing::info!("Validation failed with reason: {err}");
        return Err(status_response(StatusCode::BAD_REQUEST, err));
    }

    let generations: Vec<GenerationQueryable>;
    let mut total_generations = None;
    {
        let mut conn = appstate.postgres.get().await.map_err(|err| {
            tracing::error!("Failed to fetch Postgres connection, {err}");
            internal_server_error("Internal Service Error")
        })?;
        generations = sql_query("SELECT status, createdat, finishedon, jobid, creditsused, category, options, displayname FROM (SELECT status, createdat, finishedon, jobid, creditsused, category, options, displayname, ROW_NUMBER() OVER (ORDER BY id) AS row_num FROM generation WHERE userid = $1) AS subquery WHERE row_num BETWEEN (($2 - 1) * $3 + 1) AND ($2 * $3)")
                .bind::<BigInt, _>(access_token.user_id)
                .bind::<Integer, _>(pagination.page as i32)
                .bind::<Integer, _>(pagination.page_size as i32)
                .load(&mut conn)
                .await.map_err(|err| {
                    tracing::error!("Failed to query page {}, with page size, {}, due to {err}", pagination.page, pagination.page_size);
                    internal_server_error("Internal Service Error")
                })?;

        if pagination.get_total_pages {
            total_generations = Some(
                generation::table.filter(generation::userid.eq(&access_token.user_id))
                            .count()
                            .get_result::<i64>(&mut conn)
                            .await.map_err(|err| {
                                tracing::error!("Failed to query total page size due to {err}");
                                internal_server_error("Internal Service Error")
                            })? as usize
            );
        }
    }
    Ok(Json(GroupPayload {
        total_pages: match total_generations {
            None => None,
            Some(total_generations) => {
                Some((total_generations as f64 / pagination.page_size as f64).ceil() as usize)
            }
        },
        content: generations,
    }))
}

