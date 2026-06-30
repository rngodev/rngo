pub mod authors;
pub mod posts;

use axum::Router;
use sqlx::PgPool;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .merge(authors::router())
        .merge(posts::router())
        .with_state(pool)
}
