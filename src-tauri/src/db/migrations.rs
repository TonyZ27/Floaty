use sqlx::SqlitePool;

static MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../../migrations/0001_init.sql")),
    ("0002_reminders", include_str!("../../migrations/0002_reminders.sql")),
    ("0003_settings", include_str!("../../migrations/0003_settings.sql")),
    ("0004_soft_delete", include_str!("../../migrations/0004_soft_delete.sql")),
    ("0005_fts", include_str!("../../migrations/0005_fts.sql")),
    ("0006_collapse", include_str!("../../migrations/0006_collapse.sql")),
    ("0007_collapse_w", include_str!("../../migrations/0007_collapse_w.sql")),
    (
        "0008_window_bindings",
        include_str!("../../migrations/0008_window_bindings.sql"),
    ),
];

pub async fn run(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (
            name TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    for (name, sql) in MIGRATIONS {
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT name FROM _migrations WHERE name = ?")
                .bind(name)
                .fetch_optional(pool)
                .await?;
        if exists.is_some() {
            continue;
        }
        let mut tx = pool.begin().await?;
        sqlx::query(sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO _migrations (name, applied_at) VALUES (?, ?)")
            .bind(name)
            .bind(chrono::Utc::now().timestamp_millis())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}
