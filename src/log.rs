use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 512 * 1024; // 512KB

fn log_dir() -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".claude").join("state");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn log_file() -> Option<PathBuf> {
    Some(log_dir()?.join("code_trace.log"))
}

fn rotate_if_needed(path: &std::path::Path) {
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.len() < MAX_LOG_BYTES {
        return;
    }
    let prev = path.with_extension("log.old");
    let _ = fs::rename(path, prev);
}

fn write_log(level: &str, msg: &str) {
    let Some(path) = log_file() else { return };
    rotate_if_needed(&path);
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(f, "{ts} [{level}] {msg}");
}

pub fn info(msg: &str) {
    write_log("INFO", msg);
}

pub fn error(msg: &str) {
    write_log("ERROR", msg);
}

pub fn debug(msg: &str) {
    if std::env::var("CC_TRACE_DEBUG").unwrap_or_default().to_lowercase() == "true" {
        write_log("DEBUG", msg);
    }
}
