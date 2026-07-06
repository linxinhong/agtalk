//! `/api/v1/config/*` handler。

use crate::config::AgConfig;
use crate::proto::ServerMsg;
use crate::server::state::AppState;

pub fn handle_show(_state: &AppState) -> ServerMsg {
    match AgConfig::load() {
        Ok(cfg) => match serde_json::to_value(cfg) {
            Ok(config) => ServerMsg::ConfigShowResult { config },
            Err(e) => ServerMsg::Error {
                code: "config_error".into(),
                message: e.to_string(),
            },
        },
        Err(e) => ServerMsg::Error {
            code: "config_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_get(_state: &AppState, key: String) -> ServerMsg {
    match AgConfig::load() {
        Ok(cfg) => match cfg.get(&key) {
            Ok(value) => ServerMsg::ConfigValue { key, value },
            Err(e) => ServerMsg::Error {
                code: "config_error".into(),
                message: e.to_string(),
            },
        },
        Err(e) => ServerMsg::Error {
            code: "config_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_set(_state: &AppState, key: String, value: String) -> ServerMsg {
    match AgConfig::load() {
        Ok(mut cfg) => match cfg.set(&key, &value) {
            Ok(()) => ServerMsg::Pong,
            Err(e) => ServerMsg::Error {
                code: "config_error".into(),
                message: e.to_string(),
            },
        },
        Err(e) => ServerMsg::Error {
            code: "config_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_path(_state: &AppState) -> ServerMsg {
    match AgConfig::path() {
        Ok(path) => ServerMsg::ConfigPath {
            path: path.to_string_lossy().into_owned(),
        },
        Err(e) => ServerMsg::Error {
            code: "config_error".into(),
            message: e.to_string(),
        },
    }
}
