/// 跨连接表数据写入器。
///
/// 只负责把已经读出的行按字段名映射后分批写入目标表；读取源表、进度与任务编排
/// 留在应用层。构造时一次性读取目标列和 DDL，避免每批重复查元数据。
pub struct TableRowWriter {
    kind: DatabaseKind,
    connection_id: ConnectionId,
    /// 保留完整连接配置：表级事务需要按方言重新建立并持有长连接。
    config: ConnectionConfig,
    database: Option<String>,
    schema: Option<String>,
    connector: Box<dyn fluxdb_core::Connector>,
    target_columns: Vec<CompletionColumn>,
    /// `(源列下标, 目标列下标)`；源列下标对应当前页的行值顺序。
    selected: Vec<(usize, usize)>,
    column_list: String,
    overriding: &'static str,
    table: String,
    /// 唯一键冲突策略，默认按普通写入失败处理。
    conflict: fluxdb_core::InsertConflictMode,
    /// 表级事务会话；`None` 表示按批独立提交（默认）。
    ///
    /// 写入器在同步线程内单线程使用，不需要跨线程共享。
    session: std::cell::RefCell<Option<TableWriteSession>>,
}

impl TableRowWriter {
    /// `source_columns` 为源页的列名顺序。目标表缺失或没有任何同名可写列时直接报错，
    /// 避免空 SELECT * 把“没传输任何行”伪装成成功。
    pub fn new(
        config: &ConnectionConfig,
        object: &ObjectPath,
        source_columns: &[String],
    ) -> fluxdb_core::Result<Self> {
        Self::new_with_skips(config, object, source_columns, &[])
    }

    /// `skip_columns` 按目标字段名（忽略大小写）排除，不参与写入；
    /// 字段位置仍与传入的 `source_columns` 对齐。
    pub fn new_with_skips(
        config: &ConnectionConfig,
        object: &ObjectPath,
        source_columns: &[String],
        skip_columns: &[String],
    ) -> fluxdb_core::Result<Self> {
        if matches!(config.kind, DatabaseKind::Redis | DatabaseKind::MongoDb) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "当前连接类型不支持表数据传输",
            ));
        }
        let connector = connector_for(config)?;
        let target_columns = connector.list_completion_columns(
            object.database.as_deref(),
            object.schema.as_deref(),
            &object.name,
        )?;
        if target_columns.is_empty() {
            return Err(Error::new(
                ErrorKind::Query,
                "目标表不存在或没有可写入字段",
            ));
        }
        let selected = transfer_map_source_columns(source_columns, &target_columns);
        let ddl = connector.table_ddl(object).ok();
        let (excluded, identities) = ddl_import_column_flags(config.kind, ddl.as_deref());
        let selected = selected
            .into_iter()
            .filter(|(_, target_index)| {
                let name = &target_columns[*target_index].name;
                !excluded.contains(&name.to_ascii_uppercase())
                    && !skip_columns
                        .iter()
                        .any(|skipped| skipped.eq_ignore_ascii_case(name))
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(Error::new(ErrorKind::Query, "源表与目标表没有同名可写字段"));
        }
        let column_list = selected
            .iter()
            .map(|(_, target_index)| {
                import_quote_identifier(config.kind, &target_columns[*target_index].name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let overriding = if config.kind == DatabaseKind::Postgres
            && selected.iter().any(|(_, target_index)| {
                identities.contains(&target_columns[*target_index].name.to_ascii_uppercase())
            })
        {
            " OVERRIDING SYSTEM VALUE"
        } else {
            ""
        };
        Ok(Self {
            kind: config.kind,
            connection_id: config.id,
            config: config.clone(),
            database: object.database.clone(),
            schema: object.schema.clone(),
            connector,
            target_columns,
            selected,
            column_list,
            overriding,
            table: transfer_qualified_table(config, object)?,
            conflict: fluxdb_core::InsertConflictMode::Error,
            session: std::cell::RefCell::new(None),
        })
    }

    /// 设置唯一键冲突策略；仅 MySQL / PostgreSQL / SQLite 支持非默认值。
    pub fn with_conflict(mut self, conflict: fluxdb_core::InsertConflictMode) -> Self {
        self.conflict = conflict;
        self
    }

    /// 开启表级事务：清空目标表与全部 INSERT 在同一连接、同一事务内执行。
    ///
    /// 由调用方在写入结束后调用 [`Self::commit_atomic`]；中途错误或提前返回时
    /// 会话 Drop 会回滚，目标表保持写入前状态。
    pub fn begin_atomic(&self) -> fluxdb_core::Result<()> {
        let session = TableWriteSession::open(
            &self.config,
            self.database.as_deref(),
            self.schema.as_deref(),
        )?;
        *self.session.borrow_mut() = Some(session);
        Ok(())
    }

    /// 提交表级事务。未开启事务时是空操作。
    pub fn commit_atomic(&self) -> fluxdb_core::Result<()> {
        let session = self.session.borrow_mut().take();
        match session {
            Some(session) => session.commit(),
            None => Ok(()),
        }
    }

    /// 覆盖模式清空目标表。SQLite 没有 TRUNCATE，用 DELETE 保持语句可执行。
    ///
    /// 表级事务内必须用 DELETE：TRUNCATE 在多数方言里无法回滚。
    pub fn truncate(&self, cancel: &AtomicBool) -> fluxdb_core::Result<()> {
        transfer_check_cancel(cancel)?;
        let mut session = self.session.borrow_mut();
        if let Some(session) = session.as_mut() {
            session.execute(&format!("DELETE FROM {}", self.table), &[])?;
            return Ok(());
        }
        drop(session);
        let sql = match self.kind {
            DatabaseKind::Sqlite => format!("DELETE FROM {}", self.table),
            _ => format!("TRUNCATE TABLE {}", self.table),
        };
        self.execute(sql)?;
        Ok(())
    }

    /// 分批写入一页行。`progress` 每批回调 `(累计已写行数, 本批行数)`。
    pub fn write_rows(
        &self,
        rows: &[Vec<CellValue>],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, usize),
    ) -> fluxdb_core::Result<u64> {
        let mut written = 0u64;
        let mut session = self.session.borrow_mut();
        for chunk in rows.chunks(TABLE_IMPORT_BATCH_ROWS) {
            transfer_check_cancel(cancel)?;
            let (sql, parameters) = build_import_batch_sql(
                self.kind,
                &self.table,
                &self.column_list,
                self.overriding,
                &self.selected,
                &self.target_columns,
                chunk,
                self.conflict,
            )?;
            let affected = match session.as_mut() {
                Some(session) => session.execute(&sql, &parameters)?,
                None => self.execute_parameterized(sql, parameters)?,
            };
            // 忽略冲突时按实际写入行数计；其余模式按提交行数计（MySQL upsert
            // 会把更新行计为 2，不能直接当插入数）。
            written += if self.conflict == fluxdb_core::InsertConflictMode::Ignore {
                affected.min(chunk.len() as u64)
            } else {
                chunk.len() as u64
            };
            progress(written, chunk.len());
        }
        Ok(written)
    }

    /// 把一批行渲染成逐行 INSERT 字面量语句，用于「导出迁移 SQL」。
    ///
    /// 与执行路径共用列映射、冲突模式和表名限定；字面量按目标方言渲染，
    /// 不含任何绑定占位符。
    pub fn render_insert_scripts(
        &self,
        rows: &[Vec<CellValue>],
    ) -> fluxdb_core::Result<Vec<String>> {
        let conflict = import_conflict_clause(
            self.kind,
            &self.selected,
            &self.target_columns,
            self.conflict,
        )?;
        rows.iter()
            .map(|row| {
                let values = self
                    .selected
                    .iter()
                    .map(|(source_index, _)| {
                        let value = row.get(*source_index).unwrap_or(&CellValue::Null);
                        script_literal(self.kind, value)
                    })
                    .collect::<fluxdb_core::Result<Vec<_>>>()?
                    .join(", ");
                Ok(format!(
                    "{} {} ({}){} VALUES ({}){};",
                    conflict.prefix,
                    self.table,
                    self.column_list,
                    self.overriding,
                    values,
                    conflict.suffix
                ))
            })
            .collect()
    }

    /// 覆盖模式的清空语句（表级事务内同样使用 DELETE）。
    pub fn render_clear_script(&self) -> String {
        format!("DELETE FROM {};", self.table)
    }

    /// 渲染一条 UPDATE 字面量语句；列名与值都按目标方言生成。
    pub fn render_update_script(&self, update: &RowUpdate) -> fluxdb_core::Result<String> {
        script_update_statement(self.kind, &self.table, update)
    }

    /// 渲染一条 DELETE 字面量语句。
    pub fn render_delete_script(&self, identity: &RowIdentity) -> fluxdb_core::Result<String> {
        script_delete_statement(self.kind, &self.table, identity)
    }

    fn execute(&self, text: String) -> fluxdb_core::Result<()> {
        let result = self.connector.execute(&QueryRequest {
            connection_id: self.connection_id,
            database: self.database.clone(),
            schema: self.schema.clone(),
            session_id: None,
            text,
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })?;
        transfer_ensure_success(&result)
    }

    fn execute_parameterized(
        &self,
        text: String,
        parameters: Vec<fluxdb_core::SqlParameter>,
    ) -> fluxdb_core::Result<u64> {
        let result = self
            .connector
            .execute_parameterized(&ParameterizedQueryRequest {
                request: QueryRequest {
                    connection_id: self.connection_id,
                    database: self.database.clone(),
                    schema: self.schema.clone(),
                    session_id: None,
                    text,
                    mode: QueryMode::All,
                    options: QueryExecutionOptions::default(),
                },
                parameters,
            })?;
        transfer_ensure_success(&result)?;
        Ok(result
            .summaries
            .iter()
            .map(|summary| summary.affected_rows)
            .sum())
    }
}

/// 按字段名（忽略大小写）建立源→目标列映射；目标表不存在的列会被忽略，
/// 源表多出的列不写入，避免跨库字段顺序不同导致错列。
fn transfer_map_source_columns(
    source_columns: &[String],
    target_columns: &[CompletionColumn],
) -> Vec<(usize, usize)> {
    source_columns
        .iter()
        .enumerate()
        .filter_map(|(source_index, name)| {
            target_columns
                .iter()
                .position(|target| target.name.eq_ignore_ascii_case(name))
                .map(|target_index| (source_index, target_index))
        })
        .collect()
}

fn transfer_qualified_table(
    config: &ConnectionConfig,
    object: &ObjectPath,
) -> fluxdb_core::Result<String> {
    let dialect = fluxdb_core::sql_dialect(config.kind);
    let schema = object
        .schema
        .as_deref()
        .map(str::trim)
        .filter(|schema| !schema.is_empty());
    let schema = schema.or(match config.kind {
        DatabaseKind::Postgres | DatabaseKind::Kingbase => Some("public"),
        DatabaseKind::SqlServer => Some("dbo"),
        _ => None,
    });
    let name = dialect.quote_identifier(&object.name);
    match (schema, config.kind) {
        (Some(schema), DatabaseKind::Postgres | DatabaseKind::Kingbase)
        | (Some(schema), DatabaseKind::SqlServer)
        | (Some(schema), DatabaseKind::Dameng)
        | (Some(schema), DatabaseKind::Oracle) => {
            Ok(format!("{}.{}", dialect.quote_identifier(schema), name))
        }
        _ => Ok(name),
    }
}

fn transfer_ensure_success(result: &fluxdb_core::QueryExecutionResult) -> fluxdb_core::Result<()> {
    match result.summaries.iter().find(|summary| !summary.success) {
        Some(summary) => Err(Error::new(ErrorKind::Query, summary.message.clone())),
        None => Ok(()),
    }
}

fn transfer_check_cancel(cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::new(ErrorKind::Cancelled, "传输已取消"));
    }
    Ok(())
}

#[cfg(test)]
mod table_transfer_tests {
    use super::*;

    fn column(name: &str) -> CompletionColumn {
        CompletionColumn {
            database: None,
            schema: None,
            table: "t".into(),
            name: name.into(),
            type_name: Some("int".into()),
            nullable: true,
            primary_key: false,
            comment: None,
            stable: None,
        }
    }

    #[test]
    fn maps_columns_by_name_ignoring_case_and_order() {
        let source = vec!["ID".to_string(), "name".to_string(), "extra".to_string()];
        let target = vec![column("name"), column("id")];
        assert_eq!(transfer_map_source_columns(&source, &target), vec![(0, 1), (1, 0)]);
    }
}
