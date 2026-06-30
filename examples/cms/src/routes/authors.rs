use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use sqlx::PgPool;

use crate::error::{AppError, Result};

#[derive(Serialize, sqlx::FromRow)]
pub struct Author {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub bio: Option<String>,
    pub created_at: i64,
}

pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/authors", get(list))
        .route("/authors/{id}", get(get_by_id))
}

async fn list(State(pool): State<PgPool>) -> Result<Json<Vec<Author>>> {
    let authors = sqlx::query_as::<_, Author>("SELECT * FROM authors ORDER BY created_at DESC")
        .fetch_all(&pool)
        .await?;
    Ok(Json(authors))
}

async fn get_by_id(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Author>> {
    let author = sqlx::query_as::<_, Author>("SELECT * FROM authors WHERE id = $1")
        .bind(id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| AppError::not_found("author not found"))?;
    Ok(Json(author))
}
