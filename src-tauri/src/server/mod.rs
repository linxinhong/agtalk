//! HTTP server 与 daemon 生命周期。

pub mod daemon;
pub mod handlers;
pub mod http;
#[cfg(test)]
pub mod http_tests_common;
#[cfg(test)]
pub mod http_tests_id;
#[cfg(test)]
pub mod http_tests_mem;
#[cfg(test)]
pub mod http_tests_misc;
#[cfg(test)]
pub mod http_tests_msg;
pub mod state;
