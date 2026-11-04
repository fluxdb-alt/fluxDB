// 连接级只读策略：按数据库类型分类写入意图，并在共享执行入口统一拦截。
//
// 判定复用 AI 侧已有的分类器（SQL 语句风险、Redis 命令白名单），避免同一套规则
// 写两遍；Mongo 没有现成分类器，按 `runCommand` 命令名做保守白名单。
//
// 只读是**客户端护栏**，不替代数据库账号权限：会话级只读可被 DDL 隐式提交、
// 高权限账号等绕过，真正的边界仍是服务端授权。

use fluxdb_core::{ReadOnlyPolicy, WriteIntent};

/// SQL 方言只读策略：复用 AI 的语句风险分类器。
struct SqlReadOnlyPolicy {
    kind: DatabaseKind,
}

impl ReadOnlyPolicy for SqlReadOnlyPolicy {
    fn allows(&self, intent: &WriteIntent<'_>) -> bool {
        let WriteIntent::Sql(text) = intent else {
            return false;
        };
        let Some(policy) = policy_for(self.kind) else {
            return false;
        };
        let Ok(statements) = sqlparser::parser::Parser::parse_sql(policy.parser_dialect(), text.trim())
        else {
            // 解析失败无法证明只读，按写处理。
            return false;
        };
        if statements.is_empty() {
            return false;
        }
        statements
            .iter()
            .all(|statement| sql_statement_allowed(statement, policy))
    }
}

/// 只读连接允许的 SQL：只读语句、事务边界、库切换，以及仅因“元数据暴露”
/// 被 AI 策略标为需授权的 `SHOW …`。
///
/// AI 的分类器把 `SHOW DATABASES` 等视为信息暴露风险；连接只读关心的是“会不会
/// 写数据”，这类纯读语句必须放行，否则只读连接连库列表都查不了。
fn sql_statement_allowed(statement: &Statement, policy: &dyn SqlPolicy) -> bool {
    // 事务边界与 `USE db` 只改会话状态，不写数据。
    if is_transaction_boundary(statement) || matches!(statement, Statement::Use(_)) {
        return true;
    }
    match analyze_sql(&statement.to_string(), policy) {
        SqlRisk::ReadOnly => true,
        SqlRisk::RequiresAuthorization { reasons } => reasons
            .iter()
            .all(|reason| reason.starts_with("metadata_disclosure:")),
        SqlRisk::Rejected { .. } => false,
    }
}

fn is_transaction_boundary(statement: &Statement) -> bool {
    matches!(
        statement,
        Statement::StartTransaction { .. }
            | Statement::Commit { .. }
            | Statement::Rollback { .. }
            | Statement::Savepoint { .. }
            | Statement::ReleaseSavepoint { .. }
    )
}

/// Redis 只读策略：逐行复用命令白名单；无法识别的命令一律按写处理。
struct RedisReadOnlyPolicy;

/// AI 白名单偏保守（未列入的命令要人工授权），这里补充确定只读、且不暴露其他
/// 客户端信息的命令，避免只读连接连 `PING` 都执行不了。
///
/// `KEYS` 在 AI 侧因全库遍历/阻塞风险被拒绝；只读连接关心的是“会不会写”，
/// 它仍是纯读命令，放行。
const REDIS_EXTRA_READ_ONLY: &[&str] = &["ping", "echo", "command", "keys"];

impl ReadOnlyPolicy for RedisReadOnlyPolicy {
    fn allows(&self, intent: &WriteIntent<'_>) -> bool {
        let WriteIntent::RedisCommand(text) = intent else {
            return false;
        };
        let commands = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let mut seen = false;
        for line in commands {
            seen = true;
            let argv: Vec<String> = line.split_whitespace().map(str::to_string).collect();
            if !redis_argv_allowed(&argv) {
                return false;
            }
        }
        seen
    }
}

/// 单条 Redis 命令是否属于只读。
///
/// 供 CLI 只读代理复用：代理拿到的是已解析的 argv，与 Workbench 的文本路径共用同一套判定。
pub(crate) fn redis_argv_allowed(argv: &[String]) -> bool {
    argv.first().is_some_and(|command| {
        let command = command.to_ascii_lowercase();
        REDIS_EXTRA_READ_ONLY.contains(&command.as_str())
            || classify_redis_command(argv).is_read_only()
    })
}

/// MongoDB 只读策略：`runCommand` 只放行明确只读的命令；`aggregate` 额外拒绝写管道。
struct MongoReadOnlyPolicy;

/// 明确只读的 MongoDB 命令（保守子集）。
const MONGO_READ_ONLY_COMMANDS: &[&str] = &[
    "find",
    "count",
    "distinct",
    "listcollections",
    "listindexes",
    "listdatabases",
    "dbstats",
    "collstats",
    "explain",
    "ping",
    "hello",
    "ismaster",
    "buildinfo",
    "serverstatus",
    "connectionstatus",
];

impl ReadOnlyPolicy for MongoReadOnlyPolicy {
    fn allows(&self, intent: &WriteIntent<'_>) -> bool {
        let WriteIntent::MongoCommand(text) = intent else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
            return false;
        };
        let Some(object) = value.as_object() else {
            return false;
        };
        // 跳过 `$db` / `$readPreference` 等元字段：命令名是第一个普通 key。
        let Some(command) = object
            .keys()
            .find(|key| !key.starts_with('$'))
            .map(|key| key.to_ascii_lowercase())
        else {
            return false;
        };
        if command == "aggregate" {
            // `$out` / `$merge` 会把结果写回集合。
            let pipeline = serde_json::to_string(object.get("pipeline").unwrap_or(&value))
                .unwrap_or_default()
                .to_ascii_lowercase();
            return !pipeline.contains("\"$out\"") && !pipeline.contains("\"$merge\"");
        }
        MONGO_READ_ONLY_COMMANDS.contains(&command.as_str())
    }
}

fn read_only_policy_for(kind: DatabaseKind) -> &'static dyn ReadOnlyPolicy {
    static SQLITE: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::Sqlite,
    };
    static MYSQL: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::MySql,
    };
    static TIDB: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::TiDb,
    };
    static MARIADB: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::MariaDb,
    };
    static POSTGRES: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::Postgres,
    };
    static SQLSERVER: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::SqlServer,
    };
    static ORACLE: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::Oracle,
    };
    static DAMENG: SqlReadOnlyPolicy = SqlReadOnlyPolicy {
        kind: DatabaseKind::Dameng,
    };
    static REDIS: RedisReadOnlyPolicy = RedisReadOnlyPolicy;
    static MONGO: MongoReadOnlyPolicy = MongoReadOnlyPolicy;

    match kind {
        DatabaseKind::Sqlite => &SQLITE,
        DatabaseKind::MySql => &MYSQL,
        DatabaseKind::TiDb => &TIDB,
        DatabaseKind::MariaDb => &MARIADB,
        DatabaseKind::Postgres => &POSTGRES,
        DatabaseKind::SqlServer => &SQLSERVER,
        DatabaseKind::Oracle => &ORACLE,
        DatabaseKind::Dameng => &DAMENG,
        DatabaseKind::Redis => &REDIS,
        DatabaseKind::MongoDb => &MONGO,
    }
}

/// 只读连接下拦截写入意图；非只读连接直接放行。
///
/// 所有写入口（SQL 执行、表数据编辑、导入、Redis 命令、Mongo 文档/集合操作）
/// 都必须经过这里，避免只挡住部分 UI 路径。
pub fn ensure_read_only_allows(
    config: &ConnectionConfig,
    intent: WriteIntent<'_>,
) -> fluxdb_core::Result<()> {
    if !connection_is_read_only(config) {
        return Ok(());
    }
    if read_only_policy_for(config.kind).allows(&intent) {
        return Ok(());
    }
    Err(Error::new(ErrorKind::Permission, "连接已设为只读，未执行"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::Endpoint;

    fn config(kind: DatabaseKind, read_only: bool) -> ConnectionConfig {
        let mut options = std::collections::BTreeMap::new();
        if read_only {
            options.insert("read_only".to_string(), "true".to_string());
        }
        ConnectionConfig {
            id: ConnectionId(1),
            name: "test".into(),
            kind,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
                database: None,
            },
            options,
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    #[test]
    fn sql_read_only_allows_select_and_rejects_writes() {
        let config = config(DatabaseKind::MySql, true);
        for sql in [
            "SELECT 1",
            "select * from t",
            "SHOW TABLES",
            "SHOW DATABASES",
            "SHOW VARIABLES",
            "USE other_db",
        ] {
            assert!(
                ensure_read_only_allows(&config, WriteIntent::Sql(sql)).is_ok(),
                "{sql}"
            );
        }
        for sql in [
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a = 1",
            "DELETE FROM t",
            "DROP TABLE t",
            "CREATE TABLE t (id INT)",
        ] {
            assert!(
                ensure_read_only_allows(&config, WriteIntent::Sql(sql)).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn sql_read_only_allows_multi_statement_reads_and_transaction_boundaries() {
        let config = config(DatabaseKind::Postgres, true);
        assert!(
            ensure_read_only_allows(
                &config,
                WriteIntent::Sql("BEGIN; SELECT 1; SELECT 2; COMMIT;")
            )
            .is_ok()
        );
        assert!(
            ensure_read_only_allows(&config, WriteIntent::Sql("BEGIN; DELETE FROM t; COMMIT;"))
                .is_err()
        );
    }

    #[test]
    fn non_read_only_connection_is_not_blocked() {
        let config = config(DatabaseKind::MySql, false);
        assert!(
            ensure_read_only_allows(&config, WriteIntent::Sql("DELETE FROM t")).is_ok()
        );
    }

    #[test]
    fn mutation_is_always_blocked() {
        let config = config(DatabaseKind::MongoDb, true);
        assert!(ensure_read_only_allows(&config, WriteIntent::Mutation).is_err());
    }

    #[test]
    fn redis_read_only_uses_command_whitelist() {
        let config = config(DatabaseKind::Redis, true);
        assert!(ensure_read_only_allows(&config, WriteIntent::RedisCommand("GET k\nKEYS *")).is_ok());
        assert!(ensure_read_only_allows(&config, WriteIntent::RedisCommand("PING")).is_ok());
        assert!(ensure_read_only_allows(&config, WriteIntent::RedisCommand("GET k\nSET k v")).is_err());
        assert!(ensure_read_only_allows(&config, WriteIntent::RedisCommand("CONFIG GET maxmemory")).is_err());
    }

    #[test]
    fn mongo_read_only_blocks_writes_and_aggregate_out() {
        let config = config(DatabaseKind::MongoDb, true);
        assert!(
            ensure_read_only_allows(
                &config,
                WriteIntent::MongoCommand(r#"{"find":"users","filter":{}}"#)
            )
            .is_ok()
        );
        assert!(
            ensure_read_only_allows(
                &config,
                WriteIntent::MongoCommand(r#"{"$db":"app","find":"users"}"#)
            )
            .is_ok()
        );
        assert!(
            ensure_read_only_allows(
                &config,
                WriteIntent::MongoCommand(r#"{"aggregate":"users","pipeline":[{"$out":"x"}],"cursor":{}}"#)
            )
            .is_err()
        );
        assert!(
            ensure_read_only_allows(
                &config,
                WriteIntent::MongoCommand(r#"{"delete":"users","deletes":[]}"#)
            )
            .is_err()
        );
    }
}
