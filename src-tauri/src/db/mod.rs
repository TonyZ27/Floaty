use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::PathBuf;

pub mod migrations;
pub mod stickies;
pub mod items;
pub mod reminders;
pub mod snapshots;
pub mod window_bindings;

pub type Db = SqlitePool;

pub async fn init(data_dir: PathBuf) -> anyhow::Result<Db> {
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("floaty.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await?;
    migrations::run(&pool).await?;
    Ok(pool)
}
