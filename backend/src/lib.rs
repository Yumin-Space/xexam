//! Xexam 后端（库）。二进制在 `main.rs`。
//!
//! 模块职责：
//! - `config`    环境变量与启动参数（XEXAM_*）
//! - `models`    数据模型（字段顺序 = 旧版 JSON 键顺序）
//! - `security`  Host / Origin 守卫、令牌、请求体限制、文件名清洗
//! - `parser`    XLSX 解析（zip + quick-xml），题目 id，翻译拆分
//! - `database`  SQLite 存储（schema 与旧版逐字相同）
//! - `services`  导入流程（parse_upload → … → apply_import）、待确认槽位、成绩记录
//! - `api`       HTTP 路由与 JSON 输出

pub mod api;
pub mod config;
pub mod database;
pub mod error;
pub mod models;
pub mod parser;
pub mod security;
pub mod services;
