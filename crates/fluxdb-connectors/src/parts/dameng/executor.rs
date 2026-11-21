fn dm_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    dm_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

fn dm_execute_query_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = query_statements_for_execution(request, config.kind);
    if statements.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }
    let mut client = dm_connect(config)?;
    let pagination = query_execution_pagination(request.options.page_offset, request.options.page_size);
    // 结果里的超大字段按本次执行归属外置：重新执行/关标签时整批释放。
    let content_owner =
        fluxdb_core::result_content_owner(request.session_id, request.connection_id);
    let mut execution = QueryExecutionResult {
        summaries: Vec::new(),
        results: Vec::new(),
        rollback_snapshots: Vec::new(),
    };
    // 只读入口：达梦的 `SET TRANSACTION READ ONLY` 只影响后续事务，单独发送不会
    // 覆盖自动提交语句。这里用驱动事务关掉自动提交，执行后无条件回滚；
    // 策略层漏掉的写语句也不会落库。
    let read_only_wrap = request.options.read_only;
    if read_only_wrap {
        client.begin().map_err(|error| dm_error("开启只读事务失败", error))?;
    }
    for statement in statements {
        if should_cancel() {
            push_query_summary(
                &mut execution,
                failed_query_summary(
                    statement,
                    QueryStatementKind::Command,
                    "已取消：未执行".into(),
                    0,
                ),
                on_summary,
            );
            continue;
        }
        let started = std::time::Instant::now();
        match dm_execute(&mut client, &statement, &[]) {
            Ok(ExecuteResult::Query { columns, rows }) => {
                let columns = columns
                    .iter()
                    .map(|column| dm_column(column, false))
                    .collect::<Vec<_>>();
                let returned_rows = rows.len() as u64;
                let page = dm_collect_page(
                    columns,
                    rows.into_iter(),
                    pagination,
                    pagination.offset,
                    &content_owner,
                )?;
                let message = if let Some(incomplete) = &page.incomplete {
                    format!(
                        "已读取 {returned_rows} 行，当前仅展示 {} 行（{}）",
                        page.rows.len(),
                        incomplete.reason
                    )
                } else if page.has_more {
                    format!("返回 {returned_rows} 行，当前仅展示 {} 行（结果已截断）", page.rows.len())
                } else {
                    format!("返回 {returned_rows} 行")
                };
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::ResultSet,
                        success: true,
                        message,
                        returned_rows,
                        affected_rows: 0,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
                execution.results.push(page);
            }
            Ok(ExecuteResult::Affected(affected)) | Ok(ExecuteResult::Call { affected, .. }) => {
                // 只读入口出现无结果集的命令，说明策略层漏掉了写语句；
                // 事务随后会回滚，必须明确报“未提交”，不能声称执行成功。
                let (success, message) = if read_only_wrap {
                    (
                        false,
                        format!("只读入口拒绝：该语句不是只读查询，已回滚（影响 {affected} 行未提交）"),
                    )
                } else {
                    (true, format!("OK，影响 {affected} 行"))
                };
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::Command,
                        success,
                        message,
                        returned_rows: 0,
                        affected_rows: affected,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
            }
            Err(error) => {
                tracing::warn!(target: "fluxdb_connectors", message = %error.message, "达梦执行失败");
                push_query_summary(
                    &mut execution,
                    failed_query_summary(
                        statement,
                        QueryStatementKind::Command,
                        error.message.clone(),
                        elapsed_ms(started),
                    ),
                    on_summary,
                );
                if !request.options.continue_on_error {
                    break;
                }
            }
        }
    }
    if read_only_wrap {
        // 回滚失败不能把「已回滚」说成事实：连接随后丢弃，未提交的更改由服务端
        // 在会话结束时回滚，但这次调用不能返回成功。
        if let Err(error) = client.rollback() {
            client.close();
            tracing::warn!(
                target: "fluxdb_connectors",
                message = %error,
                "达梦只读事务回滚失败"
            );
            return Err(Error::new(
                ErrorKind::Query,
                "达梦只读事务回滚失败，本次结果未确认",
            ));
        }
    }
    client.close();
    Ok(execution)
}

/// 逐行收页：单格超阈值先外置成内容引用，页容量只由行数决定。
///
/// `skip_rows` 是 SQL 未做 OFFSET 时需要在内存里跳过的行数（已带 OFFSET 时传 0）。
///
/// ponytail: 达梦驱动（`DmClient::execute`）在返回前就把整个结果集物化，
/// 所以这里的预算只能约束「结果页留下多少」，管不住驱动那一次物化。
/// 要做真流式得改用 `execute_lazy`（当前只对参数化查询开放），
/// 需要可达的 DM8 实例验证协议后再改。
fn dm_collect_page(
    columns: Vec<Column>,
    rows: impl Iterator<Item = Vec<Value>>,
    pagination: Pagination,
    skip_rows: u64,
    owner: &str,
) -> fluxdb_core::Result<DataPage> {
    dm_collect_page_with_digests(columns, rows, pagination, skip_rows, owner, &[])
}

/// `digest_columns` 里的列在落页前替换为 SHA-256 摘要，避免大字段正文进结果页。
fn dm_collect_page_with_digests(
    columns: Vec<Column>,
    rows: impl Iterator<Item = Vec<Value>>,
    pagination: Pagination,
    skip_rows: u64,
    owner: &str,
    digest_columns: &[String],
) -> fluxdb_core::Result<DataPage> {
    let width = columns.len();
    let capacity = query_execution_row_capacity(pagination.limit);
    let mut page = DataPage {
        columns,
        rows: Vec::new(),
        offset: pagination.offset,
        limit: pagination.limit,
        has_more: false,
        incomplete: None,
    };
    for (index, row) in rows.enumerate() {
        if (index as u64) < skip_rows {
            continue;
        }
        if page.rows.len() >= capacity {
            // 读满页容量：后面还有行，这是「正常还有更多」，不是资源截断。
            page.has_more = true;
            break;
        }
        let mut values = dm_page_values(row, width);
        dm_apply_digests(&mut values, &page.columns, digest_columns);
        spill_large_cells(owner, &mut values)?;
        page.rows.push(Row { values });
    }
    if page.incomplete.is_some() {
        // 预算截断与「正常还有更多」互斥。
        page.has_more = false;
    }
    Ok(page)
}

/// 摘要列就地替换。文本按 UTF-8 字节、字节串按原始字节哈希，与服务端摘要口径一致。
fn dm_apply_digests(values: &mut [CellValue], columns: &[Column], digest_columns: &[String]) {
    use sha2::{Digest, Sha256};
    if digest_columns.is_empty() {
        return;
    }
    for (index, value) in values.iter_mut().enumerate() {
        let Some(column) = columns.get(index) else {
            continue;
        };
        if !digest_columns
            .iter()
            .any(|digest| digest.eq_ignore_ascii_case(&column.name))
        {
            continue;
        }
        let current = std::mem::replace(value, CellValue::Null);
        *value = match current {
            CellValue::Text(text) | CellValue::Json(text) => {
                CellValue::Text(hex_lower_digest(Sha256::digest(text.as_bytes())))
            }
            CellValue::Bytes(bytes) => {
                CellValue::Text(hex_lower_digest(Sha256::digest(&bytes)))
            }
            other => other,
        };
    }
}

fn hex_lower_digest(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    let bytes = bytes.as_ref();
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn dm_execute_parameterized(
    config: &ConnectionConfig,
    request: &QueryRequest,
    parameters: &[fluxdb_core::SqlParameter],
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statement = request.text.trim().to_string();
    if statement.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }
    let params = parameters
        .iter()
        .map(|parameter| match parameter {
            fluxdb_core::SqlParameter::Null => Param::Null,
            fluxdb_core::SqlParameter::Bool(value) => Param::Bool(*value),
            fluxdb_core::SqlParameter::I64(value) | fluxdb_core::SqlParameter::U64(value) | fluxdb_core::SqlParameter::Decimal(value) => value
                .parse::<i64>()
                .map(Param::Int)
                .unwrap_or_else(|_| Param::Str(value.clone())),
            fluxdb_core::SqlParameter::F64(value) => Param::Float(*value),
            fluxdb_core::SqlParameter::Text(value) => Param::Str(value.clone()),
            fluxdb_core::SqlParameter::Bytes(value) => Param::Bytes(value.clone()),
        })
        .collect::<Vec<_>>();
    let mut client = dm_connect(config)?;
    let started = std::time::Instant::now();
    let pagination = query_execution_pagination(request.options.page_offset, request.options.page_size);
    let content_owner =
        fluxdb_core::result_content_owner(request.session_id, request.connection_id);
    let read_only_wrap = request.options.read_only;
    if read_only_wrap {
        client
            .begin()
            .map_err(|error| dm_error("开启只读事务失败", error))?;
    }
    let result = dm_execute(&mut client, &statement, &params);
    // 执行失败也要先收尾：未回滚的事务会留在连接上，而连接随后即被丢弃。
    let finish = if read_only_wrap {
        client.rollback()
    } else {
        Ok(())
    };
    client.close();
    if let Err(error) = finish {
        let error = Error::new(ErrorKind::Query, format!("达梦只读事务回滚失败：{error}"));
        tracing::warn!(target: "fluxdb_connectors", message = %error.message, "达梦参数化会话收尾失败");
        return Err(error);
    }
    let result = result?;
    match result {
        ExecuteResult::Query { columns, rows } => {
            let columns = columns
                .iter()
                .map(|column| dm_column(column, false))
                .collect::<Vec<_>>();
            let returned = rows.len() as u64;
            let page = dm_collect_page(
                columns,
                rows.into_iter(),
                pagination,
                pagination.offset,
                &content_owner,
            )?;
            Ok(QueryExecutionResult {
                summaries: vec![QueryExecutionSummary {
                    sql: statement,
                    kind: QueryStatementKind::ResultSet,
                    success: true,
                    message: match &page.incomplete {
                        Some(incomplete) => format!(
                            "已读取 {returned} 行，当前仅展示 {} 行（{}，参数化绑定）",
                            page.rows.len(),
                            incomplete.reason
                        ),
                        None => format!("返回 {returned} 行结果表（参数化绑定）"),
                    },
                    returned_rows: returned,
                    affected_rows: 0,
                    elapsed_ms: elapsed_ms(started),
                }],
                results: vec![page],
                rollback_snapshots: Vec::new(),
            })
        }
        ExecuteResult::Affected(affected) | ExecuteResult::Call { affected, .. } => {
            let (success, message) = if read_only_wrap {
                (
                    false,
                    format!("只读入口拒绝：该语句不是只读查询，已回滚（影响 {affected} 行未提交）"),
                )
            } else {
                (true, format!("OK，影响 {affected} 行"))
            };
            Ok(QueryExecutionResult {
                summaries: vec![QueryExecutionSummary {
                    sql: statement,
                    kind: QueryStatementKind::Command,
                    success,
                    message,
                    returned_rows: 0,
                    affected_rows: affected,
                    elapsed_ms: elapsed_ms(started),
                }],
                results: Vec::new(),
                rollback_snapshots: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod dameng_page_tests {
    use super::*;
    use fluxdb_core::{CellContentMeta, CellContentStore, LargeValueKind};
    use std::sync::Arc;

    /// 只记录落盘长度、不真写文件的存储桩：这里验证的是「行有没有丢」。
    struct MemoryStore;

    impl CellContentStore for MemoryStore {
        fn put(
            &self,
            owner: &str,
            _kind: LargeValueKind,
            bytes: &[u8],
        ) -> fluxdb_core::Result<String> {
            Ok(format!("{owner}-{}", bytes.len()))
        }

        fn stat(&self, _content_id: &str) -> Option<CellContentMeta> {
            None
        }

        fn read(
            &self,
            _content_id: &str,
            _offset: u64,
            _len: usize,
        ) -> fluxdb_core::Result<Vec<u8>> {
            Ok(Vec::new())
        }

        fn release(&self, _content_id: &str) {}

        fn release_owner(&self, _owner: &str) {}
    }

    fn columns() -> Vec<Column> {
        vec![
            Column {
                name: "id".into(),
                type_name: Some("INT".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            Column {
                name: "body".into(),
                type_name: Some("BLOB".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ]
    }

    fn row(id: i32, body: Value) -> Vec<Value> {
        vec![Value::Int(id), body]
    }

    #[test]
    fn page_keeps_short_rows_and_skips_offset() {
        fluxdb_core::register_cell_content_store(Arc::new(MemoryStore));
        let rows = (0..3).map(|id| row(id, Value::Str("x".into()))).collect::<Vec<_>>();
        let page = dm_collect_page(
            columns(),
            rows.into_iter(),
            Pagination::new(1, 10),
            1,
            "result-1-c",
        )
        .expect("collect page");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[0], CellValue::I64(1));
        assert!(!page.has_more);
        assert!(page.incomplete.is_none());
    }

    /// 超大字段不能把整行挤出页：外置成内容引用后这行必须还在，且不留正文。
    #[test]
    fn oversized_cell_is_spilled_and_row_kept() {
        fluxdb_core::register_cell_content_store(Arc::new(MemoryStore));
        let oversized = Value::Bytes(vec![7u8; fluxdb_core::CELL_SPILL_THRESHOLD_BYTES + 1]);
        let page = dm_collect_page(
            columns(),
            std::iter::once(row(1, oversized)),
            Pagination::new(0, 10),
            0,
            "result-1-c",
        )
        .expect("collect page");
        assert_eq!(page.rows.len(), 1, "大字段行必须留在页里");
        assert!(page.incomplete.is_none(), "单格外置成功不算资源截断");
        assert!(matches!(
            page.rows[0].values[1],
            CellValue::LargeValue(_)
        ));
    }

    #[test]
    fn spill_failure_is_query_error_not_successful_short_page() {
        struct FailingStore;
        impl CellContentStore for FailingStore {
            fn put(&self, _: &str, _: LargeValueKind, _: &[u8]) -> fluxdb_core::Result<String> {
                Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Unsupported,
                    "测试外置失败",
                ))
            }
            fn stat(&self, _: &str) -> Option<CellContentMeta> {
                None
            }
            fn read(&self, _: &str, _: u64, _: usize) -> fluxdb_core::Result<Vec<u8>> {
                unreachable!()
            }
            fn release(&self, _: &str) {}
            fn release_owner(&self, _: &str) {}
        }

        fluxdb_core::register_cell_content_store(Arc::new(FailingStore));
        let oversized = Value::Bytes(vec![7u8; fluxdb_core::CELL_SPILL_THRESHOLD_BYTES + 1]);
        let error = dm_collect_page(
            columns(),
            std::iter::once(row(1, oversized)),
            Pagination::new(0, 10),
            0,
            "result-1-c",
        )
        .expect_err("外置失败必须冒泡");
        assert_eq!(error.message, "测试外置失败");
    }
}

#[cfg(test)]
mod dm_digest_tests {
    use super::*;

    fn column(name: &str, type_name: &str) -> Column {
        Column {
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable: true,
            primary_key: false,
            comment: None,
        }
    }

    #[test]
    fn digest_replaces_text_and_bytes_only() {
        let columns = [column("body", "blob"), column("note", "clob")];
        let mut values = vec![
            CellValue::Bytes(vec![0, 0xff]),
            CellValue::Text("hello".into()),
        ];
        dm_apply_digests(&mut values, &columns, &["body".into(), "note".into()]);
        let CellValue::Text(body) = &values[0] else {
            panic!("body 应变成摘要文本：{values:?}");
        };
        let CellValue::Text(note) = &values[1] else {
            panic!("note 应变成摘要文本：{values:?}");
        };
        assert_eq!(body.len(), 64);
        assert_eq!(note.len(), 64);
        assert_eq!(
            note,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
