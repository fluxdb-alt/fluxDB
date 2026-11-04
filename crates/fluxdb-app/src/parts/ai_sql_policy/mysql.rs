use fluxdb_core::DatabaseKind;
use sqlparser::dialect::{Dialect, MySqlDialect};

use super::{NamespaceScope, SqlPolicy};

pub static MYSQL: MySqlPolicy = MySqlPolicy;
/// MariaDB 与 MySQL 共用 SQL 语法与拒绝清单，只在类型标签上区分。
pub static MARIADB: MariaDbPolicy = MariaDbPolicy;

pub struct MySqlPolicy;
pub struct MariaDbPolicy;

impl SqlPolicy for MySqlPolicy {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MySql
    }

    fn label(&self) -> &'static str {
        "mysql"
    }

    fn parser_dialect(&self) -> &'static dyn Dialect {
        &MySqlDialect {}
    }

    fn function_denylist(&self) -> &'static [&'static str] {
        &[
            // 阻塞/计时/命名锁：可长时间占用连接或改写锁状态。
            "sleep",
            "benchmark",
            "get_lock",
            "release_lock",
            "release_all_locks",
            // 文件访问。
            "load_file",
        ]
    }

    fn relation_denylist(&self) -> &'static [&'static str] {
        &[
            "mysql.user",
            "performance_schema.threads",
            "performance_schema.processlist",
        ]
    }

    fn namespace_scope(&self) -> NamespaceScope {
        NamespaceScope::Database
    }

    fn read_only_show_heads(&self) -> &'static [&'static str] {
        &["INDEX", "INDEXES", "KEYS", "COLUMNS", "FIELDS", "TRIGGERS"]
    }
}

impl SqlPolicy for MariaDbPolicy {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MariaDb
    }

    fn label(&self) -> &'static str {
        "mariadb"
    }

    fn parser_dialect(&self) -> &'static dyn Dialect {
        MYSQL.parser_dialect()
    }

    fn function_denylist(&self) -> &'static [&'static str] {
        MYSQL.function_denylist()
    }

    fn relation_denylist(&self) -> &'static [&'static str] {
        MYSQL.relation_denylist()
    }

    fn namespace_scope(&self) -> NamespaceScope {
        MYSQL.namespace_scope()
    }

    fn read_only_show_heads(&self) -> &'static [&'static str] {
        MYSQL.read_only_show_heads()
    }
}
