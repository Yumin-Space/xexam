//! HTTP 路由。**接口契约（不要改签名）**。
//!
//! `build_router(state)` 返回完整的 `axum::Router`，供 `main.rs` 和集成测试（`tower::ServiceExt::oneshot`）共用。
//! 行为依据 `docs/audit/api-contract.md` 与 `tests/baseline/golden/`。
use crate::config::Config;
use crate::database::Store;
use crate::services::PendingStore;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub store: Arc<Store>,
    pub pending: PendingStore,
    /// 只有 `--allow-shutdown` 时才有
    pub shutdown: Option<tokio::sync::mpsc::Sender<()>>,
}

pub fn build_router(_state: AppState) -> axum::Router {
    unimplemented!("api::build_router")
}
