use axum::{
    routing,
    Router,
    middleware as axum_middleware,
};
use tower::ServiceBuilder;

#[tokio::main]
async fn main() -> Result<(), common_types_accounts::E> {
    ::std::env::set_var("AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH", "true");

    tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_target(false)
            .without_time()
            .init();

    let appstate = common_types_accounts::State::make_state().await?;
    let router = Router::new()
                    .route("/get-profile", routing::get(common_types_accounts::Routes::get_profile::request))
                    .route("/get-profile/:desired_user_id", routing::get(common_types_accounts::Routes::get_profile::request))
                    .route_layer(ServiceBuilder::new()
                                 .layer(axum_middleware::from_fn_with_state(appstate.clone(), common_types_accounts::Middleware::validate_access_auth::middleware))
                              )
                    .route_layer(axum_middleware::from_fn(common_types_accounts::Middleware::set_cors_headers::middleware))
                    .with_state(appstate);

    lambda_web::run_hyper_on_lambda(router).await
}
