/// 跨连接表数据写入器。
///
/// 只负责把已经读出的行按字段名映射后分批写入目标表；读取源表、进度与任务编排
/// 留在应用层。构造时一次性读取目标列和 DDL，避免每批重复查元数据。
pub struct TableRowWriter {
    kind: DatabaseKind,
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    connector: Box<dyn fluxdb_core::Connector>,
    target_columns: Vec<CompletionColumn>,
    /// `(源列下标, 目标列下标)`；源列下标对应当前页的行值顺序。
    selected: Vec<(usize, usize)>,
    column_list: String,
    overriding: &'static str,
    table: String,
}

impl TableRowWriter {
    /// `source_columns` 为源页的列名顺序。目标表缺失或没有任何同名可写列时直接报错，
    /// 避免空 SELECT * 把“没传输任何行”伪装成成功。
    pub fn new(
        config: &ConnectionConfig,
        object: &ObjectPath,
        source_columns: &[String],
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
                !excluded.contains(&target_columns[*target_index].name.to_ascii_uppercase())
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
            database: object.database.clone(),
            schema: object.schema.clone(),
            connector,
            target_columns,
            selected,
            column_list,
            overriding,
            table: transfer_qualified_table(config, object)?,
        })
    }

    /// 覆盖模式清空目标表。SQLite 没有 TRUNCATE，用 DELETE 保持语句可执行。
    pub fn truncate(&self, cancel: &AtomicBool) -> fluxdb_core::Result<()> {
        transfer_check_cancel(cancel)?;
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
            )?;
            self.execute_parameterized(sql, parameters)?;
            written += chunk.len() as u64;
            progress(written, chunk.len());
        }
        Ok(written)
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
    ) -> fluxdb_core::Result<()> {
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
        transfer_ensure_success(&result)
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
