//! SQLite 存储。**接口契约（不要改签名）**。表结构、PRAGMA、SQL 的语义与 `app/db.py` 逐字一致：
//! journal_mode=DELETE、synchronous=FULL、foreign_keys=ON、busy_timeout=5000、`BEGIN IMMEDIATE`。
//!
//! `Store` 内部是 `Mutex<rusqlite::Connection>`，所有方法都是**同步**的；API 层用 `spawn_blocking` 调用，
//! 锁不跨 `.await`。依据：`docs/audit/database.md`。
//!
//! 文件划分：
//! - `schema`           表结构文本（与旧版逐字相同）
//! - `store`            `Store` 本体：打开、题库、导入、学习记录
//! - `legacy_progress`  旧版 progress.json 的读取与逐项转换（Python `int()` / 真值语义）
//! - `timefmt`          本地时间 `%Y-%m-%d %H:%M` 与可替换的 `Clock`
//! - `pyjson`           与 Python `json.dumps(ensure_ascii=False)` 一致的写法（分隔符、键排序）

pub mod legacy_progress;
pub mod pyjson;
pub mod schema;
pub mod store;
pub mod timefmt;

pub use store::{percent_round_half_even, Store};
pub use timefmt::{Clock, FixedClock, FnClock, SystemClock};

pub const MASTER_STREAK: i64 = 2;
pub const HISTORY_LIMIT: i64 = 60;
