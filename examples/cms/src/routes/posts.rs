use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::{AppError, Result};

#[derive(Serialize, sqlx::FromRow)]
pub struct Post {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub slug: String,
    pub body: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/posts", get(list))
        .route("/posts/slug/{slug}", get(get_by_slug))
        .route("/posts/{id}", get(get_by_id))
}

async fn list(State(pool): State<PgPool>, Query(q): Query<ListQuery>) -> Result<Json<Vec<Post>>> {
    let posts = match q.status {
        Some(status) => {
            sqlx::query_as::<_, Post>(
                "SELECT * FROM posts WHERE status = $1 ORDER BY created_at DESC",
            )
            .bind(status)
            .fetch_all(&pool)
            .await
        }
        None => {
            sqlx::query_as::<_, Post>("SELECT * FROM posts ORDER BY created_at DESC")
                .fetch_all(&pool)
                .await
        }
    }?;
    Ok(Json(posts))
}

async fn get_by_id(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Post>> {
    let post = sqlx::query_as::<_, Post>("SELECT * FROM posts WHERE id = $1")
        .bind(id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| AppError::not_found("post not found"))?;
    Ok(Json(post))
}

async fn get_by_slug(State(pool): State<PgPool>, Path(slug): Path<String>) -> Result<Json<Post>> {
    let post = sqlx::query_as::<_, Post>("SELECT * FROM posts WHERE slug = $1")
        .bind(slug)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| AppError::not_found("post not found"))?;
    Ok(Json(post))
}
