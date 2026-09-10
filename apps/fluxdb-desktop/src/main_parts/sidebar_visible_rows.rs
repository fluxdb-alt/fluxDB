#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidebarRowKind {
    Connection,
    Group,
    Database,
    ObjectGroup,
    Table,
    SavedQuery,
    TableFolder,
}

impl SidebarRowKind {
    /// 该行固定高度（px）：连接/分组行 30，其余 26，与 tree_helpers 各构建函数 `.h()` 一致。
    fn height(self) -> f32 {
        match self {
            SidebarRowKind::Connection | SidebarRowKind::Group => 30.,
            _ => 26.,
        }
    }
}

/// sidebar 拉平后的一行。只携带渲染各 `*_tree` 构建函数所需的「每行变量」，
/// expanded/selected/pinned/connecting 等环境量在虚拟化闭包里按行现算，不落进缓存。
#[derive(Clone, Debug)]
struct SidebarVisibleRow {
    pub kind: SidebarRowKind,
    /// 树的缩进深度（0..=4），直接传给 `*_tree(indent, ...)`。
    pub indent: u8,
    /// 稳定行标识：唯一身份锚点；仅被 flatten 单测断言（`#[cfg(test)]`），
    /// 运行时构建函数走自身 ElementId，故显式放行 dead_code。
    #[allow(dead_code)]
    pub key: String,
    pub connection_id: ConnectionId,
    /// Connection/Group 行所属分组（顶层连接为 None）。
    pub group_id: Option<ConnectionGroupId>,
    /// Database 行：数据库 ObjectPath；ObjectGroup 行：所在数据库 ObjectPath。
    pub database_path: Option<ObjectPath>,
    /// ObjectGroup 行：数据库显示名（构建 object_group_tree 用）。
    pub database_name: Option<String>,
    /// ObjectGroup 行：分组种类。
    pub object_group: Option<ObjectGroup>,
    /// Table 行：表对象。
    pub object: Option<ObjectSummary>,
    /// SavedQuery 行：完整查询对象（构建 saved_query_tree 用）。
    pub query: Option<SavedQuery>,
    /// TableFolder 行：父 key + 文件夹名。
    pub folder_parent_key: Option<String>,
    pub folder_name: Option<String>,
}

impl SidebarVisibleRow {
    /// 该行固定高度。
    fn height(&self) -> f32 {
        self.kind.height()
    }
}

/// 把连接浏览器整树拉平为「可见行」列表，供 VirtualList 只渲染视口附近的行。
///
/// 语义与 sidebar.rs 旧 `sidebar()` 整树遍历**逐行对齐**：
/// - `sidebar_layout.order` 决定顺序（顶层连接/分组交错）；
/// - 连接的子树只在 `connection.expanded` 时下钻（复用 `connection_should_show_children`）；
/// - 数据库下钻 `ObjectGroup::ALL` 精确镜像 search-active / 正常两分支，含 `matching_groups` 过滤
///   与 `continue` 跳过（search 下无匹配内容时整库隐藏）；
/// - 折叠子树不进列表（这是省成本的关键：只保留可见行）。
///
/// search 时数据库/分组强制展开（`tree_expanded_for_search`），与旧遍历一致。
#[allow(clippy::too_many_arguments)]
fn flatten_sidebar_visible_rows(
    connections: &[ConnectionState],
    sidebar_layout: &SidebarLayout,
    saved_queries: &[SavedQuery],
    _connecting_connections: &BTreeSet<ConnectionId>,
    loading_databases: &BTreeSet<String>,
    _loaded_database_children: &BTreeSet<String>,
    pinned_databases: &BTreeSet<String>,
    pinned_tables: &BTreeSet<String>,
    table_folders: &BTreeMap<String, Vec<String>>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
    expanded_databases: &BTreeMap<String, bool>,
    expanded_object_groups: &BTreeMap<String, bool>,
    search_query: &str,
) -> Vec<SidebarVisibleRow> {
    let search_active = !search_query.is_empty();
    let mut rows = Vec::new();

    for entry in &sidebar_layout.order {
        match entry {
            SidebarOrderEntry::Connection { id } => {
                let Some(connection) = connections
                    .iter()
                    .find(|connection| connection.config.id == *id)
                else {
                    continue;
                };
                if search_active
                    && !connection_matches_sidebar_search(connection, saved_queries, search_query)
                {
                    continue;
                }
                push_connection_visible_rows(
                    connection,
                    0,
                    None,
                    _connecting_connections,
                    loading_databases,
                    _loaded_database_children,
                    pinned_databases,
                    pinned_tables,
                    table_folders,
                    table_folder_assignments,
                    expanded_databases,
                    expanded_object_groups,
                    saved_queries,
                    search_active,
                    search_query,
                    &mut rows,
                );
            }
            SidebarOrderEntry::Group {
                id,
                connection_ids,
            } => {
                let Some(group) = sidebar_layout
                    .groups
                    .iter()
                    .find(|group| group.id == *id)
                else {
                    continue;
                };
                if search_active
                    && !connection_ids.iter().any(|connection_id| {
                        connections
                            .iter()
                            .find(|connection| connection.config.id == *connection_id)
                            .is_some_and(|connection| {
                                connection_matches_sidebar_search(
                                    connection,
                                    saved_queries,
                                    search_query,
                                )
                            })
                    })
                {
                    continue;
                }
                rows.push(SidebarVisibleRow {
                    kind: SidebarRowKind::Group,
                    indent: 0,
                    key: format!("group:{}", group.id.0),
                    connection_id: ConnectionId(0),
                    group_id: Some(group.id),
                    database_path: None,
                    database_name: None,
                    object_group: None,
                    object: None,
                    query: None,
                    folder_parent_key: None,
                    folder_name: None,
                });
                if !group.collapsed {
                    for connection_id in connection_ids {
                        let Some(connection) = connections
                            .iter()
                            .find(|connection| connection.config.id == *connection_id)
                        else {
                            continue;
                        };
                        if search_active
                            && !connection_matches_sidebar_search(
                                connection,
                                saved_queries,
                                search_query,
                            )
                        {
                            continue;
                        }
                        push_connection_visible_rows(
                            connection,
                            1,
                            Some(group.id),
                            _connecting_connections,
                            loading_databases,
                            _loaded_database_children,
                            pinned_databases,
                            pinned_tables,
                            table_folders,
                            table_folder_assignments,
                            expanded_databases,
                            expanded_object_groups,
                            saved_queries,
                            search_active,
                            search_query,
                            &mut rows,
                        );
                    }
                }
            }
        }
    }

    rows
}

#[allow(clippy::too_many_arguments)]
fn push_connection_visible_rows(
    connection: &ConnectionState,
    indent: u8,
    group_id: Option<ConnectionGroupId>,
    _connecting_connections: &BTreeSet<ConnectionId>,
    loading_databases: &BTreeSet<String>,
    _loaded_database_children: &BTreeSet<String>,
    pinned_databases: &BTreeSet<String>,
    pinned_tables: &BTreeSet<String>,
    table_folders: &BTreeMap<String, Vec<String>>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
    expanded_databases: &BTreeMap<String, bool>,
    expanded_object_groups: &BTreeMap<String, bool>,
    saved_queries: &[SavedQuery],
    search_active: bool,
    search_query: &str,
    rows: &mut Vec<SidebarVisibleRow>,
) {
    let connection_id = connection.config.id;
    rows.push(SidebarVisibleRow {
        kind: SidebarRowKind::Connection,
        indent,
        key: format!("connection:{}", connection_id.0),
        connection_id,
        group_id,
        database_path: None,
        database_name: None,
        object_group: None,
        object: None,
        query: None,
        folder_parent_key: None,
        folder_name: None,
    });

    // 与旧整树构建一致：仅连接展开时下钻子树。
    if !connection_should_show_children(connection.expanded, search_active) {
        return;
    }

    let databases = if search_active && (!connection.connected || connection.objects.is_empty()) {
        Vec::new()
    } else {
        connection_databases(connection)
    };
    let mut databases = databases;
    databases.sort_by_key(|database| {
        let name = database_display_name(database);
        (
            !pinned_databases.contains(&database_tree_key(connection_id, &name)),
            name,
        )
    });

    for database in databases {
        let database_name = database
            .path
            .database
            .clone()
            .unwrap_or_else(|| database.path.name.clone());
        let matching_groups = if search_active {
            ObjectGroup::ALL
                .iter()
                .copied()
                .filter(|group| match group {
                    ObjectGroup::Queries => saved_queries_for_database(
                        saved_queries,
                        connection_id,
                        &database_name,
                    )
                    .into_iter()
                    .any(|saved| search_matches_text(&saved.name, search_query)),
                    ObjectGroup::Tables => {
                        group_objects(connection, &database_name, *group)
                            .into_iter()
                            .any(|object| object_matches_sidebar_search(object, search_query))
                            || sorted_table_folders(table_folders, connection_id, &database_name)
                                .into_iter()
                                .any(|folder| search_matches_text(folder, search_query))
                    }
                    _ => group_objects(connection, &database_name, *group)
                        .into_iter()
                        .any(|object| object_matches_sidebar_search(object, search_query)),
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if search_active && matching_groups.is_empty() {
            continue;
        }

        let database_key = database_tree_key(connection_id, &database_name);
        let database_expanded = if database.path.kind == ObjectKind::RedisDb {
            false
        } else {
            tree_expanded_for_search(
                expanded_databases.get(&database_key).copied(),
                search_active,
            )
        };
        let database_loading = loading_databases.contains(&database_key);

        rows.push(SidebarVisibleRow {
            kind: SidebarRowKind::Database,
            indent: indent + 1,
            key: database_key,
            connection_id,
            group_id,
            database_path: Some(database.path.clone()),
            database_name: Some(database_name.clone()),
            object_group: None,
            object: None,
            query: None,
            folder_parent_key: None,
            folder_name: None,
        });

        let show_groups = database.path.kind != ObjectKind::RedisDb && database_expanded;
        if search_active && database_expanded {
            for group in matching_groups {
                if show_groups {
                    push_object_group_visible_rows(
                        connection,
                        &database,
                        &database_name,
                        group,
                        group_id,
                        indent + 2,
                        search_active,
                        search_query,
                        _connecting_connections,
                        loading_databases,
                        _loaded_database_children,
                        pinned_tables,
                        table_folders,
                        table_folder_assignments,
                        expanded_object_groups,
                        saved_queries,
                        rows,
                    );
                }
            }
        } else if show_groups && !database_loading {
            for group in ObjectGroup::ALL {
                push_object_group_visible_rows(
                    connection,
                    &database,
                    &database_name,
                    group,
                    group_id,
                    indent + 2,
                    search_active,
                    search_query,
                    _connecting_connections,
                    loading_databases,
                    _loaded_database_children,
                    pinned_tables,
                    table_folders,
                    table_folder_assignments,
                    expanded_object_groups,
                    saved_queries,
                    rows,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_object_group_visible_rows(
    connection: &ConnectionState,
    database: &ObjectSummary,
    database_name: &str,
    group: ObjectGroup,
    group_id: Option<ConnectionGroupId>,
    indent: u8,
    search_active: bool,
    search_query: &str,
    _connecting_connections: &BTreeSet<ConnectionId>,
    _loading_databases: &BTreeSet<String>,
    _loaded_database_children: &BTreeSet<String>,
    pinned_tables: &BTreeSet<String>,
    table_folders: &BTreeMap<String, Vec<String>>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
    expanded_object_groups: &BTreeMap<String, bool>,
    saved_queries: &[SavedQuery],
    rows: &mut Vec<SidebarVisibleRow>,
) {
    let connection_id = connection.config.id;
    let group_key = object_group_tree_key(connection_id, database_name, group);
    let group_expanded = if search_active {
        tree_expanded_for_search(expanded_object_groups.get(&group_key).copied(), true)
    } else {
        expanded_object_groups.get(&group_key).copied().unwrap_or(false)
    };

    rows.push(SidebarVisibleRow {
        kind: SidebarRowKind::ObjectGroup,
        indent,
        key: group_key,
        connection_id,
        group_id,
        database_path: Some(database.path.clone()),
        database_name: Some(database_name.to_string()),
        object_group: Some(group),
        object: None,
        query: None,
        folder_parent_key: None,
        folder_name: None,
    });

    // 备份节点：不再展开子行——单击直接打开该库的备份列表 tab（见 object_group_tree）。
    if group == ObjectGroup::Backup {
        return;
    }

    if !group_expanded {
        return;
    }

    if group == ObjectGroup::Queries {
        let queries = saved_queries_for_database(saved_queries, connection_id, database_name);
        for query in queries.into_iter().filter(|saved| {
            !search_active || search_matches_text(&saved.name, search_query)
        }) {
            rows.push(SidebarVisibleRow {
                kind: SidebarRowKind::SavedQuery,
                indent: indent + 1,
                key: format!("query:{}", query.id),
                connection_id,
                group_id,
                database_path: None,
                database_name: None,
                object_group: None,
                object: None,
                query: Some(query.clone()),
                folder_parent_key: None,
                folder_name: None,
            });
        }
        return;
    }

    if group == ObjectGroup::Tables {
        let folder_parent_key = table_folder_parent_key(connection_id, database_name);
        let folders = sorted_table_folders(table_folders, connection_id, database_name);
        for folder in folders {
            let folder_tables = sorted_folder_table_objects(
                connection,
                database_name,
                &folder_parent_key,
                folder,
                pinned_tables,
                table_folder_assignments,
            );
            let folder_matches =
                !search_active
                    || search_matches_text(folder, search_query)
                        || folder_tables
                            .iter()
                            .any(|object| object_matches_sidebar_search(object, search_query));
            if !folder_matches {
                continue;
            }
            let folder_expanded = if search_active {
                tree_expanded_for_search(
                    expanded_object_groups
                        .get(&table_folder_tree_key(&folder_parent_key, folder))
                        .copied(),
                    true,
                )
            } else {
                expanded_object_groups
                    .get(&table_folder_tree_key(&folder_parent_key, folder))
                    .copied()
                    .unwrap_or(true)
            };
            let folder_key = table_folder_tree_key(&folder_parent_key, folder);
            rows.push(SidebarVisibleRow {
                kind: SidebarRowKind::TableFolder,
                indent: indent + 1,
                key: folder_key.clone(),
                connection_id,
                group_id,
                database_path: Some(database.path.clone()),
                database_name: Some(database_name.to_string()),
                object_group: None,
                object: None,
                query: None,
                folder_parent_key: Some(folder_parent_key.clone()),
                folder_name: Some(folder.clone()),
            });
            if folder_expanded {
                for object in folder_tables.into_iter().filter(|object| {
                    !search_active || object_matches_sidebar_search(object, search_query)
                }) {
                    push_table_visible_row(
                        object,
                        indent + 2,
                        connection_id,
                        group_id,
                        rows,
                    );
                }
            }
        }
    }

    let objects = if group == ObjectGroup::Tables {
        sorted_unassigned_group_objects(
            connection,
            database_name,
            group,
            pinned_tables,
            table_folder_assignments,
        )
    } else {
        sorted_group_objects(connection, database_name, group, pinned_tables)
    };
    for object in objects.into_iter().filter(|object| {
        !search_active || object_matches_sidebar_search(object, search_query)
    }) {
        push_table_visible_row(object, indent + 1, connection_id, group_id, rows);
    }
}

fn push_table_visible_row(
    object: &ObjectSummary,
    indent: u8,
    connection_id: ConnectionId,
    group_id: Option<ConnectionGroupId>,
    rows: &mut Vec<SidebarVisibleRow>,
) {
    rows.push(SidebarVisibleRow {
        kind: SidebarRowKind::Table,
        indent,
        key: table_tree_key(&object.path),
        connection_id,
        group_id,
        database_path: None,
        database_name: None,
        object_group: None,
        object: Some(object.clone()),
        query: None,
        folder_parent_key: None,
        folder_name: None,
    });
}

/// 把拉平后的一行还原为对应 `*_tree` 构建函数的渲染结果（`AnyElement`）。
///
/// 行数据结构（`SidebarVisibleRow`）只存「结构标识」；expanded/pinned/selected/loading/
/// connecting 等环境量在此处按行从 `this`（NavicatMain + AppState）现算，不落进缓存。
/// 这样只有结构变化才会失效缓存，而连接/加载/选中这类高频瞬态态永远读到最新值。
///
/// 各分支的 expanded/search 重算逻辑与 `flatten_sidebar_visible_rows` 逐行保持一致：
/// search 时数据库/分组/文件夹强制展开（`tree_expanded_for_search`），非 search 时读存储值。
#[allow(clippy::too_many_arguments)]
fn build_sidebar_row(
    row: &SidebarVisibleRow,
    this: &NavicatMain,
    state: &AppState,
    colors: UiColors,
    search_query: &str,
    rename_group_input: Entity<InputState>,
    table_folder_rename_input: Entity<InputState>,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let search_active = !search_query.is_empty();
    let search_opt = search_active.then_some(search_query);
    let active_object_path = state.active_tab().and_then(|tab| match &tab.kind {
        TabKind::DataEditor(editor) => Some(&editor.object),
        _ => None,
    });
    // 目标连接：几乎所有行都需要其 `ConnectionState`（自身 / 派生 / 颜色）。
    let connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == row.connection_id);

    match row.kind {
        SidebarRowKind::Connection => {
            let connection = connection.expect("connection row always has a connection");
            let connecting = this.connecting_connections.contains(&connection.config.id);
            connection_tree(connection, connecting, row.indent, row.group_id, colors, cx)
                .into_any_element()
        }
        SidebarRowKind::Group => {
            let group_id = row.group_id.expect("group row has group id");
            let group = state
                .sidebar_layout
                .groups
                .iter()
                .find(|group| group.id == group_id)
                .expect("group exists in layout");
            let renaming = this
                .pending_rename_group
                .as_ref()
                .is_some_and(|pending| pending.group_id == group_id);
            group_tree(
                group.id,
                &group.name,
                group.collapsed,
                renaming,
                rename_group_input,
                colors,
                cx,
            )
            .into_any_element()
        }
        SidebarRowKind::Database => {
            let database_path = row.database_path.clone().expect("database row has path");
            let database_name = database_path
                .database
                .clone()
                .unwrap_or_else(|| database_path.name.clone());
            let database_key = database_tree_key(row.connection_id, &database_name);
            let database_expanded = if database_path.kind == ObjectKind::RedisDb {
                false
            } else {
                tree_expanded_for_search(
                    this.expanded_databases.get(&database_key).copied(),
                    search_active,
                )
            };
            let database_loading = this.loading_databases.contains(&database_key);
            let database_loaded = this.loaded_database_children.contains(&database_key)
                || connection
                    .is_some_and(|connection| {
                        connection_has_loaded_children(connection, &database_name)
                    });
            let database_pinned = this.pinned_databases.contains(&database_key);
            let database_selected = active_object_path.is_some_and(|path| {
                path.connection_id == row.connection_id
                    && object_path_database_name(path) == database_name
            });
            let connection_color_hex = connection
                .map(|connection| connection_config_color_hex(&connection.config.options))
                .unwrap_or("#9ca3af");
            database_tree(
                row.connection_id,
                database_path,
                row.indent,
                database_expanded,
                database_loading,
                database_loaded,
                database_selected,
                database_pinned,
                connection_color_hex,
                colors,
                cx,
            )
            .into_any_element()
        }
        SidebarRowKind::ObjectGroup => {
            let database_name = row.database_name.clone().expect("object group has db name");
            let group = row.object_group.expect("object group row has group");
            let group_key =
                object_group_tree_key(row.connection_id, &database_name, group);
            let group_expanded = if search_active {
                tree_expanded_for_search(
                    this.expanded_object_groups.get(&group_key).copied(),
                    true,
                )
            } else {
                this.expanded_object_groups.get(&group_key).copied().unwrap_or(false)
            };
            let database_path = row.database_path.clone().expect("object group has path");
            object_group_tree(
                row.connection_id,
                database_path,
                database_name,
                group,
                row.indent,
                group_expanded,
                colors,
                cx,
            )
            .into_any_element()
        }
        SidebarRowKind::Table => {
            let object = row.object.clone().expect("table row has object");
            let pinned =
                this.pinned_tables.contains(&table_tree_key(&object.path));
            let connection_color_hex = connection
                .map(|connection| connection_config_color_hex(&connection.config.options))
                .unwrap_or("#9ca3af");
            table_tree(
                &object,
                row.indent,
                active_object_path,
                connection_color_hex,
                pinned,
                search_opt,
                colors,
                cx,
            )
            .into_any_element()
        }
        SidebarRowKind::SavedQuery => {
            let query = row.query.clone().expect("saved query row has query");
            saved_query_tree(&query, row.indent, search_opt, colors, cx).into_any_element()
        }
        SidebarRowKind::TableFolder => {
            let parent_key = row
                .folder_parent_key
                .clone()
                .expect("folder row has parent key");
            let folder = row.folder_name.clone().expect("folder row has name");
            // 文件夹默认展开（正常模式读取失败视为 true），与旧整树构建镜像。
            let folder_key = table_folder_tree_key(&parent_key, &folder);
            let folder_expanded = if search_active {
                tree_expanded_for_search(this.expanded_object_groups.get(&folder_key).copied(), true)
            } else {
                this.expanded_object_groups.get(&folder_key).copied().unwrap_or(true)
            };
            table_folder_tree(
                parent_key,
                &folder,
                row.indent,
                folder_expanded,
                this.selected_table_folder.as_ref(),
                this.pending_rename_table_folder.as_ref(),
                table_folder_rename_input,
                search_opt,
                colors,
                cx,
            )
            .into_any_element()
        }
    }
}

#[cfg(test)]
mod sidebar_flatten_tests {
    use super::*;

    fn database_option(kind: ObjectKind, name: &str) -> ObjectSummary {
        ObjectSummary {
            path: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some(name.to_string()),
                schema: None,
                name: name.to_string(),
                kind,
            },
            rows: None,
            modified_at: None,
            comment: None,
        }
    }

    fn table_option(connection: ConnectionId, db: &str, name: &str) -> ObjectSummary {
        ObjectSummary {
            path: ObjectPath {
                connection_id: connection,
                database: Some(db.to_string()),
                schema: None,
                name: name.to_string(),
                kind: ObjectKind::Table,
            },
            rows: None,
            modified_at: None,
            comment: None,
        }
    }

    fn connection_state(connection: &ConnectionConfig, expanded: bool) -> ConnectionState {
        ConnectionState {
            config: connection.clone(),
            connected: true,
            expanded,
            objects: Vec::new(),
            redis_overview: RedisConnectionOverview::default(),
        }
    }

    fn layout_with(entries: Vec<SidebarOrderEntry>) -> SidebarLayout {
        SidebarLayout {
            groups: Vec::new(),
            order: entries,
            table_folders: BTreeMap::new(),
            table_folder_assignments: BTreeMap::new(),
        }
    }

    fn kinds(rows: &[SidebarVisibleRow]) -> Vec<SidebarRowKind> {
        rows.iter().map(|row| row.kind).collect()
    }
    fn keys(rows: &[SidebarVisibleRow]) -> Vec<String> {
        rows.iter().map(|row| row.key.clone()).collect()
    }

    #[test]
    fn collapsed_connection_emits_only_self() {
        let conn = ConnectionConfig {
            id: ConnectionId(1),
            name: "c1".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "h".into(),
                port: 0,
                database: None,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        };
        let mut state = connection_state(&conn, false);
        state.objects.push(table_option(ConnectionId(1), "db1", "t1"));
        let rows = flatten_sidebar_visible_rows(
            &[state],
            &layout_with(vec![SidebarOrderEntry::Connection { id: ConnectionId(1) }]),
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            "",
        );
        // 连接折叠：只出连接行，不下钻任何对象。
        assert_eq!(kinds(&rows), vec![SidebarRowKind::Connection]);
        assert_eq!(rows[0].height(), 30.);
    }

    #[test]
    fn expanded_connection_lists_database_and_unassigned_table() {
        let conn = ConnectionConfig {
            id: ConnectionId(1),
            name: "c1".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "h".into(),
                port: 0,
                database: None,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        };
        let mut state = connection_state(&conn, true);
        state.objects.push(database_option(ObjectKind::Database, "db1"));
        state
            .objects
            .push(table_option(ConnectionId(1), "db1", "orders"));
        let mut layout = layout_with(vec![SidebarOrderEntry::Connection {
            id: ConnectionId(1),
        }]);
        layout.table_folders.insert(
            format!("{}:db1:tables", 1u64),
            vec!["归档".to_string()],
        );
        // 连接展开 → 数据库行。db 展开才有分组、分组展开才有表；显式展开 db1 与表分组。
        let expanded_databases: BTreeMap<String, bool> =
            BTreeMap::from([(database_tree_key(ConnectionId(1), "db1"), true)]);
        let mut expanded_object_groups: BTreeMap<String, bool> = BTreeMap::new();
        expanded_object_groups.insert(
            object_group_tree_key(ConnectionId(1), "db1", ObjectGroup::Tables),
            true,
        );
        let rows = flatten_sidebar_visible_rows(
            &[state.clone()],
            &layout,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &layout.table_folders.clone(),
            &layout.table_folder_assignments.clone(),
            &expanded_databases,
            &expanded_object_groups,
            "",
        );
        let k = kinds(&rows);
        assert_eq!(k[0], SidebarRowKind::Connection);
        assert_eq!(k[1], SidebarRowKind::Database);
        assert_eq!(rows[1].indent, 1);
        // 6 个分组 + 表行；至少包含一个 ObjectGroup。
        let group_count = k
            .iter()
            .filter(|kind| **kind == SidebarRowKind::ObjectGroup)
            .count();
        assert_eq!(group_count, 6);
        assert!(k.contains(&SidebarRowKind::Table));
        // 有归档文件夹默认展开，应出现 TableFolder 行。
        assert!(k.contains(&SidebarRowKind::TableFolder));
        // 表行缩进为分组内一级(分组 indent2，表 indent3)。
        let table_row_ix = k.iter().position(|kind| *kind == SidebarRowKind::Table).unwrap();
        assert_eq!(rows[table_row_ix].indent, 3);
        // 稳定 key 唯一。
        let ks = keys(&rows);
        let mut uniq = ks.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), ks.len());
    }

    #[test]
    fn redis_db_has_no_object_groups() {
        let conn = ConnectionConfig {
            id: ConnectionId(1),
            name: "r1".into(),
            kind: DatabaseKind::Redis,
            endpoint: Endpoint::Tcp {
                host: "h".into(),
                port: 6379,
                database: Some("0".into()),
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        };
        let mut state = connection_state(&conn, true);
        state
            .objects
            .push(database_option(ObjectKind::RedisDb, "0"));
        let rows = flatten_sidebar_visible_rows(
            &[state],
            &layout_with(vec![SidebarOrderEntry::Connection { id: ConnectionId(1) }]),
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            "",
        );
        // Redis db 行不展开任何对象分组。
        assert_eq!(
            kinds(&rows),
            vec![SidebarRowKind::Connection, SidebarRowKind::Database]
        );
    }

    #[test]
    fn search_forces_expansion_and_filters() {
        let conn = ConnectionConfig {
            id: ConnectionId(1),
            name: "c1".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "h".into(),
                port: 0,
                database: None,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        };
        let mut state = connection_state(&conn, true);
        state
            .objects
            .push(database_option(ObjectKind::Database, "db1"));
        state
            .objects
            .push(table_option(ConnectionId(1), "db1", "orders"));
        state
            .objects
            .push(table_option(ConnectionId(1), "db1", "customers"));
        let rows = flatten_sidebar_visible_rows(
            &[state],
            &layout_with(vec![SidebarOrderEntry::Connection { id: ConnectionId(1) }]),
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            "orders",
        );
        // search：只保留匹配 "orders" 的表，customers 被过滤。
        let tables: Vec<&String> = rows
            .iter()
            .filter(|row| row.kind == SidebarRowKind::Table)
            .filter_map(|row| row.object.as_ref().map(|o| &o.path.name))
            .collect();
        assert_eq!(tables, vec!["orders"]);
    }
}
