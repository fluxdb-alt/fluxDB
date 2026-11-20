//! 数据同步 / 结构同步。
//!
//! 目录职责：
//! - `model`：对外契约（端点、选项、预览、报告）。
//! - `column_types`：目标列类型族与跨类型值规范化。
//! - `data_diff`：纯差异算法，可单测，不连库。
//! - `data_sync`：分页读取与执行编排。
//! - `inventory`：端点表清单与跨连接配对键。
//!
//! 结构同步复用 `inventory` 与 `SyncEndpoint`：
//! - `structure_diff`：纯对象级差异算法。
//! - `structure_sql`：方言语句生成与执行。
//! - `structure_sync`：预览与执行编排。
//! - `type_mapping`：跨数据库类型的字段类型/默认值映射。
//! - `cross_type_ddl`：跨类型建表与结构语句生成。

mod column_types;
mod cross_type_ddl;
mod data_diff;
mod data_sync;
mod inventory;
mod ledger;
mod model;
mod structure_diff;
mod structure_sql;
mod structure_sync;
#[cfg(test)]
mod test_support;
mod type_mapping;

pub use ledger::SyncRunDetail;
pub use model::*;
