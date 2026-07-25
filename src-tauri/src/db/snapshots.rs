use crate::db::Db;
use std::fs;
use std::path::{Path, PathBuf};

const KEEP_LAST: usize = 10;
const PREFIX: &str = "floaty-";
const SUFFIX: &str = ".db";

/// 启动时自动快照：用 SQLite VACUUM INTO 把当前 DB 复制到
/// `<data_dir>/backups/floaty-YYYYMMDD-HHMMSS.db`，保留最近 KEEP_LAST 份。
///
/// 跟手动备份共用 VACUUM INTO，运行期安全。
/// 任何步骤失败都不该影响启动：调用方把错误打 log 就行。
pub async fn take(db: &Db, data_dir: &Path) -> anyhow::Result<PathBuf> {
    let backup_dir = data_dir.join("backups");
    fs::create_dir_all(&backup_dir)?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let backup_path = backup_dir.join(format!("{PREFIX}{ts}{SUFFIX}"));

    let escaped = backup_path.to_string_lossy().replace('\'', "''");
    let sql = format!("VACUUM INTO '{}'", escaped);
    sqlx::query(&sql).execute(db).await?;

    prune(&backup_dir, KEEP_LAST)?;
    Ok(backup_path)
}

fn prune(dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.starts_with(PREFIX) && s.ends_with(SUFFIX)
        })
        .collect();
    // 文件名里的 YYYYMMDD-HHMMSS 字典序和时间顺序一致，降序 = 最新在前
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
    for e in entries.into_iter().skip(keep) {
        let _ = fs::remove_file(e.path());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("floaty-snapshot-test-{}-{}", std::process::id(), line!()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn prune_keeps_most_recent_only() {
        let dir = unique_test_dir();
        for i in 0..15 {
            // floaty-YYYYMMDD-HHMMSS.db 格式；用 04 月 00..14 日拼出可排序的字符串
            let name = format!("{PREFIX}202604{i:02}-120000{SUFFIX}");
            fs::write(dir.join(&name), b"x").unwrap();
        }
        fs::write(dir.join("README.txt"), b"x").unwrap();
        prune(&dir, 10).unwrap();
        let remaining: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(remaining.iter().filter(|n| n.starts_with(PREFIX)).count(), 10);
        assert!(remaining.contains(&"README.txt".to_string()));
        // 确认保留的是"后 10 个"——即 05..=14 这些天
        assert!(remaining.iter().any(|n| n.contains("20260405")));
        assert!(remaining.iter().any(|n| n.contains("20260414")));
        assert!(!remaining.iter().any(|n| n.contains("20260400")));
        let _ = fs::remove_dir_all(&dir);
    }
}
