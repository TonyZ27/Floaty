use crate::db::Db;
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub use crate::assignment::BindingState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct WindowBinding {
    pub sticky_id: String,
    pub app_bundle_id: String,
    pub app_name: String,
    pub window_title: String,
    pub runtime_pid: Option<i64>,
    pub runtime_window_id: Option<i64>,
    pub state: BindingState,
    pub prior_pinned: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWindowBinding {
    pub sticky_id: String,
    pub app_bundle_id: String,
    pub app_name: String,
    pub window_title: String,
    pub runtime_pid: i64,
    pub runtime_window_id: i64,
}

pub async fn bind(db: &Db, input: NewWindowBinding) -> AppResult<WindowBinding> {
    let mut tx = db.begin().await?;
    let sticky: Option<(i64,)> =
        sqlx::query_as("SELECT pinned FROM stickies WHERE id = ? AND deleted_at IS NULL")
            .bind(&input.sticky_id)
            .fetch_optional(&mut *tx)
            .await?;
    let current_pinned = sticky.ok_or(AppError::NotFound)?.0;
    let prior_pinned: Option<(i64,)> =
        sqlx::query_as("SELECT prior_pinned FROM window_bindings WHERE sticky_id = ?")
            .bind(&input.sticky_id)
            .fetch_optional(&mut *tx)
            .await?;
    let prior_pinned = prior_pinned.map(|row| row.0).unwrap_or(current_pinned);
    let now = chrono::Utc::now().timestamp_millis();

    sqlx::query("UPDATE stickies SET pinned = 0, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&input.sticky_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO window_bindings (
            sticky_id, app_bundle_id, app_name, window_title, runtime_pid,
            runtime_window_id, state, prior_pinned, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(sticky_id) DO UPDATE SET
            app_bundle_id = excluded.app_bundle_id,
            app_name = excluded.app_name,
            window_title = excluded.window_title,
            runtime_pid = excluded.runtime_pid,
            runtime_window_id = excluded.runtime_window_id,
            state = excluded.state,
            updated_at = excluded.updated_at",
    )
    .bind(&input.sticky_id)
    .bind(&input.app_bundle_id)
    .bind(&input.app_name)
    .bind(&input.window_title)
    .bind(input.runtime_pid)
    .bind(input.runtime_window_id)
    .bind(BindingState::Active)
    .bind(prior_pinned)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let saved =
        sqlx::query_as::<_, WindowBinding>("SELECT * FROM window_bindings WHERE sticky_id = ?")
            .bind(&input.sticky_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(saved)
}

pub async fn get(db: &Db, sticky_id: &str) -> AppResult<Option<WindowBinding>> {
    let binding =
        sqlx::query_as::<_, WindowBinding>("SELECT * FROM window_bindings WHERE sticky_id = ?")
            .bind(sticky_id)
            .fetch_optional(db)
            .await?;
    Ok(binding)
}

pub async fn list(db: &Db) -> AppResult<Vec<WindowBinding>> {
    let bindings =
        sqlx::query_as::<_, WindowBinding>("SELECT * FROM window_bindings ORDER BY created_at ASC")
            .fetch_all(db)
            .await?;
    Ok(bindings)
}

pub async fn set_state(db: &Db, sticky_id: &str, state: BindingState) -> AppResult<WindowBinding> {
    let now = chrono::Utc::now().timestamp_millis();
    let updated =
        sqlx::query("UPDATE window_bindings SET state = ?, updated_at = ? WHERE sticky_id = ?")
            .bind(state)
            .bind(now)
            .bind(sticky_id)
            .execute(db)
            .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    get(db, sticky_id).await?.ok_or(AppError::NotFound)
}

pub async fn set_state_and_clear_runtime(
    db: &Db,
    sticky_id: &str,
    state: BindingState,
) -> AppResult<WindowBinding> {
    let now = chrono::Utc::now().timestamp_millis();
    let updated = sqlx::query(
        "UPDATE window_bindings
         SET state = ?, runtime_pid = NULL, runtime_window_id = NULL, updated_at = ?
         WHERE sticky_id = ?",
    )
    .bind(state)
    .bind(now)
    .bind(sticky_id)
    .execute(db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    get(db, sticky_id).await?.ok_or(AppError::NotFound)
}

pub async fn mark_all_reconnect_required(db: &Db) -> AppResult<u64> {
    let now = chrono::Utc::now().timestamp_millis();
    let updated = sqlx::query(
        "UPDATE window_bindings
         SET state = ?, runtime_pid = NULL, runtime_window_id = NULL, updated_at = ?",
    )
    .bind(BindingState::ReconnectRequired)
    .bind(now)
    .execute(db)
    .await?;
    Ok(updated.rows_affected())
}

pub async fn unbind(db: &Db, sticky_id: &str) -> AppResult<Option<WindowBinding>> {
    let mut tx = db.begin().await?;
    let binding =
        sqlx::query_as::<_, WindowBinding>("SELECT * FROM window_bindings WHERE sticky_id = ?")
            .bind(sticky_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(binding) = binding else {
        tx.commit().await?;
        return Ok(None);
    };

    sqlx::query("UPDATE stickies SET pinned = ?, updated_at = ? WHERE id = ?")
        .bind(binding.prior_pinned)
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(sticky_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM window_bindings WHERE sticky_id = ?")
        .bind(sticky_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(binding))
}

pub async fn remove_for_deleted_sticky(db: &Db, sticky_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM window_bindings WHERE sticky_id = ?")
        .bind(sticky_id)
        .execute(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> crate::db::Db {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrations::run(&pool).await.unwrap();
        pool
    }

    impl NewWindowBinding {
        fn test(sticky_id: &str, runtime_window_id: i64) -> Self {
            Self {
                sticky_id: sticky_id.to_owned(),
                app_bundle_id: "com.example.Target".to_owned(),
                app_name: "Target".to_owned(),
                window_title: "Document".to_owned(),
                runtime_pid: 42,
                runtime_window_id,
            }
        }
    }

    #[tokio::test]
    async fn binding_schema_requires_one_existing_unique_sticky() {
        let db = test_pool().await;
        let insert = "INSERT INTO window_bindings (
                sticky_id, app_bundle_id, app_name, window_title, runtime_pid,
                runtime_window_id, state, prior_pinned, created_at, updated_at
            ) VALUES (?, 'com.example.Target', 'Target', '', 42, 101, 'active', 0, 0, 0)";

        let null_owner = sqlx::query(insert)
            .bind(Option::<String>::None)
            .execute(&db)
            .await;
        assert!(null_owner.is_err());

        let orphaned = sqlx::query(insert).bind("missing").execute(&db).await;
        assert!(orphaned.is_err());

        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        sqlx::query(insert)
            .bind(&sticky.id)
            .execute(&db)
            .await
            .unwrap();
        let duplicate = sqlx::query(insert).bind(&sticky.id).execute(&db).await;
        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn replacing_binding_keeps_one_row_and_original_pin() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        sqlx::query("UPDATE stickies SET pinned = 1 WHERE id = ?")
            .bind(&sticky.id)
            .execute(&db)
            .await
            .unwrap();

        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 202))
            .await
            .unwrap();

        let saved = get(&db, &sticky.id).await.unwrap().unwrap();
        assert_eq!(saved.runtime_window_id, Some(202));
        assert_eq!(saved.prior_pinned, 1);
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM window_bindings WHERE sticky_id = ?")
                .bind(&sticky.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(count.0, 1);
        assert_eq!(
            crate::db::stickies::get(&db, &sticky.id)
                .await
                .unwrap()
                .pinned,
            0
        );
    }

    #[tokio::test]
    async fn failed_replacement_keeps_the_original_target() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_binding_update
             BEFORE UPDATE ON window_bindings
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&db)
        .await
        .unwrap();

        assert!(bind(&db, NewWindowBinding::test(&sticky.id, 202))
            .await
            .is_err());
        assert_eq!(
            get(&db, &sticky.id)
                .await
                .unwrap()
                .unwrap()
                .runtime_window_id,
            Some(101)
        );
    }

    #[tokio::test]
    async fn failed_initial_bind_does_not_disable_pinning() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        sqlx::query("UPDATE stickies SET pinned = 1 WHERE id = ?")
            .bind(&sticky.id)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_binding_insert
             BEFORE INSERT ON window_bindings
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&db)
        .await
        .unwrap();

        assert!(bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .is_err());
        assert!(get(&db, &sticky.id).await.unwrap().is_none());
        assert_eq!(
            crate::db::stickies::get(&db, &sticky.id)
                .await
                .unwrap()
                .pinned,
            1
        );
    }

    #[tokio::test]
    async fn startup_never_reuses_runtime_identity() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();

        mark_all_reconnect_required(&db).await.unwrap();
        let saved = get(&db, &sticky.id).await.unwrap().unwrap();
        assert_eq!(saved.state, BindingState::ReconnectRequired);
        assert_eq!(saved.runtime_pid, None);
        assert_eq!(saved.runtime_window_id, None);
    }

    #[tokio::test]
    async fn target_close_keeps_binding_and_clears_runtime_identity_atomically() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();

        let saved = set_state_and_clear_runtime(&db, &sticky.id, BindingState::TargetClosed)
            .await
            .unwrap();

        assert_eq!(saved.state, BindingState::TargetClosed);
        assert_eq!(saved.runtime_pid, None);
        assert_eq!(saved.runtime_window_id, None);
        assert!(get(&db, &sticky.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn unbind_restores_original_pin_and_removes_binding() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        sqlx::query("UPDATE stickies SET pinned = 1 WHERE id = ?")
            .bind(&sticky.id)
            .execute(&db)
            .await
            .unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();

        let removed = unbind(&db, &sticky.id).await.unwrap().unwrap();

        assert_eq!(removed.prior_pinned, 1);
        assert!(get(&db, &sticky.id).await.unwrap().is_none());
        assert_eq!(
            crate::db::stickies::get(&db, &sticky.id)
                .await
                .unwrap()
                .pinned,
            1
        );
    }

    #[tokio::test]
    async fn unbind_rolls_back_pin_restore_when_removal_fails() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        sqlx::query("UPDATE stickies SET pinned = 1 WHERE id = ?")
            .bind(&sticky.id)
            .execute(&db)
            .await
            .unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_binding_delete
             BEFORE DELETE ON window_bindings
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&db)
        .await
        .unwrap();

        assert!(unbind(&db, &sticky.id).await.is_err());
        assert!(get(&db, &sticky.id).await.unwrap().is_some());
        assert_eq!(
            crate::db::stickies::get(&db, &sticky.id)
                .await
                .unwrap()
                .pinned,
            0
        );
    }

    #[tokio::test]
    async fn binding_requires_an_active_sticky() {
        let db = test_pool().await;

        let result = bind(&db, NewWindowBinding::test("missing", 101)).await;

        assert!(matches!(result, Err(crate::error::AppError::NotFound)));
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM window_bindings")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn soft_delete_removes_binding_without_restoring_pin() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        sqlx::query("UPDATE stickies SET pinned = 1 WHERE id = ?")
            .bind(&sticky.id)
            .execute(&db)
            .await
            .unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();

        crate::db::stickies::delete(&db, &sticky.id).await.unwrap();
        crate::db::stickies::restore(&db, &sticky.id).await.unwrap();

        assert!(get(&db, &sticky.id).await.unwrap().is_none());
        assert_eq!(
            crate::db::stickies::get(&db, &sticky.id)
                .await
                .unwrap()
                .pinned,
            0
        );
    }

    #[tokio::test]
    async fn soft_delete_rolls_back_when_binding_cleanup_fails() {
        let db = test_pool().await;
        let sticky = crate::db::stickies::create_default(&db).await.unwrap();
        bind(&db, NewWindowBinding::test(&sticky.id, 101))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_binding_delete
             BEFORE DELETE ON window_bindings
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&db)
        .await
        .unwrap();

        assert!(crate::db::stickies::delete(&db, &sticky.id).await.is_err());
        assert!(get(&db, &sticky.id).await.unwrap().is_some());
        assert!(crate::db::stickies::get(&db, &sticky.id)
            .await
            .unwrap()
            .deleted_at
            .is_none());
    }
}
