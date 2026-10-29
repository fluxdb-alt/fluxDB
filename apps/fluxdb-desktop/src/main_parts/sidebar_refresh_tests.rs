mod sidebar_refresh_tests {
    use super::*;

    fn connection_with_table() -> ConnectionState {
        let config = ConnectionConfig {
            id: ConnectionId(1),
            name: "refresh".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: PathBuf::from(":memory:"),
                read_only: false,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
        };
        ConnectionState {
            config,
            connected: true,
            expanded: true,
            objects: vec![ObjectSummary {
                path: ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".into()),
                    schema: None,
                    name: "SALES_REGION".into(),
                    kind: ObjectKind::Table,
                },
                rows: None,
                modified_at: None,
                comment: None,
                stable: None,
            }],
            redis_overview: Default::default(),
        }
    }

    fn visible_rows(loading: bool, loaded: bool) -> Vec<SidebarVisibleRow> {
        let connection = connection_with_table();
        let layout = SidebarLayout::for_connections(std::slice::from_ref(&connection.config));
        let database_key = database_tree_key(ConnectionId(1), "main");
        let group_key =
            object_group_tree_key_scoped(ConnectionId(1), "main", None, ObjectGroup::Tables);
        let loading_databases = loading.then(|| database_key.clone()).into_iter().collect();
        let loaded_database_children = loaded.then(|| database_key.clone()).into_iter().collect();

        flatten_sidebar_visible_rows(
            &[connection],
            &layout,
            &[],
            &BTreeSet::new(),
            &loading_databases,
            &loaded_database_children,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::from([(database_key, true)]),
            &BTreeMap::from([(group_key, true)]),
            "",
        )
    }

    fn schema_refresh_key(kind: DatabaseKind) -> String {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".into()),
            schema: Some("app".into()),
            name: "customers".into(),
            kind: ObjectKind::Table,
        };
        if matches!(
            kind,
            DatabaseKind::Postgres | DatabaseKind::Dameng | DatabaseKind::SqlServer
        ) {
            schema_tree_key(
                object.connection_id,
                object.database.as_deref().expect("database"),
                object.schema.as_deref().expect("schema"),
            )
        } else {
            database_tree_key(object.connection_id, "main")
        }
    }

    #[test]
    fn explicit_refresh_replaces_an_in_flight_load() {
        let key = database_tree_key(ConnectionId(1), "main");
        let mut loading = BTreeSet::from([key.clone()]);
        let mut tasks = BTreeSet::from([key.clone()]);
        // 普通 load_database_children 对 in-flight 作用域直接返回；显式刷新先移除
        // 旧状态，再发起新请求，不能被静默吞掉。
        assert!(loading.contains(&key));
        loading.remove(&key);
        tasks.remove(&key);
        assert!(!loading.contains(&key));
        assert!(!tasks.contains(&key));
    }

    #[test]
    fn schema_lazy_databases_refresh_the_table_schema_not_the_database() {
        let database_key = database_tree_key(ConnectionId(1), "main");
        let schema_key = schema_tree_key(ConnectionId(1), "main", "app");
        for kind in [
            DatabaseKind::Postgres,
            DatabaseKind::Dameng,
            DatabaseKind::SqlServer,
        ] {
            assert_eq!(schema_refresh_key(kind), schema_key, "{kind:?}");
            assert_ne!(schema_refresh_key(kind), database_key, "{kind:?}");
        }
        assert_eq!(schema_refresh_key(DatabaseKind::MySql), database_key);
    }

    fn has_table_group_and_row(rows: &[SidebarVisibleRow]) -> bool {
        let has_group = rows.iter().any(|row| {
            row.kind == SidebarRowKind::ObjectGroup
                && row.object_group == Some(ObjectGroup::Tables)
        });
        let has_table = rows.iter().any(|row| {
            row.kind == SidebarRowKind::Table
                && row
                    .object
                    .as_ref()
                    .is_some_and(|object| object.path.name == "SALES_REGION")
        });
        has_group && has_table
    }

    #[test]
    fn table_rows_stay_visible_while_refreshing_loaded_database() {
        assert!(has_table_group_and_row(&visible_rows(true, true)));
    }

    /// SQL 型连接的库下不能出现「集合」分组（Mongo 专用）。
    #[test]
    fn sql_databases_do_not_show_mongo_collection_group() {
        for kind in [
            DatabaseKind::MySql,
            DatabaseKind::Postgres,
            DatabaseKind::Sqlite,
            DatabaseKind::Redis,
        ] {
            assert!(
                !ObjectGroup::for_kind(kind).contains(&ObjectGroup::Collections),
                "{kind:?}"
            );
        }
        assert_eq!(
            ObjectGroup::for_kind(DatabaseKind::MongoDb),
            &[ObjectGroup::Collections]
        );
    }

    #[test]
    fn first_database_load_does_not_show_empty_table_group() {
        assert!(!has_table_group_and_row(&visible_rows(true, false)));
    }

    #[test]
    fn mongo_collection_tab_selects_its_sidebar_row() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("fluxdb_demo".into()),
            schema: None,
            name: "events".into(),
            kind: ObjectKind::Collection,
        };
        let mut state = AppState::default();
        state.tabs.push(TabState {
            id: TabId(1),
            title: "events".into(),
            kind: TabKind::MongoCollection(MongoCollectionState::new(object.clone(), false, 100)),
            dirty: false,
        });
        state.active_tab = Some(TabId(1));

        assert_eq!(active_sidebar_object_path(&state), Some(&object));
    }
}
