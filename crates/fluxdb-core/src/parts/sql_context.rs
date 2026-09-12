// 查询会话与作用域标识（SQL 工作台通用）。
//
// `QuerySessionId` 标识一个可复用的查询会话（连接 + database/schema + 会话内事务/状态）；
// `QueryScopeKey` 提供 database(+schema) 级稳定标识，供 tab 去重、树节点、缓存、历史过滤、
// 异步 request key 复用同一份作用域，避免各处 `split('.')` 恢复对象名。

/// 一个可复用查询会话的稳定标识（进程内 / 连接生命周期内唯一）。
///
/// 仅作身份（不携带连接内容）；连接重连/重建会话可得新 ID，不跨进程持久化。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct QuerySessionId(pub u64);

impl std::fmt::Display for QuerySessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "q{}", self.0)
    }
}

/// 查询作用域：连接 + 可选 database + 可选 schema。
///
/// - PG 两段对象名是 `schema.table`，作用域内的 database 是物理连接库、schema 是 namespace。
/// - MySQL/TiDB 只有 database（schema 恒为 `None`）。
/// - Redis / SQLite 单库场景走 `database`（或 `None`）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryScope {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub schema: Option<String>,
}

impl QueryScope {
    /// 构造查询作用域。
    pub fn new(
        connection_id: ConnectionId,
        database: Option<String>,
        schema: Option<String>,
    ) -> Self {
        Self {
            connection_id,
            database,
            schema,
        }
    }

    /// 稳定、长度编码的字符串标识，供缓存 / 树节点 / 历史过滤复用。
    ///
    /// 采用长度前缀段（`conn`/`db`/`schema`），不以 `split('.')` 恢复对象名，
    /// 对象名含点号、空格、Unicode、双引号也不会产生歧义。
    pub fn key(&self) -> String {
        fn seg(prefix: &str, value: &str) -> String {
            format!("{}{}:{}:", prefix, value.len(), value)
        }
        let mut key = String::new();
        key.push_str(&seg("c", &self.connection_id.0.to_string()));
        if let Some(db) = &self.database {
            key.push_str(&seg("d", db));
        }
        if let Some(sch) = &self.schema {
            key.push_str(&seg("s", sch));
        }
        key
    }
}

#[cfg(test)]
mod sql_context_tests {
    use super::*;

    /// 结构体字段缺失时默认实例化（serde 兼容路径的分支之一）。
    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    struct ScopeCarrier {
        #[serde(default)]
        pub schema: Option<String>,
    }

    #[test]
    fn scope_key_is_unambiguous_with_dots_and_unicode() {
        let a = QueryScope::new(ConnectionId(1), Some("my.db".into()), Some("租户.A".into()));
        let b = QueryScope::new(ConnectionId(1), Some("my".into()), Some("db.租户.A".into()));
        // 长度前缀编码：两段看起来“像同一个串”的不同切分必须产生不同 key。
        assert_ne!(a.key(), b.key());
        assert_eq!(a.key(), QueryScope::new(ConnectionId(1), Some("my.db".into()), Some("租户.A".into())).key());
    }

    #[test]
    fn missing_schema_field_defaults_to_none() {
        // 模拟旧记录（无 schema 字段）：serde 默认加载为 None，不破坏反序列化。
        let json = r#"{"schema": null}"#;
        let c: ScopeCarrier = serde_json::from_str(json).unwrap();
        assert!(c.schema.is_none());
        let json2 = r#"{}"#;
        let c2: ScopeCarrier = serde_json::from_str(json2).unwrap();
        assert!(c2.schema.is_none());
    }
}
