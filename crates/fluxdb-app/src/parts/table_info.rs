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
    current.retain(|object| {
        let same_database = object.path.database.as_deref().unwrap_or("main") == database;
        !(same_database && matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
    });
    current.extend(children);
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
        // 备份列表 tab 按「连接 + 库」归属（与侧边栏备份节点一一对应）。
        TabKind::BackupList(list) => {
            list.connection_id == connection_id && list.database == database
        }
        TabKind::UserAdmin(_) | TabKind::Settings => false,
    }
}

fn path_database_name(path: &ObjectPath) -> &str {
    path.database.as_deref().unwrap_or("main")
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
        TabKind::CreateTable(create) => create.connection_id == connection_id,
        TabKind::UserAdmin(admin) => admin.connection_id == connection_id,
        TabKind::BackupList(list) => list.connection_id == connection_id,
        TabKind::Settings => false,
    }
}

#[allow(dead_code)]
fn _keep_query_request_visible(_: QueryRequest) {}
