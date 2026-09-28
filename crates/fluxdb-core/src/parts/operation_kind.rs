// 通用「操作类别」：跨数据库类型的规范化动作词。
//
// 各类型对同一件事有各自的说法（SQL 的 SELECT/INSERT/ALTER、Redis 的 GET/SET/CONFIG、
// Mongo 的 find/insertMany/createIndex…）。历史落盘前统一归一到这里的取值，使
// 「只看写操作」这类过滤与统计对所有数据库类型成立；新增数据库类型只加映射，不新增
// 存储结构。
//
// 取值不做穷举校验：认不出的取值按 `Unknown` 处理，保证新版本写入的新取值在旧版本读出
// 时不会反序列化失败。

/// 一条记录做了什么（与「历史类别」`category` 正交：category 是入口/形状，本类型是动作）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OperationKind {
    /// 只读取数：SELECT / SHOW / GET / KEYS / find…
    Query,
    /// 改数据：INSERT/UPDATE/DELETE / SET / DEL / insertMany…
    Write,
    /// 改结构：DDL / createIndex / dropCollection…
    Schema,
    /// 改实例状态、权限或配置：CONFIG SET / FLUSHALL / GRANT / KILL…
    Admin,
    /// 无法归类（也是将来新增取值的兜底）。
    #[default]
    Unknown,
}

impl OperationKind {
    /// 落盘取值（`history.operation_kind` 列）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Write => "write",
            Self::Schema => "schema",
            Self::Admin => "admin",
            Self::Unknown => "unknown",
        }
    }

    /// 从落盘值还原；认不出的取值按 `Unknown`（前向兼容，不报错）。
    pub fn from_storage(value: &str) -> Self {
        match value {
            "query" => Self::Query,
            "write" => Self::Write,
            "schema" => Self::Schema,
            "admin" => Self::Admin,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for OperationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
