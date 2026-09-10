// PostgresConnector：实现同步 `Connector` trait（T04）。
//
// 整体保持 trait 的「无状态、每次调用按需拨号/复用会话」形态：`test_connection`
// 拨一条隔离连接做 `SELECT version()` 探活后即弃；`execute` 走共享 runtime 的会话。

#[derive(Clone, Default)]
pub struct PostgresConnector {
    config: Option<ConnectionConfig>,
}

impl PostgresConnector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }
}

impl Connector for PostgresConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Postgres
    }

    fn list_objects(&self, _path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        // 对象浏览接入在 T06；此处显式失败，不返回假数据。
        Err(Error::new(
            ErrorKind::Unsupported,
            "PostgreSQL 对象浏览尚未接入（T06）",
        ))
    }

    fn load_data(
        &self,
        _path: &ObjectPath,
        _offset: u64,
        _limit: u64,
        _sort: &[SortSpec],
        _filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        // 数据读取接入在 T07；此处显式失败。
        Err(Error::new(
            ErrorKind::Unsupported,
            "PostgreSQL 数据读取尚未接入（T07）",
        ))
    }

    fn apply_changes(&self, _changes: &DataChangeSet) -> fluxdb_core::Result<()> {
        // 数据编辑提交接入在后续任务；此处显式失败。
        Err(Error::new(
            ErrorKind::Unsupported,
            "PostgreSQL 数据编辑提交尚未接入",
        ))
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        if config.kind != DatabaseKind::Postgres {
            return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
        }
        let database = pg_request_database(config, None);
        // 拨一条隔离短连接做真实建连 + 认证 + 版本读取，用后即弃。
        // 单一 block_on：拨号与查询都 `.await`，避免嵌套 block_on。
        pg_runtime().block_on(async {
            let session = pg_connect(config, &database).await?;
            let row = tokio::time::timeout(Duration::from_secs(10), session.client.query_one("SELECT version()", &[]))
                .await
                .map_err(|_| Error::new(ErrorKind::Connection, "连接超时"))?
                .map_err(pg_error)?;
            let version: String = row
                .try_get(0)
                .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?;
            tracing::info!(
                target: "fluxdb_connectors",
                connection_id = ?config.id,
                connection = ?config.name,
                version = %version,
                "PostgreSQL 连接测试成功"
            );
            Ok::<_, fluxdb_core::Error>(())
        })
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL SQL 执行需要连接配置上下文"))?;
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?request.connection_id,
            database = ?request.database,
            sql = %truncate_sql_for_log(&request.text),
            "PostgreSQL SQL 执行开始"
        );
        pg_execute_query(config, request)
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL SQL 执行需要连接配置上下文"))?;
        pg_execute_query_with_progress(config, request, on_summary, should_cancel)
    }
}
