//! 账号凭据、连接存储、本机登录识别、CLI 切换与浏览器官方授权。

pub(crate) mod agent_configs;
pub(crate) mod auth_snapshot;
pub(crate) mod browser;
pub(crate) mod catalog_backfill;
pub(crate) mod claude_auth;
pub(crate) mod cli_apply;
pub(crate) mod cli_switch;
pub mod connections;
pub(crate) mod creds;
pub(crate) mod current_connection;
pub(crate) mod local_active;
pub(crate) mod local_enroll;
pub(crate) mod login_detect;
pub(crate) mod opencode;
pub(crate) mod provider_key;
pub(crate) mod qoder_desktop;
