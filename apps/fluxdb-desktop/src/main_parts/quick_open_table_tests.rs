mod quick_open_table_tests {
    use fluxdb_core::{ConnectionId, ObjectKind, ObjectPath};

    use crate::quick_open_table_matches;

    fn table(name: &str, schema: Option<&str>) -> ObjectPath {
        ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("app".into()),
            schema: schema.map(str::to_string),
            name: name.into(),
            kind: ObjectKind::Table,
        }
    }

    /// 空搜索保留全部候选，并按名称稳定排序。
    #[test]
    fn empty_search_keeps_all_tables_sorted_by_name() {
        let entries = vec![table("users", None), table("accounts", None)];
        let names = quick_open_table_matches(&entries, "  ")
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["accounts", "users"]);
    }

    /// 子序列命中即保留；完全无关的表被过滤。
    #[test]
    fn fuzzy_search_filters_unrelated_tables() {
        let entries = vec![
            table("orders", None),
            table("order_items", None),
            table("users", None),
        ];
        let mut names = quick_open_table_matches(&entries, "ord")
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["order_items", "orders"]);
    }

    /// schema 命中也能召回表，便于跨 schema 同名表按 schema 搜索。
    #[test]
    fn schema_match_includes_table() {
        let entries = vec![table("users", Some("public"))];
        let names = quick_open_table_matches(&entries, "pub")
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["users"]);
    }
}
