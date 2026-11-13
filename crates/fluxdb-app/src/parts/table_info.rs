impl TableInfoResult {
    fn tab(&self) -> TableInfoTab {
        match self {
            TableInfoResult::Indexes(_) => TableInfoTab::Indexes,
            TableInfoResult::ForeignKeys(_) => TableInfoTab::ForeignKeys,
            TableInfoResult::Triggers(_) => TableInfoTab::Triggers,
            TableInfoResult::Ddl(_) => TableInfoTab::Ddl,
        }
    }
}

fn mark_table_info_loading(table_info: &mut TableInfoState, tab: TableInfoTab) {
    match tab {
        TableInfoTab::Columns => {}
        TableInfoTab::Indexes
            if matches!(
                table_info.indexes,
                LoadState::NotLoaded | LoadState::Failed(_)
            ) =>
        {
            table_info.indexes = LoadState::Loading;
        }
        TableInfoTab::ForeignKeys
            if matches!(
                table_info.foreign_keys,
                LoadState::NotLoaded | LoadState::Failed(_)
            ) =>
        {
            table_info.foreign_keys = LoadState::Loading;
        }
        TableInfoTab::Triggers
            if matches!(
                table_info.triggers,
                LoadState::NotLoaded | LoadState::Failed(_)
            ) =>
        {
            table_info.triggers = LoadState::Loading;
        }
        TableInfoTab::Ddl
            if matches!(table_info.ddl, LoadState::NotLoaded | LoadState::Failed(_)) =>
        {
            table_info.ddl = LoadState::Loading;
        }
        _ => {}
    }
}

fn apply_table_info_result(table_info: &mut TableInfoState, result: TableInfoResult) {
    match result {
        TableInfoResult::Indexes(indexes) => table_info.indexes = LoadState::Loaded(indexes),
        TableInfoResult::ForeignKeys(foreign_keys) => {
            table_info.foreign_keys = LoadState::Loaded(foreign_keys);
        }
        TableInfoResult::Triggers(triggers) => table_info.triggers = LoadState::Loaded(triggers),
        TableInfoResult::Ddl(ddl) => table_info.ddl = LoadState::Loaded(ddl),
    }
}

fn set_table_info_failed(
    table_info: &mut TableInfoState,
    tab: TableInfoTab,
    error: UserFacingError,
) {
    match tab {
        TableInfoTab::Columns => {}
        TableInfoTab::Indexes => table_info.indexes = LoadState::Failed(error),
        TableInfoTab::ForeignKeys => table_info.foreign_keys = LoadState::Failed(error),
        TableInfoTab::Triggers => table_info.triggers = LoadState::Failed(error),
        TableInfoTab::Ddl => table_info.ddl = LoadState::Failed(error),
    }
}

/// 库级懒加载会整体替换的对象类型。
///
/// 集合必须在内：MongoDB 的集合列表与表同级加载，漏掉它会让每次刷新都把整份
/// 集合清单再追加一遍，侧边栏出现重复行。
const TABLE_LIKE_KINDS: &[ObjectKind] = &[
    ObjectKind::Table,
    ObjectKind::View,
    ObjectKind::Procedure,
    ObjectKind::Function,
    ObjectKind::Collection,
];

fn replace_loaded_children(
    current: &mut Vec<ObjectSummary>,
    parent: &ObjectPath,
    children: Vec<ObjectSummary>,
) {
    let database = parent
        .database
        .as_deref()
        .filter(|database| !database.is_empty())
        .unwrap_or(&parent.name);
    // PostgreSQL 按 schema 懒加载：展开某一 schema 时只替换该 schema 的表/视图，
    // 不得清掉同库其他 schema 已加载的对象（跨 schema 同名同时保留）。
    let schema_scope = object_path_schema(parent);
    // DB 级加载（schema_scope=None）时，仅当本次 children 真的是表/视图/集合清单
    // 才按整库清单替换；若是 PG 返回的 schema 列表（不含表/视图），则不能据此清空整库表，
    // 否则数据库节点上右键刷新会把所有已加载的表/视图误删。
    let clears_database_tables = schema_scope.is_none()
        && children
            .iter()
            .any(|child| TABLE_LIKE_KINDS.contains(&child.path.kind));
    current.retain(|object| {
        let same_database = object.path.database.as_deref().unwrap_or("main") == database;
        if !(same_database && TABLE_LIKE_KINDS.contains(&object.path.kind)) {
            return true;
        }
        match schema_scope {
            Some(schema) => object.path.schema.as_deref() != Some(schema),
            None => !clears_database_tables,
        }
    });
    current.extend(children);
}

/// 第一层对象（连接展开后直接可见的那一层）：库 / Schema / Redis 库。
/// 表 / 视图 / 集合 / Redis Key 属于更深一层，由 `LoadObjectChildren` 就地合入。
fn is_connection_level0(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Database | ObjectKind::Schema | ObjectKind::RedisDb
    )
}

/// 侧边栏「刷新连接树」的第一层替换：用服务端最新结果换掉旧的第一层条目，
/// 同时保留「所属库仍然存在」的表 / 视图行，使已展开的数据库子树不会因刷新被清空。
///
/// 库名比较沿用 `path_database_name`，与 [`replace_loaded_children`] 保持同一口径；
/// 库已被删除时其名下的表 / 视图行一并丢弃，避免留下孤儿行。
fn replace_connection_level0(connection: &mut ConnectionState, level0: Vec<ObjectSummary>) {
    let live_databases = level0
        .iter()
        .map(|object| path_database_name(&object.path).to_string())
        .collect::<BTreeSet<_>>();

    connection
        .objects
        .retain(|object| !is_connection_level0(object.path.kind));
    connection.objects.retain(|object| {
        live_databases.contains(path_database_name(&object.path))
    });
    connection.objects.extend(level0);
}

fn tab_belongs_to_database(tab: &TabState, connection_id: ConnectionId, database: &str) -> bool {
    match &tab.kind {
        TabKind::ObjectList(list) => list.parent.as_ref().is_some_and(|path| {
            path.connection_id == connection_id && path_database_name(path) == database
        }),
        TabKind::DataEditor(editor) => {
            editor.object.connection_id == connection_id
                && path_database_name(&editor.object) == database
        }
        TabKind::QueryEditor(editor) => {
            editor.connection_id == connection_id
                && editor.database.as_deref().unwrap_or("main") == database
        }
        TabKind::CreateTable(create) => {
            create.connection_id == connection_id
                && create.database.as_deref().unwrap_or("main") == database
        }
        // Redis Workbench 的库名即 DB 编号。
        TabKind::RedisWorkbench(workbench) => {
            workbench.connection_id == connection_id && workbench.database.to_string() == database
        }
        // Redis CLI 终端同样按「连接 + 库」归属。
        TabKind::RedisCli(cli) => {
            cli.connection_id == connection_id && cli.database.to_string() == database
        }
        // Redis Pub/Sub 会话面板同样按「连接 + 库」归属。
        TabKind::RedisPubSub(pubsub) => {
            pubsub.connection_id == connection_id && pubsub.database.to_string() == database
        }
        // MongoDB 命令行按「连接 + 库」归属。
        TabKind::MongoCli(cli) => cli.connection_id == connection_id && cli.database == database,
        // MongoDB 命令工作台同样按「连接 + 库」归属。
        TabKind::MongoWorkbench(workbench) => {
            workbench.connection_id == connection_id && workbench.database == database
        }
        // 备份列表 tab 按「连接 + 库」归属（与侧边栏备份节点一一对应）。
        TabKind::BackupList(list) => {
            list.connection_id == connection_id && list.database == database
        }
        TabKind::Settings(_) => false,
        TabKind::AiChat { workspace } => workspace.as_ref().is_some_and(|workspace| {
            workspace.connection_id == connection_id && workspace.database == database
        }),
        // ER 图 tab 按「连接 + 库」归属（与侧边栏 ER 节点一一对应）。
        TabKind::ErDiagram(er) => er.connection_id == connection_id && er.database == database,
        TabKind::MongoCollection(collection) => {
            collection.object.connection_id == connection_id
                && path_database_name(&collection.object) == database
        }
        // 服务器面板是连接级视图，不随某个库的删除而关闭。
        TabKind::UserAdmin(_)
        | TabKind::ServerPanel(_)
        | TabKind::RedisServerPanel(_)
        | TabKind::MongoServerPanel(_) => false,
    }
}

fn path_database_name(path: &ObjectPath) -> &str {
    path.database.as_deref().unwrap_or("main")
}

/// 对象的 schema 作用域：PG 对象携带 schema；其余数据库为 `None`。
fn object_path_schema(path: &ObjectPath) -> Option<&str> {
    path.schema.as_deref().filter(|schema| !schema.is_empty())
}

fn tab_belongs_to_connection(tab: &TabState, connection_id: ConnectionId) -> bool {
    match &tab.kind {
        TabKind::ObjectList(list) => list
            .parent
            .as_ref()
            .is_some_and(|path| path.connection_id == connection_id),
        TabKind::DataEditor(editor) => editor.object.connection_id == connection_id,
        TabKind::QueryEditor(editor) => editor.connection_id == connection_id,
        TabKind::RedisWorkbench(workbench) => workbench.connection_id == connection_id,
        TabKind::RedisCli(cli) => cli.connection_id == connection_id,
        TabKind::RedisPubSub(pubsub) => pubsub.connection_id == connection_id,
        TabKind::MongoCli(cli) => cli.connection_id == connection_id,
        TabKind::MongoWorkbench(workbench) => workbench.connection_id == connection_id,
        TabKind::CreateTable(create) => create.connection_id == connection_id,
        TabKind::UserAdmin(admin) => admin.connection_id == connection_id,
        TabKind::BackupList(list) => list.connection_id == connection_id,
        TabKind::ErDiagram(er) => er.connection_id == connection_id,
        TabKind::MongoCollection(collection) => collection.object.connection_id == connection_id,
        TabKind::ServerPanel(panel) => panel.connection_id == connection_id,
        TabKind::RedisServerPanel(panel) => panel.connection_id == connection_id,
        TabKind::MongoServerPanel(panel) => panel.connection_id == connection_id,
        TabKind::Settings(_) => false,
        TabKind::AiChat { workspace } => workspace
            .as_ref()
            .is_some_and(|workspace| workspace.connection_id == connection_id),
    }
}


#[cfg(test)]
mod replace_loaded_children_tests {
    use super::*;

    fn object(database: &str, name: &str, kind: ObjectKind) -> ObjectSummary {
        ObjectSummary {
            path: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some(database.into()),
                schema: None,
                name: name.into(),
                kind,
            },
            rows: None,
            size_bytes: None,
            modified_at: None,
            comment: None,
            stable: None,
        }
    }

    fn names(objects: &[ObjectSummary]) -> Vec<String> {
        objects
            .iter()
            .map(|object| object.path.name.clone())
            .collect()
    }

    // 回归：MongoDB 集合列表刷新必须替换而不是追加，否则侧边栏每次刷新都多一份重复行。
    #[test]
    fn refresh_replaces_mongo_collections() {
        let database = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("demo".into()),
            schema: None,
            name: "demo".into(),
            kind: ObjectKind::Database,
        };
        let mut current = vec![
            object("demo", "customers", ObjectKind::Collection),
            object("demo", "orders", ObjectKind::Collection),
        ];
        replace_loaded_children(
            &mut current,
            &database,
            vec![object("demo", "customers", ObjectKind::Collection)],
        );
        assert_eq!(names(&current), vec!["customers"]);
    }

    // 库级加载若返回的是 PG 的 schema 列表（不含表/视图/集合），不得清空已加载的表。
    #[test]
    fn schema_list_does_not_clear_tables() {
        let database = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("demo".into()),
            schema: None,
            name: "demo".into(),
            kind: ObjectKind::Database,
        };
        let mut current = vec![object("demo", "users", ObjectKind::Table)];
        replace_loaded_children(
            &mut current,
            &database,
            vec![object("demo", "public", ObjectKind::Schema)],
        );
        assert!(names(&current).contains(&"users".to_string()), "{current:?}");
    }
}
