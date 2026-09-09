// Included in crate-root scope by ../lib.rs; grouped by SQL completion index responsibility.

const COMPLETION_INDEX_TTL_SECONDS: u64 = 30 * 60;
const COMPLETION_PREFIX_LIMIT: usize = 24;
const COMPLETION_FUZZY_SCAN_LIMIT: usize = 20_000;
const COMPLETION_RESULT_LIMIT: usize = 100;
const COMPLETION_WARMUP_BATCH_SIZE: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct DbKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct TableKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    table: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct PrefixKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    prefix: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct TableId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct ColumnId(usize);

#[derive(Clone, Debug)]
struct IndexedColumnRef {
    source: ColumnRef,
    lower_column: String,
    column_tokens: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct CompletionIndex {
    tables_by_db: BTreeMap<DbKey, Vec<TableId>>,
    columns_by_table: BTreeMap<TableKey, Vec<ColumnId>>,
    columns_by_db: BTreeMap<DbKey, Vec<ColumnId>>,
    column_prefix_index: BTreeMap<PrefixKey, Vec<ColumnId>>,
    tables: Vec<TableRef>,
    columns: Vec<IndexedColumnRef>,
    dirty_databases: BTreeSet<DbKey>,
    dirty_tables: BTreeSet<TableKey>,
    metas: BTreeMap<DbKey, CompletionIndexMeta>,
    /// T051：正在后台刷新的库，用于 stale-while-refresh 的去重，避免每次按键重复触发刷新线程。
    refresh_inflight: BTreeSet<DbKey>,
}

impl CompletionIndex {
    fn db_key(
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> DbKey {
        DbKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
        }
    }

    fn table_key(
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> TableKey {
        TableKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            table: table.to_ascii_lowercase(),
        }
    }

    fn insert_tables(
        &mut self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        tables: Vec<CompletionTable>,
        db_kind: DatabaseKind,
    ) {
        let db_key = Self::db_key(connection_id, database, schema);
        let mut table_ids = Vec::new();
        for table in tables {
            let source = TableRef {
                database: table.database.clone().or_else(|| database.map(str::to_string)),
                schema: table.schema.clone().or_else(|| schema.map(str::to_string)),
                name: table.name,
                kind: table.kind,
                rows: None,
                comment: None,
            };
            let table_id = TableId(self.tables.len());
            self.tables.push(source);
            table_ids.push(table_id);
        }
        self.tables_by_db.insert(db_key.clone(), table_ids);
        self.touch_meta(db_key, db_kind);
    }

    fn replace_table_columns(
        &mut self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        columns: Vec<CompletionColumn>,
        db_kind: DatabaseKind,
    ) {
        let table_key = Self::table_key(connection_id, database, schema, table);
        let db_key = Self::db_key(connection_id, database, schema);
        self.remove_table_column_ids(&table_key);

        let mut column_ids = Vec::new();
        for (index, column) in columns.into_iter().enumerate() {
            let source = ColumnRef {
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                table: column.table.clone(),
                column: column.name,
                type_name: column.type_name,
                nullable: column.nullable,
                primary_key: column.primary_key,
                ordinal_position: Some((index + 1) as u32),
                comment: column.comment,
            };
            let column_id = self.push_column(source);
            column_ids.push(column_id);
            self.columns_by_db
                .entry(db_key.clone())
                .or_default()
                .push(column_id);
            self.index_column_prefixes(&db_key, column_id);
        }

        self.columns_by_table.insert(table_key, column_ids);
        self.touch_meta(db_key, db_kind);
        self.dirty_tables
            .retain(|key| key != &Self::table_key(connection_id, database, schema, table));
    }

    fn insert_snapshot(&mut self, snapshot: CompletionIndexSnapshot) {
        if snapshot.meta.app_index_version != COMPLETION_INDEX_VERSION {
            return;
        }
        let db_key = Self::db_key(
            snapshot.connection_id,
            snapshot.database.as_deref(),
            snapshot.schema.as_deref(),
        );
        self.clear_database(&db_key);
        let table_ids = snapshot
            .tables
            .into_iter()
            .map(|table| {
                let table_id = TableId(self.tables.len());
                self.tables.push(table);
                table_id
            })
            .collect::<Vec<_>>();
        self.tables_by_db.insert(db_key.clone(), table_ids);
        for column in snapshot.columns {
            let table_key = Self::table_key(
                snapshot.connection_id,
                column.database.as_deref().or(snapshot.database.as_deref()),
                column.schema.as_deref().or(snapshot.schema.as_deref()),
                &column.table,
            );
            let column_id = self.push_column(column);
            self.columns_by_table
                .entry(table_key)
                .or_default()
                .push(column_id);
            self.columns_by_db
                .entry(db_key.clone())
                .or_default()
                .push(column_id);
            self.index_column_prefixes(&db_key, column_id);
        }
        self.metas.insert(db_key, snapshot.meta);
    }

    fn snapshot(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        db_kind: DatabaseKind,
    ) -> CompletionIndexSnapshot {
        let db_key = Self::db_key(connection_id, database, schema);
        let tables = self
            .tables_by_db
            .get(&db_key)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.tables.get(id.0).cloned())
            .collect::<Vec<_>>();
        let columns = self
            .columns_by_db
            .get(&db_key)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.columns.get(id.0).map(|column| column.source.clone()))
            .collect::<Vec<_>>();
        let mut meta = self
            .metas
            .get(&db_key)
            .cloned()
            .unwrap_or_else(|| empty_completion_meta(db_kind));
        meta.app_index_version = COMPLETION_INDEX_VERSION;
        meta.db_kind = db_kind;
        meta.table_count = tables.len();
        meta.table_fingerprints = table_fingerprints(&columns);
        CompletionIndexSnapshot {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            tables,
            columns,
            routines: Vec::new(),
            triggers: Vec::new(),
            meta,
        }
    }

    fn clear_database(&mut self, db_key: &DbKey) {
        self.tables_by_db.remove(db_key);
        self.columns_by_db.remove(db_key);
        self.metas.remove(db_key);
        self.dirty_databases.remove(db_key);
        self.columns_by_table.retain(|key, _| {
            key.connection_id != db_key.connection_id
                || key.database != db_key.database
                || key.schema != db_key.schema
        });
        self.dirty_tables.retain(|key| {
            key.connection_id != db_key.connection_id
                || key.database != db_key.database
                || key.schema != db_key.schema
        });
        self.rebuild_prefix_index();
    }

    fn mark_dirty(
        &mut self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) {
        let db_key = Self::db_key(connection_id, database, schema);
        self.dirty_databases.insert(db_key.clone());
        self.metas
            .entry(db_key)
            .and_modify(|meta| meta.dirty = true);
    }

    fn mark_table_dirty(
        &mut self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) {
        let db_key = Self::db_key(connection_id, database, schema);
        let table_key = Self::table_key(connection_id, database, schema, table);
        self.dirty_tables.insert(table_key);
        self.metas
            .entry(db_key)
            .and_modify(|meta| meta.dirty = true);
    }

    fn has_database_index(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> bool {
        let db_key = Self::db_key(connection_id, database, schema);
        self.tables_by_db.contains_key(&db_key) || self.columns_by_db.contains_key(&db_key)
    }

    fn is_database_dirty(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> bool {
        let db_key = Self::db_key(connection_id, database, schema);
        self.dirty_databases.contains(&db_key)
    }

    fn dirty_table_names(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Vec<String> {
        let db_key = Self::db_key(connection_id, database, schema);
        self.dirty_tables
            .iter()
            .filter(|key| {
                key.connection_id == db_key.connection_id
                    && key.database == db_key.database
                    && key.schema == db_key.schema
            })
            .map(|key| key.table.clone())
            .collect()
    }

    fn is_dirty_or_expired(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        now: u64,
    ) -> bool {
        let db_key = Self::db_key(connection_id, database, schema);
        if self.dirty_databases.contains(&db_key) {
            return true;
        }
        if self.dirty_tables.iter().any(|key| {
            key.connection_id == db_key.connection_id
                && key.database == db_key.database
                && key.schema == db_key.schema
        }) {
            return true;
        }
        let Some(meta) = self.metas.get(&db_key) else {
            return true;
        };
        meta.dirty || now.saturating_sub(meta.last_verified_at) >= meta.ttl_seconds
    }

    fn table_columns(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> Vec<CompletionColumn> {
        let table_key = Self::table_key(connection_id, database, schema, table);
        self.columns_by_table
            .get(&table_key)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.columns.get(id.0))
            .map(indexed_column_to_completion)
            .collect()
    }

    fn database_columns(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        prefix: &str,
    ) -> Vec<CompletionColumn> {
        let db_key = Self::db_key(connection_id, database, schema);
        let prefix = prefix.trim().to_ascii_lowercase();
        let mut seen = BTreeSet::new();
        let mut ids = Vec::new();
        if !prefix.is_empty() {
            let prefix_key = PrefixKey {
                connection_id,
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                prefix: prefix.clone(),
            };
            if let Some(prefix_ids) = self.column_prefix_index.get(&prefix_key) {
                ids.extend(prefix_ids.iter().copied());
            }
        }
        if ids.len() < COMPLETION_RESULT_LIMIT {
            if let Some(db_ids) = self.columns_by_db.get(&db_key) {
                for id in db_ids.iter().take(COMPLETION_FUZZY_SCAN_LIMIT) {
                    let Some(column) = self.columns.get(id.0) else {
                        continue;
                    };
                    if prefix.is_empty() || matches_completion_fuzzy(&column.source.column, &prefix) {
                        ids.push(*id);
                    }
                    if ids.len() >= COMPLETION_RESULT_LIMIT * 4 {
                        break;
                    }
                }
            }
        }

        ids.into_iter()
            .filter(|id| seen.insert(*id))
            .filter_map(|id| self.columns.get(id.0))
            .map(indexed_column_to_completion)
            .collect()
    }

    fn database_tables(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Vec<CompletionTable> {
        let db_key = Self::db_key(connection_id, database, schema);
        self.tables_by_db
            .get(&db_key)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.tables.get(id.0))
            .map(|table| CompletionTable {
                database: table.database.clone(),
                schema: table.schema.clone(),
                name: table.name.clone(),
                kind: table.kind,
            })
            .collect()
    }

    /// 收集该连接在索引中出现过的 (database, schema) 名称，用于骨架级 schema 补全（P1.5）。
    fn database_schemas(&self, connection_id: ConnectionId) -> Vec<(Option<String>, Option<String>)> {
        self.tables_by_db
            .keys()
            .filter(|key| key.connection_id == connection_id)
            .map(|key| (key.database.clone(), key.schema.clone()))
            .collect()
    }

    fn push_column(&mut self, source: ColumnRef) -> ColumnId {
        let lower_column = source.column.to_ascii_lowercase();
        let column_tokens = completion_tokens(&source.column);
        let indexed = IndexedColumnRef {
            source,
            lower_column,
            column_tokens,
        };
        let column_id = ColumnId(self.columns.len());
        self.columns.push(indexed);
        column_id
    }

    fn remove_table_column_ids(&mut self, table_key: &TableKey) {
        let Some(old_ids) = self.columns_by_table.remove(table_key) else {
            return;
        };
        let old_ids = old_ids.into_iter().collect::<BTreeSet<_>>();
        for ids in self.columns_by_db.values_mut() {
            ids.retain(|id| !old_ids.contains(id));
        }
        self.rebuild_prefix_index();
    }

    fn rebuild_prefix_index(&mut self) {
        self.column_prefix_index.clear();
        let entries = self
            .columns_by_db
            .iter()
            .flat_map(|(db_key, ids)| ids.iter().map(|id| (db_key.clone(), *id)))
            .collect::<Vec<_>>();
        for (db_key, id) in entries {
            self.index_column_prefixes(&db_key, id);
        }
    }

    fn index_column_prefixes(&mut self, db_key: &DbKey, column_id: ColumnId) {
        let Some(column) = self.columns.get(column_id.0) else {
            return;
        };
        let mut prefixes = BTreeSet::new();
        for token in std::iter::once(column.lower_column.as_str())
            .chain(column.column_tokens.iter().map(String::as_str))
        {
            for prefix in token_prefixes(token) {
                prefixes.insert(prefix);
            }
        }
        for prefix in prefixes {
            self.column_prefix_index
                .entry(PrefixKey {
                    connection_id: db_key.connection_id,
                    database: db_key.database.clone(),
                    schema: db_key.schema.clone(),
                    prefix,
                })
                .or_default()
                .push(column_id);
        }
    }

    fn touch_meta(&mut self, db_key: DbKey, db_kind: DatabaseKind) {
        let now = unix_timestamp_secs();
        let meta = self
            .metas
            .entry(db_key.clone())
            .or_insert_with(|| empty_completion_meta(db_kind));
        meta.app_index_version = COMPLETION_INDEX_VERSION;
        meta.db_kind = db_kind;
        meta.last_indexed_at = now;
        meta.last_verified_at = now;
        meta.dirty = false;
        self.dirty_databases.remove(&db_key);
    }

    /// T051：标记某库进入后台刷新。返回 false 表示已在刷新中（去重，不再触发新线程）。
    fn begin_refresh(&mut self, db_key: DbKey) -> bool {
        self.refresh_inflight.insert(db_key)
    }

    /// T051：后台刷新结束（无论成功或失败）后清除刷新标记。
    fn end_refresh(&mut self, db_key: &DbKey) {
        self.refresh_inflight.remove(db_key);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RankedCompletionItem {
    item: QueryCompletionItem,
    score: i32,
    rank: (u8, usize, usize, usize),
    source_table: String,
    /// 限定前缀（别名优先，其次表名）。P2.10 重复列消歧用。
    source_qualifier: String,
}

/// 重复列消歧（P2.10）：当同一列名出现在多个不同的引用表中时，把候选 label 与
/// apply 前缀改为「限定前缀.列名」（限定前缀取别名，其次表名）；唯一列保持裸列名。
/// 判定按「列名 × 出现过的不同表」计数：仅当跨表重复才加前缀，避免候选变长。
fn disambiguate_duplicate_columns(mut items: Vec<RankedCompletionItem>) -> Vec<RankedCompletionItem> {
    let mut column_owner_tables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for item in &items {
        column_owner_tables
            .entry(item.item.label.to_ascii_lowercase())
            .or_default()
            .insert(item.source_table.clone());
    }
    for item in &mut items {
        let owners = column_owner_tables
            .get(&item.item.label.to_ascii_lowercase())
            .map(|set| set.len())
            .unwrap_or(0);
        if owners <= 1 {
            continue;
        }
        let qualifier = if item.source_qualifier.is_empty() {
            item.source_table.clone()
        } else {
            item.source_qualifier.clone()
        };
        item.item.label = format!("{qualifier}.{}", item.item.label);
        item.item.insert_text = format!("{qualifier}.{}", item.item.insert_text);
    }
    items
}

fn rank_column_completion(
    column: CompletionColumn,
    prefix: &str,
    scope: CompletionColumnScope,
) -> RankedCompletionItem {
    let mut score = match completion_match_rank(&column.name, prefix).0 {
        0 => 0,
        1 => 10,
        2 => 30,
        3 => 40,
        _ => 100,
    };
    if completion_tokens(&column.name)
        .iter()
        .any(|token| token.starts_with(&prefix.to_ascii_lowercase()))
    {
        score = score.min(20);
    }
    match scope {
        CompletionColumnScope::AliasQualified => score -= 20,
        CompletionColumnScope::ReferencedTable => score -= 30,
        CompletionColumnScope::DatabaseWide => {}
    }
    if column.primary_key {
        score -= 5;
    }
    if column.name.eq_ignore_ascii_case("id") && !prefix.eq_ignore_ascii_case("id") {
        score += 10;
    }
    let detail = column_completion_detail_with_table(&column);
    RankedCompletionItem {
        rank: completion_match_rank(&column.name, prefix),
        source_table: column.table.to_ascii_lowercase(),
        source_qualifier: column.table.clone(),
        item: QueryCompletionItem {
            label: column.name.clone(),
            insert_text: column.name,
            kind: QueryCompletionKind::Column,
            detail,
            documentation: column.comment.clone(),
            filter_text: None,
            sort_text: None,
                    ..Default::default()
},
        score,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionColumnScope {
    DatabaseWide,
    ReferencedTable,
    AliasQualified,
}

fn sort_ranked_completion_items(mut items: Vec<RankedCompletionItem>) -> Vec<QueryCompletionItem> {
    items.sort_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| {
                left.item
                    .label
                    .to_ascii_lowercase()
                    .cmp(&right.item.label.to_ascii_lowercase())
            })
            .then_with(|| left.source_table.cmp(&right.source_table))
    });
    items
        .into_iter()
        .take(COMPLETION_RESULT_LIMIT)
        .map(|item| {
            let mut completion = item.item;
            // 以通用 sort_text 把 App 层来源分数带过 bridge；editor-core 不需要理解 SQL。
            completion.sort_text = Some(format!(
                "{:04}:{:01}:{:08}:{:08}",
                item.score.saturating_add(1000),
                item.rank.0,
                item.rank.1,
                item.rank.2
            ));
            completion
        })
        .collect()
}

fn completion_tokens(value: &str) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    let lower = value.to_ascii_lowercase();
    tokens.insert(lower.clone());
    for part in lower.split(|ch: char| ch == '_' || ch == '-' || ch.is_whitespace()) {
        if !part.is_empty() {
            tokens.insert(part.to_string());
        }
    }
    for part in camel_case_tokens(value) {
        tokens.insert(part);
    }
    tokens.into_iter().collect()
}

fn camel_case_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if ch.is_ascii_uppercase() && !current.is_empty() {
            tokens.push(current.to_ascii_lowercase());
            current.clear();
        }
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(current.to_ascii_lowercase());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(current.to_ascii_lowercase());
    }
    tokens
}

fn token_prefixes(token: &str) -> Vec<String> {
    let token = token.trim().to_ascii_lowercase();
    let chars = token.chars().collect::<Vec<_>>();
    let max = chars.len().min(COMPLETION_PREFIX_LIMIT);
    (1..=max)
        .map(|len| chars.iter().take(len).collect::<String>())
        .collect()
}

fn indexed_column_to_completion(column: &IndexedColumnRef) -> CompletionColumn {
    CompletionColumn {
        table: column.source.table.clone(),
        name: column.source.column.clone(),
        type_name: column.source.type_name.clone(),
        nullable: column.source.nullable,
        primary_key: column.source.primary_key,
        comment: column.source.comment.clone(),
    }
}

fn column_completion_detail_with_table(column: &CompletionColumn) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(type_name) = column.type_name.as_ref().filter(|value| !value.is_empty()) {
        parts.push(type_name.clone());
    }
    if column.primary_key {
        parts.push("PK".to_string());
    }
    parts.push(column.table.clone());
    Some(parts.join(" · "))
}

fn empty_completion_meta(db_kind: DatabaseKind) -> CompletionIndexMeta {
    CompletionIndexMeta {
        app_index_version: COMPLETION_INDEX_VERSION,
        db_kind,
        last_indexed_at: 0,
        last_verified_at: 0,
        ttl_seconds: COMPLETION_INDEX_TTL_SECONDS,
        dirty: false,
        table_count: 0,
        table_fingerprints: Vec::new(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletionSnapshotSignature {
    table_count: usize,
    column_count: usize,
    fingerprints: Vec<TableFingerprint>,
}

impl CompletionSnapshotSignature {
    fn from_snapshot(snapshot: &CompletionIndexSnapshot) -> Self {
        Self {
            table_count: snapshot.tables.len(),
            column_count: snapshot.columns.len(),
            fingerprints: table_fingerprints(&snapshot.columns),
        }
    }
}

fn table_fingerprints(columns: &[ColumnRef]) -> Vec<TableFingerprint> {
    let mut grouped: BTreeMap<(Option<String>, Option<String>, String), Vec<&ColumnRef>> =
        BTreeMap::new();
    for column in columns {
        grouped
            .entry((
                column.database.clone(),
                column.schema.clone(),
                column.table.clone(),
            ))
            .or_default()
            .push(column);
    }
    grouped
        .into_iter()
        .map(|((database, schema, table), columns)| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            table.hash(&mut hasher);
            for column in columns {
                column.column.hash(&mut hasher);
                column.type_name.hash(&mut hasher);
                column.nullable.hash(&mut hasher);
                column.primary_key.hash(&mut hasher);
                column.ordinal_position.hash(&mut hasher);
                column.comment.hash(&mut hasher);
            }
            TableFingerprint {
                database,
                schema,
                table,
                fingerprint: hasher.finish(),
            }
        })
        .collect()
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
