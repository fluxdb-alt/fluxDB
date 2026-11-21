//! 同步测试共用的 SQLite 临时库工具。
//!
//! 只在 `cfg(test)` 下编译：测试直接建真实连接、执行真实 SQL，不 mock 连接器。

use crate::{AppCommand, AppController, ConnectionDraft};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, DatabaseKind, Endpoint, QueryExecutionOptions, QueryMode, QueryRequest,
};

pub(crate) fn create_sqlite(
    controller: &mut AppController,
    name: &str,
    path: &std::path::Path,
) -> ConnectionConfig {
    controller.dispatch(AppCommand::CreateConnection(ConnectionDraft {
        name: name.into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: path.to_path_buf(),
            read_only: false,
        },
        options: std::collections::BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }));
    controller
        .connection_configs()
        .into_iter()
        .find(|config| config.name == name)
        .expect("created connection")
}

pub(crate) fn execute(config: &ConnectionConfig, sql: &str) {
    let result = connector_for(config)
        .expect("connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            session_id: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("execute");
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "{sql}"
    );
}

/// 为每个测试生成互不冲突的临时库路径。
pub(crate) fn temp_db(label: &str) -> std::path::PathBuf {
    let stamp = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    std::env::temp_dir().join(format!("fluxdb-sync-{label}-{stamp}.db"))
}
