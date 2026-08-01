//! http_tests 共享 setup（cfg(test)）。

use crate::server::state::AppState;
use crate::storage::Storage;
use std::ffi::OsString;
use tempfile::TempDir;

use crate::config::AgConfig;
use crate::identity::{mailbox, session_file};

pub fn test_state() -> (AppState, String, String, TempDir) {
    let tmp = TempDir::new().unwrap();
    let dot = tmp.path().join(".agtalk");
    let storage = Storage::open_in_memory().unwrap();
    let nora = mailbox::create(&storage, "nora", "前端", "projA").unwrap();
    let quinn = mailbox::create(&storage, "quinn", "后端", "projB").unwrap();

    session_file::write(
        &dot,
        "nora",
        &session_file::SessionFile {
            version: 2,
            address: nora.clone(),
            name: "nora".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            registered_by: None,
            notify: session_file::SessionNotify {
                channel: "none".to_string(),
                endpoint: serde_json::Value::Null,
            },
        },
    )
    .unwrap();

    session_file::write(
        &dot,
        "quinn",
        &session_file::SessionFile {
            version: 2,
            address: quinn.clone(),
            name: "quinn".to_string(),
            intro: "后端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            registered_by: None,
            notify: session_file::SessionNotify {
                channel: "none".to_string(),
                endpoint: serde_json::Value::Null,
            },
        },
    )
    .unwrap();

    crate::mem::index::register(
        &storage,
        &nora,
        "nora",
        "projA",
        &dot.join("nora").join("memory"),
    );
    crate::mem::index::register(
        &storage,
        &quinn,
        "quinn",
        "projB",
        &dot.join("quinn").join("memory"),
    );

    let state = AppState::new(storage, AgConfig::default(), dot);
    (state, nora, quinn, tmp)
}

pub struct EnvGuard(Option<OsString>);

impl EnvGuard {
    pub fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("AGTALK_CONFIG_DIR");
        std::env::set_var("AGTALK_CONFIG_DIR", path);
        Self(previous)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(ref p) = self.0 {
            std::env::set_var("AGTALK_CONFIG_DIR", p);
        } else {
            std::env::remove_var("AGTALK_CONFIG_DIR");
        }
    }
}

pub fn browser_test_state() -> (AppState, TempDir, TempDir, EnvGuard) {
    let tmp = TempDir::new().unwrap();
    let browser_tmp = TempDir::new().unwrap();
    let guard = EnvGuard::set(browser_tmp.path());
    let storage = Storage::open_in_memory().unwrap();
    let state = AppState::new(storage, AgConfig::default(), tmp.path().join(".agtalk"));
    (state, tmp, browser_tmp, guard)
}
