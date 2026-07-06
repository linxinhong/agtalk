//! 工具/诊断领域模块。

pub mod doctor;

use crate::config::AgConfig;
use crate::storage::Storage;
use std::path::PathBuf;

/// doctor 运行所需的上下文。
///
/// CLI 与 server handler 各自构造此上下文，然后调用 `doctor::run()`。
#[derive(Clone)]
pub struct DoctorContext {
    pub dot_agtalk: PathBuf,
    pub config: AgConfig,
    pub storage: Option<Storage>,
    pub as_name: Option<String>,
}

impl DoctorContext {
    pub fn new(
        dot_agtalk: PathBuf,
        config: AgConfig,
        storage: Option<Storage>,
        as_name: Option<String>,
    ) -> Self {
        Self {
            dot_agtalk,
            config,
            storage,
            as_name,
        }
    }
}
