//! Reminder scheduler: tokio 后台任务，sleep_until 到最早的 pending reminder，
//! 到期弹 macOS 系统通知，mark_fired，继续等下一个。
//! 新增/修改/删除 reminders 后通过 Scheduler::wake() 打断 sleep 重新排程。

use crate::db::{self, reminders::Reminder, Db};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};

#[derive(Clone)]
pub struct Scheduler {
    wake: Arc<Notify>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            wake: Arc::new(Notify::new()),
        }
    }

    /// 通知调度器重新读取 DB（例如 reminders 增删后）
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// 启动后台循环。setup 里 spawn 一次。
    pub fn spawn_loop(self, app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                let db = match app.try_state::<Db>() {
                    Some(s) => s,
                    None => {
                        // DB 还没 manage 好，稍等
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                };

                let pending = match db::reminders::list_pending(&db).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[floaty] scheduler list_pending failed: {}", e);
                        self.wake.notified().await;
                        continue;
                    }
                };

                // 找最早的一条
                let next: Option<Reminder> = pending.into_iter().min_by_key(|r| r.fire_at);

                match next {
                    None => {
                        // 没 pending —— 睡到被 wake() 唤醒
                        self.wake.notified().await;
                    }
                    Some(r) => {
                        let now = chrono::Utc::now().timestamp_millis();
                        let due_ms = r.fire_at;
                        if due_ms <= now {
                            // 立即触发（包含启动时的漏触发兜底）
                            fire(&app, &r);
                            let _ = db::reminders::mark_fired(&db, &r.id).await;
                            continue;
                        }
                        let wait_ms = (due_ms - now).min(3600 * 1000) as u64; // 最多睡 1h，之后回头复查
                        let deadline = Instant::now() + Duration::from_millis(wait_ms);
                        tokio::select! {
                            _ = sleep_until(deadline) => {
                                // 再确认一次（可能中途被 snooze 或删除）
                                if let Ok(fresh) = db::reminders::get(&db, &r.id).await {
                                    if fresh.fired_at.is_none() && fresh.fire_at <= chrono::Utc::now().timestamp_millis() {
                                        fire(&app, &fresh);
                                        let _ = db::reminders::mark_fired(&db, &fresh.id).await;
                                    }
                                }
                            }
                            _ = self.wake.notified() => {
                                // 重新排程
                            }
                        }
                    }
                }
            }
        });
    }
}

fn fire(app: &AppHandle, reminder: &Reminder) {
    let title = "Floaty 提醒".to_string();
    let body = if reminder.text_preview.is_empty() {
        "到期任务".to_string()
    } else {
        reminder.text_preview.clone()
    };
    if let Err(e) = app.notification().builder().title(&title).body(&body).show() {
        eprintln!("[floaty] notification show failed: {}", e);
    }
}
