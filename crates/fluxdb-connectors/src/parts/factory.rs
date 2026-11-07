/// 按连接配置创建统一的 Connector。
///
/// 数据库类型到具体实现的映射只保留在 connector 层；上层只依赖 `Connector` trait。
/// demo 连接统一使用内存实现，尚未实现真实连接器的类型在这里明确返回不支持。
pub fn connector_for(config: &ConnectionConfig) -> fluxdb_core::Result<Box<dyn Connector>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        // `demo.apply_delay_ms` 让 headless UI 测试能稳定复现「数据库响应慢」：
        // 只作用于内存 demo 连接，生产连接配置不会带这个键。
        let delay = config
            .options
            .get("demo.apply_delay_ms")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let fail_submit = config
            .options
            .get("demo.fail_submit")
            .is_some_and(|value| value == "true");
        return Ok(Box::new(MockConnector::with_apply_delay(
            config.kind,
            std::time::Duration::from_millis(delay),
        )
        .with_fail_submit(fail_submit)));
    }

    let connector: Box<dyn Connector> = match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            Box::new(MySqlConnector::with_config(config.clone()))
        }
        DatabaseKind::Sqlite => Box::new(SqliteConnector::with_config(config.clone())),
        // KingbaseES 在 PG 兼容模式下复用 PostgreSQL 连接器；Oracle 兼容模式不支持。
        DatabaseKind::Postgres | DatabaseKind::Kingbase => {
            Box::new(PostgresConnector::with_config(config.clone()))
        }
        DatabaseKind::Dameng => Box::new(DamengConnector::with_config(config.clone())),
        DatabaseKind::SqlServer => Box::new(SqlServerConnector::with_config(config.clone())),
        DatabaseKind::Oracle => Box::new(OracleConnector::with_config(config.clone())),
        DatabaseKind::Redis => Box::new(RedisConnector::with_config(config.clone())),
        DatabaseKind::MongoDb => Box::new(MongoConnector::with_config(config.clone())),
    };
    Ok(connector)
}

