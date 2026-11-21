//! 数据同步：预览比对与执行编排。
//!
//! 预览与执行共用同一个比对状态机（[`super::data_diff`]）；执行前重新比对，
//! 不信任界面上的旧快照。比对只读，写目标库前必须经过只读连接校验。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fluxdb_connectors::{TableRowWriter, connector_for};
use fluxdb_core::{
    CellValue, CompletionColumn, ConnectionConfig, DataChangeSet, DataPage, DatabaseKind, Error,
    ErrorKind, FilterOp, FilterSpec, ObjectPath, RowIdentity, RowUpdate, SortDirection, SortSpec,
    WriteIntent, is_binary_type_name, is_large_text_type_name,
};

use crate::{
    AppController, export_pages_with_digests_for_connection, load_cell_binary_for_connection,
    load_cell_text_for_connection,
};

use super::column_types::{column_family, families_compatible};
use super::data_diff::{self, ColumnPlan, KeyPlanError, TableDiff};
use super::inventory;
use super::ledger::{SyncLedger, report_status};
use super::model::{
    DataSyncOptions, DataSyncPreview, DataSyncReport, DataSyncRequest, FAILURE_DETAIL_LIMIT,
    RowErrorPolicy, RowWriteFailure, SyncEndpoint, TableDataDiff, TableDiffState, TableSkipReason,
    TableSyncReport,
};
use super::script_file::MigrationScriptFile;
use super::structure_sql;

impl AppController {
    /// 比对两端数据并返回逐表差异。只读，不修改目标库。
    ///
    /// `tables` 为空时比对源端全部表；目标端缺失的表按 `TargetMissing` 跳过，
    /// 由执行阶段根据 `create_missing` 决定是否建表。
    pub fn preview_data_sync(
        &self,
        request: &DataSyncRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<DataSyncPreview> {
        let (source_config, target_config) =
            self.sync_endpoint_configs(&request.source, &request.target)?;
        ensure_data_sync_comparable(&source_config, &target_config)?;

        let source_tables = inventory::endpoint_tables(self, &request.source)?;
        let target_tables = inventory::endpoint_tables(self, &request.target)?;
        let selected = request
            .tables
            .iter()
            .map(|table| table.trim().to_string())
            .filter(|table| !table.is_empty())
            .collect::<BTreeSet<_>>();

        tracing::info!(
            target: "fluxdb_app",
            source_tables = source_tables.len(),
            selected = selected.len(),
            "数据预览：开始"
        );
        let mut tables = Vec::new();
        for (key, source_path) in &source_tables {
            if !selected.is_empty() && !selected.contains(key) {
                continue;
            }
            check_cancel(cancel)?;
            let target_path =
                inventory::resolve(&target_tables, source_path, key, &request.options.table_map);
            let Some(target_path) = target_path else {
                tables.push(skipped_table(
                    key,
                    source_path,
                    TableSkipReason::TargetMissing,
                ));
                continue;
            };
            progress(&format!("比对 {key}"));
            let result = self.preview_table(
                &source_config,
                &target_config,
                source_path,
                target_path,
                key,
                &request.options,
                cancel,
            );
            match result {
                Ok(table) => tables.push(table),
                Err(error) if error.kind == ErrorKind::Cancelled => return Err(error),
                Err(error) => tables.push(TableDataDiff::failed(
                    key.clone(),
                    source_path.name.clone(),
                    source_path.schema.clone(),
                    error.message,
                )),
            }
        }
        for key in &selected {
            if source_tables.contains_key(key) {
                continue;
            }
            tables.push(TableDataDiff::skipped(
                key.clone(),
                key.clone(),
                None,
                TableSkipReason::SourceMissing,
            ));
        }

        Ok(DataSyncPreview {
            source_kind: source_config.kind,
            target_kind: target_config.kind,
            tables,
        })
    }

    /// 预览单表：小表逐块比完给精确结果，大表只比首块，其余由用户点「下一页」推进。
    ///
    /// 行数取一次 `COUNT(*)`；计数不支持或失败时按大表处理 —— 宁可只给片段，
    /// 也不能退回「进第四步就扫完整张千万行表」。
    #[allow(clippy::too_many_arguments)]
    fn preview_table(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source: &ObjectPath,
        target: &ObjectPath,
        key: &str,
        options: &DataSyncOptions,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<TableDataDiff> {
        let row_count = self
            .preview_data_export(source, &[], &[], &[])
            .map(|preview| preview.row_count)
            .unwrap_or(u64::MAX);
        // 行数只用于决定「整表比完 / 只比首块」，是排查预览卡顿的关键字段。
        tracing::info!(
            target: "fluxdb_app",
            table = key,
            row_count,
            "数据预览：源表行数"
        );
        // 大表首块按源端自己的块边界封顶，单块成本与目标端稀疏程度无关。
        let capped = row_count > FULL_PREVIEW_ROW_LIMIT;
        let mut table = self.diff_one_table(
            source_config,
            target_config,
            source,
            target,
            key,
            options,
            cancel,
            None,
            DEFAULT_CHUNK_ROWS,
            true,
            capped,
        )?;
        tracing::info!(
            target: "fluxdb_app",
            table = key,
            page_done = table.page_done,
            compared = table.compared_rows(),
            "数据预览：首块完成"
        );
        if capped {
            return Ok(table);
        }
        let mut chunks = 0usize;
        while !table.page_done {
            check_cancel(cancel)?;
            let Some(cursor) = table.cursor.clone() else {
                break;
            };
            // 兜底：行数统计偏小时不能让界面一直转下去，超过块数上限按未比完返回。
            if chunks >= MAX_FULL_PREVIEW_CHUNKS {
                tracing::warn!(
                    target: "fluxdb_app",
                    table = key,
                    row_count,
                    chunks,
                    "数据预览：块数超过上限，停止自动比对"
                );
                break;
            }
            let chunk_rows = table.chunk_rows;
            let page = self.diff_one_table(
                source_config,
                target_config,
                source,
                target,
                key,
                options,
                cancel,
                Some(&cursor),
                chunk_rows,
                true,
                capped,
            )?;
            table.absorb_page(page);
            chunks += 1;
            // 游标没有前进说明比对器无法继续切分，停止以免死循环。
            if table.cursor.as_ref() == Some(&cursor) {
                break;
            }
        }
        Ok(table)
    }

    pub(super) fn sync_endpoint_configs(
        &self,
        source: &super::model::SyncEndpoint,
        target: &super::model::SyncEndpoint,
    ) -> fluxdb_core::Result<(ConnectionConfig, ConnectionConfig)> {
        let source = self
            .connection_config(source.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "源连接不存在"))?
            .clone();
        let target = self
            .connection_config(target.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "目标连接不存在"))?
            .clone();
        Ok((source, target))
    }

    /// 继续比对单表的下一页差异：从 `cursor` 之后接着扫一块键区间。
    ///
    /// 预览不做全量比对，用户点「下一页」才推进一块，界面始终只持有当前块的差异。
    pub fn preview_data_sync_page(
        &self,
        request: &DataSyncRequest,
        table_key: &str,
        cursor: Option<&CellValue>,
        chunk_rows: usize,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<TableDataDiff> {
        let (source_config, target_config) =
            self.sync_endpoint_configs(&request.source, &request.target)?;
        ensure_data_sync_comparable(&source_config, &target_config)?;

        let source_tables = inventory::endpoint_tables(self, &request.source)?;
        let target_tables = inventory::endpoint_tables(self, &request.target)?;
        let Some(source_path) = source_tables.get(table_key) else {
            return Ok(TableDataDiff::skipped(
                table_key.to_string(),
                table_key.to_string(),
                None,
                TableSkipReason::SourceMissing,
            ));
        };
        let Some(target_path) = inventory::resolve(
            &target_tables,
            source_path,
            table_key,
            &request.options.table_map,
        ) else {
            return Ok(TableDataDiff::skipped(
                table_key.to_string(),
                source_path.name.clone(),
                source_path.schema.clone(),
                TableSkipReason::TargetMissing,
            ));
        };
        progress(&format!("比对 {table_key}"));
        tracing::info!(
            target: "fluxdb_app",
            table = table_key,
            has_cursor = cursor.is_some(),
            chunk_rows,
            "数据预览：比对下一页"
        );
        self.diff_one_table(
            &source_config,
            &target_config,
            source_path,
            target_path,
            table_key,
            &request.options,
            cancel,
            cursor,
            chunk_rows,
            true,
            // 「下一页」始终封顶：目标端稀疏时按目标键切块会吞掉几十万行源数据。
            true,
        )
    }

    /// 比对单表的一块键区间 `(cursor, 上界]`；表级失败（大字段、键重复等）由调用方
    /// 降级为独立结果项，不中断其它表。
    ///
    /// 目标端先按块灌入内存（最多 `chunk_rows` 行，加上同键尾部），再按同一区间扫源端，
    /// 所以单页内存只与块大小相关，不再受整表行数限制。
    #[allow(clippy::too_many_arguments)]
    fn diff_one_table(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source: &ObjectPath,
        target: &ObjectPath,
        key: &str,
        options: &DataSyncOptions,
        cancel: &AtomicBool,
        cursor: Option<&CellValue>,
        chunk_rows: usize,
        count_source_tail: bool,
        source_capped: bool,
    ) -> fluxdb_core::Result<TableDataDiff> {
        let source_columns = connector_for(source_config)?.list_completion_columns(
            source.database.as_deref(),
            source.schema.as_deref(),
            &source.name,
        )?;
        let target_columns = connector_for(target_config)?.list_completion_columns(
            target.database.as_deref(),
            target.schema.as_deref(),
            &target.name,
        )?;
        let source_names = source_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let target_names = target_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();

        let plan = match sync_plan(&source_names, &target_columns, options, key) {
            Ok(plan) => plan,
            Err(reason) => return Ok(key_plan_failure(key, source, reason)),
        };
        if let Err(message) = ensure_compatible_columns(&plan, &source_columns, &target_columns) {
            return Ok(TableDataDiff::failed(
                key.to_string(),
                source.name.clone(),
                source.schema.clone(),
                message,
            ));
        }

        let chunk_rows = chunk_rows.clamp(MIN_CHUNK_ROWS, MAX_CHUNK_ROWS);
        // 首列可能为 NULL 时不能按键区间切分：`> cursor` 会静默跳过 NULL 行。
        // 这类表退回「一次比完」，行为与分页前一致。
        let pageable = key_not_null(&source_columns, &plan.key_source_names[0])
            && key_not_null(&target_columns, &plan.key_target_names[0]);
        let digest = lob_digest_plan(
            source_config.kind,
            target_config.kind,
            &plan,
            &source_columns,
            &target_columns,
        );
        let context = ChunkContext {
            plan: &plan,
            source_names: &source_names,
            target_names: &target_names,
            pageable,
            source_digests: &digest.source_names,
            target_digests: &digest.target_names,
        };
        // 预览分块先用源端自己的块边界封顶：目标端稀疏时（目标 10 万行、源 1400 万行），
        // 目标第 4096 行的键可能落在源端几十万行之后，照它切块单块就要扫几十万行。
        let cap = if source_capped {
            self.preview_chunk_cap(source, &context, cursor, chunk_rows, cancel, key)?
        } else {
            None
        };
        let mut engine = TableDiff::new(plan.clone());
        let walk = self.walk_chunk(
            source_config,
            target_config,
            source,
            target,
            &context,
            cursor,
            chunk_rows,
            cap.as_ref(),
            cancel,
            key,
            &mut engine,
            |engine, row| engine.absorb_source(row).map(|_| ()),
        )?;

        // 预览加速：目标端已没有更大的键，源端剩余行全部是新增，直接计数即可收尾。
        // 大表（千万行级）从「再读几千块」变成一次 COUNT；执行阶段仍逐块读取写入。
        let mut walk = walk;
        let mut tail_inserted = 0u64;
        if count_source_tail
            && !walk.done
            && walk.target_exhausted
            && let Some(from) = walk.cursor.clone()
        {
            let filters = range_filters(&plan.key_source_names[0], Some(&from), None);
            match self.preview_data_export(source, &[], &[], &filters) {
                Ok(preview) => {
                    // COUNT 本身不可中断，返回后立刻响应「停止」。
                    check_cancel(cancel)?;
                    tail_inserted = preview.row_count;
                    walk.done = true;
                    walk.cursor = None;
                }
                // 连接器不支持计数时回退逐块读取，行为与之前一致。
                Err(error) if error.kind == ErrorKind::Unsupported => {}
                Err(error) => return Err(error),
            }
        }

        let outcome = engine.finish();
        let mut table =
            TableDataDiff::ready(key.to_string(), source.name.clone(), source.schema.clone());
        table.state = if outcome.inserted + outcome.updated + outcome.deleted + tail_inserted > 0 {
            TableDiffState::Different
        } else {
            TableDiffState::Equal
        };
        table.inserted = outcome.inserted + tail_inserted;
        table.updated = outcome.updated;
        table.deleted = outcome.deleted;
        table.unchanged = outcome.unchanged;
        table.key_columns = outcome.key_columns;
        table.columns_only_in_source = outcome.columns_only_in_source;
        table.columns_only_in_target = outcome.columns_only_in_target;
        table.column_count = outcome.column_count;
        table.samples = outcome.samples;
        table.page_done = walk.done;
        table.cursor = walk.cursor;
        table.chunk_rows = walk.chunk_rows;
        Ok(table)
    }

    /// 扫描一块键区间：目标端灌入 `engine`，源端逐行交给 `on_source` 处理。
    ///
    /// 预览与执行共用这段边界逻辑：目标端读够一块就停，但同键的行必须整组读完——
    /// 复合键的首列相同时，下一页用 `> 上界` 续扫会跳过这组里的后续行。
    #[allow(clippy::too_many_arguments)]
    fn walk_chunk(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source: &ObjectPath,
        target: &ObjectPath,
        context: &ChunkContext<'_>,
        cursor: Option<&CellValue>,
        chunk_rows: usize,
        cap: Option<&CellValue>,
        cancel: &AtomicBool,
        key: &str,
        engine: &mut TableDiff,
        mut on_source: impl FnMut(&mut TableDiff, &[CellValue]) -> fluxdb_core::Result<()>,
    ) -> fluxdb_core::Result<ChunkWalk> {
        let plan = context.plan;
        let first_target_index = plan.key_pairs[0].1;
        let first_source_key = plan.key_source_names[0].clone();
        let first_target_key = plan.key_target_names[0].clone();
        let chunk_limit = if context.pageable {
            chunk_rows
        } else {
            usize::MAX
        };
        let started = Instant::now();

        let mut upper: Option<CellValue> = None;
        let mut taken = 0usize;
        let target_stopped = scan_rows(
            self,
            target_config,
            target,
            &sort_by(&plan.key_target_names),
            &range_filters(&first_target_key, cursor, cap),
            context.target_names,
            context.target_digests,
            cancel,
            key,
            "目标",
            |row| {
                let value = row.get(first_target_index).ok_or_else(row_width_error)?;
                match &upper {
                    Some(bound) if value != bound => Ok(false),
                    Some(_) => engine.absorb_target(row).map(|_| true),
                    None => {
                        engine.absorb_target(row)?;
                        taken += 1;
                        if taken >= chunk_limit {
                            upper = Some(value.clone());
                        }
                        Ok(true)
                    }
                }
            },
        )?;

        // 源端：目标端还有本块上界时只扫 `(cursor, upper]`。
        //
        // 目标端已经读完（`upper` 为空）时后面全是新增，源端也必须自己按块截断：
        // 否则「目标小、源大」的表一次只点就会把剩余整张源表读完（十几分钟卡住）。
        let first_source_index = plan.key_pairs[0].0;
        let source_chunking = context.pageable && upper.is_none();
        let mut source_upper: Option<CellValue> = None;
        let mut source_taken = 0usize;
        // 封顶优先于目标端上界：目标端上界不会超过封顶值。
        let source_filters = range_filters(&first_source_key, cursor, upper.as_ref().or(cap));
        scan_rows(
            self,
            source_config,
            source,
            &sort_by(&plan.key_source_names),
            &source_filters,
            context.source_names,
            context.source_digests,
            cancel,
            key,
            "源",
            |row| {
                if source_chunking {
                    let value = row.get(first_source_index).ok_or_else(row_width_error)?;
                    match &source_upper {
                        // 越过本块上界：留到下一块，不吸收也不写入。
                        Some(bound) if value != bound => return Ok(false),
                        Some(_) => {}
                        None => {
                            source_taken += 1;
                            if source_taken >= chunk_limit {
                                source_upper = Some(value.clone());
                            }
                        }
                    }
                }
                on_source(engine, row)?;
                Ok(true)
            },
        )?;

        // 收敛判定：源端被本块上界截断则还有后续；否则目标端提前停止即确定有；
        // 两端都读完时只需探测源端在 `upper` 之后是否还有行（那些行只可能是新增）。
        let (done, next_cursor) = match (&source_upper, &upper) {
            (Some(bound), _) => (false, Some(bound.clone())),
            // 封顶切块：封顶键取自第 chunk_rows + 1 行，源端一定还有后续行，无需探测。
            // 游标取目标端本块上界与封顶值中较小的那个，保证区间不丢行。
            (None, _) if cap.is_some() => (false, upper.as_ref().or(cap).cloned()),
            (None, None) => (true, None),
            (None, Some(bound)) if target_stopped => (false, Some(bound.clone())),
            (None, Some(bound)) => {
                let more = self.has_row_after(
                    source_config,
                    source,
                    &plan.key_source_names,
                    context.source_names,
                    &first_source_key,
                    bound,
                    cancel,
                    key,
                )?;
                if more {
                    (false, Some(bound.clone()))
                } else {
                    (true, None)
                }
            }
        };

        // 目标端读到区间末尾（不是被块上界截断）：之后源端剩下的行全部是新增。
        // 但封顶后「目标端本块没读满」只说明封顶区间内没有更多目标行，必须先探测
        // 封顶之后是否还有，才能认定目标端真的耗尽。
        let target_exhausted = if target_stopped {
            false
        } else if let Some(cap) = cap {
            !self.has_row_after(
                target_config,
                target,
                &plan.key_target_names,
                context.target_names,
                &first_target_key,
                cap,
                cancel,
                key,
            )?
        } else {
            true
        };

        Ok(ChunkWalk {
            done,
            cursor: next_cursor,
            chunk_rows: next_chunk_rows(chunk_rows, started.elapsed()),
            target_exhausted,
        })
    }

    /// 读源端一小块，返回本块键上界（含）；`None` 表示源端在本区间内已读完。
    ///
    /// 单块行数由源端自己决定，不再被目标端键的稀疏程度放大。
    #[allow(clippy::too_many_arguments)]
    fn preview_chunk_cap(
        &self,
        path: &ObjectPath,
        context: &ChunkContext<'_>,
        cursor: Option<&CellValue>,
        chunk_rows: usize,
        cancel: &AtomicBool,
        key: &str,
    ) -> fluxdb_core::Result<Option<CellValue>> {
        let field = &context.plan.key_source_names[0];
        let index = context.plan.key_pairs[0].0;
        let mut seen = 0usize;
        // 第 chunk_rows 行的键：本块上界，下一块从它之后继续。
        let mut kept: Option<CellValue> = None;
        let mut cap = None;
        let mut scan_error: Option<Error> = None;
        self.export_pages_for_connection(
            path,
            &sort_by(&context.plan.key_source_names),
            &range_filters(field, cursor, None),
            &|| cancel.load(Ordering::Relaxed),
            &mut |page: DataPage| {
                for row in &page.rows {
                    let Some(value) = row.values.get(index) else {
                        scan_error = Some(row_width_error());
                        return false;
                    };
                    if seen == chunk_rows - 1 {
                        kept = Some(value.clone());
                    }
                    seen += 1;
                    if seen <= chunk_rows {
                        continue;
                    }
                    // 越过块行数：键变了才能确定边界，同一键值整组留在本块。
                    if kept.as_ref() != Some(value) {
                        cap = kept.clone();
                        return false;
                    }
                    if seen >= chunk_rows * 2 {
                        // 单个键值重复行过多，放弃封顶，退回目标端驱动的分块。
                        return false;
                    }
                }
                true
            },
        )?;
        if let Some(error) = scan_error {
            return Err(error);
        }
        check_cancel(cancel)?;
        tracing::debug!(target: "fluxdb_app", table = key, capped = cap.is_some(), "数据预览：源端封顶");
        Ok(cap)
    }

    /// 键列上是否还存在大于 `bound` 的行。分页接口没有 LIMIT，取到第一行就停。
    ///
    /// `ponytail:` 探测会多读一页（4096 行）再丢弃；等连接器支持 `LIMIT 1` 的执行路径再优化。
    #[allow(clippy::too_many_arguments)]
    fn has_row_after(
        &self,
        config: &ConnectionConfig,
        path: &ObjectPath,
        key_names: &[String],
        expected: &[String],
        field: &str,
        bound: &CellValue,
        cancel: &AtomicBool,
        key: &str,
    ) -> fluxdb_core::Result<bool> {
        let mut found = false;
        scan_rows(
            self,
            config,
            path,
            &sort_by(key_names),
            &range_filters(field, Some(bound), None),
            expected,
            &[],
            cancel,
            key,
            "探测",
            |_| {
                found = true;
                Ok(false)
            },
        )?;
        Ok(found)
    }

    /// 把摘要列换回完整值。摘要只为比对服务，写回前必须重新读取正文，
    /// 否则会把哈希文本当成真实数据写进目标表。
    fn materialize_lob_row(
        &self,
        config: &ConnectionConfig,
        path: &ObjectPath,
        plan: &ColumnPlan,
        digest: &LobDigest,
        row: &mut [CellValue],
    ) -> fluxdb_core::Result<()> {
        if digest.is_empty() {
            return Ok(());
        }
        let identity = digest_identity(plan, row)?;
        for column in &digest.columns {
            // NULL 大字段没有完整值可读，保留页里的 NULL。
            if matches!(row.get(column.source_index), Some(CellValue::Null)) {
                continue;
            }
            let value = if column.binary {
                CellValue::Bytes(load_cell_binary_for_connection(
                    config,
                    path,
                    &identity,
                    &column.source_name,
                )?)
            } else {
                CellValue::Text(load_cell_text_for_connection(
                    config,
                    path,
                    &identity,
                    &column.source_name,
                )?)
            };
            if let Some(slot) = row.get_mut(column.source_index) {
                *slot = value;
            }
        }
        Ok(())
    }

    /// 读取完整大字段；失败按行错误策略处理，返回 `false` 表示该行应跳过。
    #[allow(clippy::too_many_arguments)]
    fn materialize_or_record(
        &self,
        config: &ConnectionConfig,
        path: &ObjectPath,
        plan: &ColumnPlan,
        digest: &LobDigest,
        row: &mut [CellValue],
        policy: RowErrorPolicy,
        failures: &mut RowWriteFailures<'_>,
        progress: &mut dyn FnMut(&str),
        key: &str,
    ) -> fluxdb_core::Result<bool> {
        match self.materialize_lob_row(config, path, plan, digest, row) {
            Ok(()) => Ok(true),
            Err(error) if skippable(&error, policy) => {
                let label = source_key_label(plan, row);
                progress(&skip_line(key, &label, &error));
                failures.record(label, &error);
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}

/// 摘要可比范围：同一种库，或共享同一连接器/哈希口径的协议家族。
fn digest_kinds_compatible(source: DatabaseKind, target: DatabaseKind) -> bool {
    source == target
        || (source.is_mysql_family() && target.is_mysql_family())
        || (source.is_postgres_family() && target.is_postgres_family())
}

/// 该字段是否属于「页里只有摘要、写回需要完整值」的大字段。
fn lob_type(type_name: Option<&str>) -> bool {
    let Some(type_name) = type_name else {
        return false;
    };
    is_binary_type_name(type_name) || is_large_text_type_name(type_name)
}

/// 用比对键构造行身份（字段名按源端表列名）。
fn digest_identity(plan: &ColumnPlan, row: &[CellValue]) -> fluxdb_core::Result<RowIdentity> {
    let mut values = BTreeMap::new();
    for (position, (source_index, _)) in plan.key_pairs.iter().enumerate() {
        let Some(name) = plan.key_source_names.get(position) else {
            continue;
        };
        let value = row
            .get(*source_index)
            .cloned()
            .ok_or_else(row_width_error)?;
        values.insert(name.clone(), value);
    }
    Ok(RowIdentity { values })
}

/// 单块比对需要的表级信息（预览与执行共用）。
struct ChunkContext<'a> {
    plan: &'a ColumnPlan,
    source_names: &'a [String],
    target_names: &'a [String],
    /// 键首列两端都确定非空时才按键区间切分。
    pageable: bool,
    /// 源端需要按摘要读取的字段；空表示普通读取。
    source_digests: &'a [String],
    /// 目标端需要按摘要读取的字段；空表示普通读取。
    target_digests: &'a [String],
}

/// 摘要口径一致的两端之间的大字段摘要计划。
///
/// 摘要只在同一数据库类型或同一协议家族（MySQL 家族 / PostgreSQL 家族）时启用：
/// 不同方言的文本编码（UTF-8 / UTF-16 / 库内码页）和二进制投影口径不同，
/// 跨类型比较只会产生假差异。
struct LobDigest {
    /// 源端摘要字段名（发往源端扫描）。
    source_names: Vec<String>,
    /// 目标端摘要字段名（发往目标端扫描）。
    target_names: Vec<String>,
    /// 需要在写回前换成完整值的源端字段。
    columns: Vec<LobDigestColumn>,
}

struct LobDigestColumn {
    source_index: usize,
    source_name: String,
    /// `true` 走 `load_cell_binary`，`false` 走 `load_cell_text`。
    binary: bool,
}

impl LobDigest {
    fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

/// 计算大字段摘要列：只有两端同类（二进制/BLOB 或大文本/CLOB）的字段才参与。
fn lob_digest_plan(
    source_kind: DatabaseKind,
    target_kind: DatabaseKind,
    plan: &ColumnPlan,
    source_columns: &[CompletionColumn],
    target_columns: &[CompletionColumn],
) -> LobDigest {
    let mut digest = LobDigest {
        source_names: Vec::new(),
        target_names: Vec::new(),
        columns: Vec::new(),
    };
    if !digest_kinds_compatible(source_kind, target_kind) {
        return digest;
    }
    for (source_index, target_index) in &plan.compare_pairs {
        let Some(source_column) = source_columns.get(*source_index) else {
            continue;
        };
        let Some(target_column) = target_columns.get(*target_index) else {
            continue;
        };
        let source_type = source_column.type_name.as_deref().unwrap_or("");
        let target_type = target_column.type_name.as_deref().unwrap_or("");
        let binary = if is_binary_type_name(source_type) && is_binary_type_name(target_type) {
            true
        } else if is_large_text_type_name(source_type) && is_large_text_type_name(target_type) {
            false
        } else {
            // 一端是普通类型时不摘要：否则摘要文本与真实值天然不同，会产生假差异。
            continue;
        };
        digest.source_names.push(source_column.name.clone());
        digest.target_names.push(target_column.name.clone());
        digest.columns.push(LobDigestColumn {
            source_index: *source_index,
            source_name: source_column.name.clone(),
            binary,
        });
    }
    digest
}

/// 一块扫描的收敛结果。
struct ChunkWalk {
    /// 本块之后已无数据。
    done: bool,
    /// 下一页游标；`done` 时为 `None`。
    cursor: Option<CellValue>,
    /// nibble 后的块行数，下一页沿用它。
    chunk_rows: usize,
    /// 目标端已没有更大的键：本块之后的源端行全部是新增，预览可只计数不再读取。
    target_exhausted: bool,
}

/// 分页读取一侧的行并逐行交给调用方；`absorb` 返回 `false` 表示取够了，正常结束扫描。
///
/// 返回 `true` 表示**提前停止**（调用方取够了），`false` 表示已读到过滤范围的末尾。
///
/// 第一页的字段顺序必须与元数据一致：比对按元数据下标取值，顺序漂移会导致
/// 把 A 列当 B 列比较，宁可报错也不能静默写错数据。
#[allow(clippy::too_many_arguments)]
fn scan_rows(
    controller: &AppController,
    config: &ConnectionConfig,
    path: &ObjectPath,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    expected: &[String],
    digest_columns: &[String],
    cancel: &AtomicBool,
    key: &str,
    side: &str,
    mut absorb: impl FnMut(&[CellValue]) -> fluxdb_core::Result<bool>,
) -> fluxdb_core::Result<bool> {
    let mut scanned = 0u64;
    let mut columns_checked = false;
    let mut scan_error: Option<Error> = None;
    let mut stopped = false;
    let on_cancel = || cancel.load(Ordering::Relaxed);
    let mut on_page = |page: DataPage| {
        if !columns_checked {
            columns_checked = true;
            if let Err(error) = ensure_page_columns(&page, expected) {
                scan_error = Some(error);
                return false;
            }
        }
        match absorb_page(&page, &mut absorb) {
            Ok(true) => {}
            Ok(false) => {
                stopped = true;
                return false;
            }
            Err(error) => {
                scan_error = Some(error);
                return false;
            }
        }
        scanned += page.rows.len() as u64;
        check_progress(config, key, side, scanned);
        true
    };
    if digest_columns.is_empty() {
        controller.export_pages_for_connection(path, sort, filters, &on_cancel, &mut on_page)?;
    } else {
        export_pages_with_digests_for_connection(
            config,
            path,
            sort,
            filters,
            digest_columns,
            &on_cancel,
            &mut on_page,
        )?;
    }
    if let Some(error) = scan_error {
        return Err(error);
    }
    check_cancel(cancel)?;
    Ok(stopped)
}

fn ensure_page_columns(page: &DataPage, expected: &[String]) -> fluxdb_core::Result<()> {
    let same = page.columns.len() == expected.len()
        && page
            .columns
            .iter()
            .zip(expected)
            .all(|(column, name)| column.name.eq_ignore_ascii_case(name));
    if same {
        return Ok(());
    }
    Err(Error::new(
        ErrorKind::Internal,
        "数据页字段顺序与表元数据不一致，已停止比对",
    ))
}

/// 返回 `false` 表示调用方已取够本块数据。
fn absorb_page(
    page: &DataPage,
    absorb: &mut impl FnMut(&[CellValue]) -> fluxdb_core::Result<bool>,
) -> fluxdb_core::Result<bool> {
    for row in &page.rows {
        if !absorb(&row.values)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn row_width_error() -> Error {
    Error::new(ErrorKind::Internal, "行值与字段数量不一致")
}

/// 该列是否一定非空（列名忽略大小写）。
///
/// 主键列在数据库层面必然非空；SQLite 的 `INTEGER PRIMARY KEY` 在 `PRAGMA table_info`
/// 里 `notnull = 0`，只按 `nullable` 判断会把整库误判成不可分页。
fn key_not_null(columns: &[CompletionColumn], name: &str) -> bool {
    columns.iter().any(|column| {
        column.name.eq_ignore_ascii_case(name) && (column.primary_key || !column.nullable)
    })
}

/// 键区间过滤 `(lower, upper]`；两端都为空时等价于全表扫描。
fn range_filters(
    field: &str,
    lower: Option<&CellValue>,
    upper: Option<&CellValue>,
) -> Vec<FilterSpec> {
    let spec = |op: FilterOp, value: &CellValue| FilterSpec {
        field: field.to_string(),
        op,
        values: vec![value.clone()],
        enabled: true,
    };
    let mut filters = Vec::with_capacity(2);
    if let Some(lower) = lower {
        filters.push(spec(FilterOp::GreaterThan, lower));
    }
    if let Some(upper) = upper {
        filters.push(spec(FilterOp::LessThanOrEqual, upper));
    }
    filters
}

/// 首页块大小。
const DEFAULT_CHUNK_ROWS: usize = 4096;
/// 预览自动比完的行数上限：不超过它的表进第四步就给整表结果，超过的只比首块。
///
/// 千万行表全量预览在远程库上要几十分钟，而执行阶段本来就会重新全量比对，
/// 整表预览属于白花的等待。
const FULL_PREVIEW_ROW_LIMIT: u64 = 100_000;
/// 自动比完的块数上限（100k 行按 4096/块约 25 块，留余量）；超限按未比完返回。
const MAX_FULL_PREVIEW_CHUNKS: usize = 32;
/// 块下限：太小会让「下一页」变成多次往返。
const MIN_CHUNK_ROWS: usize = 512;
/// 块上限取连接器单页行数：块超过一页会引入 OFFSET 翻页，反而更慢。
const MAX_CHUNK_ROWS: usize = 4096;
/// 单块平均每行耗时超过该值就减半（借鉴 pt-table-sync 的 nibble 策略）。
///
/// 不能用「单块总耗时」判断：建连、元数据、空区间探测是固定成本，远程库上一块
/// 4096 行本来就要 2~3s，按总耗时减半会把块越切越小、往返次数翻几倍。
const SLOW_ROW: Duration = Duration::from_millis(2);
/// 单块耗时低于该值就翻倍。
const FAST_CHUNK: Duration = Duration::from_millis(500);

/// nibble 自适应：以「每行耗时」判断快慢，块太慢减半、太快翻倍。
fn next_chunk_rows(rows: usize, elapsed: Duration) -> usize {
    if rows == 0 {
        return rows;
    }
    if elapsed.as_secs_f64() / rows as f64 > SLOW_ROW.as_secs_f64() {
        (rows / 2).max(MIN_CHUNK_ROWS)
    } else if elapsed < FAST_CHUNK {
        (rows * 2).min(MAX_CHUNK_ROWS)
    } else {
        rows
    }
}

/// 进度日志按页输出，方便排查「卡在哪个表、读到多少行」。
fn check_progress(config: &ConnectionConfig, key: &str, side: &str, scanned: u64) {
    tracing::debug!(connection = %config.name, table = key, side, scanned, "数据同步扫描进度");
}

fn sort_by(columns: &[String]) -> Vec<SortSpec> {
    columns
        .iter()
        .map(|field| SortSpec {
            field: field.clone(),
            direction: SortDirection::Asc,
        })
        .collect()
}

fn skipped_table(key: &str, path: &ObjectPath, reason: TableSkipReason) -> TableDataDiff {
    TableDataDiff::skipped(
        key.to_string(),
        path.name.clone(),
        path.schema.clone(),
        reason,
    )
}

/// 键列缺失是配置问题，需要把列名带回界面；其余原因只展示固定文案。
fn key_plan_failure(key: &str, path: &ObjectPath, reason: KeyPlanError) -> TableDataDiff {
    match reason {
        KeyPlanError::MissingColumn(column) => TableDataDiff::failed(
            key.to_string(),
            path.name.clone(),
            path.schema.clone(),
            format!("比对键 {column} 在某一端不存在"),
        ),
        other => skipped_table(key, path, key_plan_reason(other)),
    }
}

fn key_plan_reason(reason: KeyPlanError) -> TableSkipReason {
    match reason {
        KeyPlanError::NoKey => TableSkipReason::NoKey,
        KeyPlanError::MissingColumn(_) => TableSkipReason::KeyColumnMissing,
        KeyPlanError::NoSharedColumns => TableSkipReason::NoSharedColumns,
    }
}

/// 数据同步允许任意关系型数据库组合；值转换与类型兼容性由逐表计划负责。
pub(super) fn ensure_data_sync_comparable(
    source: &ConnectionConfig,
    target: &ConnectionConfig,
) -> fluxdb_core::Result<()> {
    for config in [source, target] {
        if matches!(config.kind, DatabaseKind::Redis | DatabaseKind::MongoDb) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "当前连接类型不支持数据同步",
            ));
        }
    }
    Ok(())
}

/// 同方言 / MySQL 家族可直接复用源端 DDL；其它组合走类型映射生成 DDL。
pub(super) fn structure_kinds_compatible(source: DatabaseKind, target: DatabaseKind) -> bool {
    source == target || (source.is_mysql_family() && target.is_mysql_family())
}

/// 建立列计划：先按用户映射改名，再解析键/跳过列，最后注入目标列类型族。
fn sync_plan(
    source_names: &[String],
    target_columns: &[CompletionColumn],
    options: &DataSyncOptions,
    table_key: &str,
) -> Result<ColumnPlan, KeyPlanError> {
    let column_map = options.column_map_for(table_key);
    let mapped_names = mapped_source_names(source_names, &column_map);
    let target_names = target_columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let target_primary_key = target_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mapped_keys = options
        .key_columns_for(table_key)
        .iter()
        .map(|name| mapped_name(name, &column_map).unwrap_or(name).to_string())
        .collect::<Vec<_>>();
    let key_columns = data_diff::resolve_key_columns(
        &mapped_keys,
        &mapped_names,
        &target_names,
        &target_primary_key,
    )?;
    let families = target_columns
        .iter()
        .map(|column| column_family(column.type_name.as_deref()))
        .collect::<Vec<_>>();
    Ok(
        data_diff::plan_columns(&mapped_names, &target_names, &key_columns)?
            .skip_columns(options.skip_columns_for(table_key))
            .with_target_families(families),
    )
}

fn mapped_source_names(source_names: &[String], map: &BTreeMap<String, String>) -> Vec<String> {
    source_names
        .iter()
        .map(|name| {
            mapped_name(name, map)
                .map(str::to_string)
                .unwrap_or_else(|| name.clone())
        })
        .collect()
}

fn mapped_name<'a>(name: &str, map: &'a BTreeMap<String, String>) -> Option<&'a str> {
    map.get(name)
        .or_else(|| {
            map.iter()
                .find(|(source, _)| source.eq_ignore_ascii_case(name))
                .map(|(_, target)| target)
        })
        .map(String::as_str)
}

/// 预览阶段先按类型族挡掉必然不可写的列组合，避免扫描整表后才在首行失败。
fn ensure_compatible_columns(
    plan: &ColumnPlan,
    source_columns: &[CompletionColumn],
    target_columns: &[CompletionColumn],
) -> Result<(), String> {
    let mut conflicts = Vec::new();
    for (source_index, target_index) in plan.key_pairs.iter().chain(&plan.compare_pairs) {
        let Some(source_column) = source_columns.get(*source_index) else {
            continue;
        };
        let Some(target_column) = target_columns.get(*target_index) else {
            continue;
        };
        if families_compatible(
            column_family(source_column.type_name.as_deref()),
            column_family(target_column.type_name.as_deref()),
        ) {
            continue;
        }
        conflicts.push(format!(
            "{} ({} → {})",
            target_column.name,
            source_column.type_name.as_deref().unwrap_or("?"),
            target_column.type_name.as_deref().unwrap_or("?")
        ));
    }
    if conflicts.is_empty() {
        return Ok(());
    }
    let shown = conflicts
        .iter()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if conflicts.len() > 5 {
        format!(" 等 {} 列", conflicts.len())
    } else {
        String::new()
    };
    Err(format!(
        "字段类型不兼容，已跳过该表（可先做结构同步或调整字段映射）: {shown}{suffix}"
    ))
}

fn check_cancel(cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::new(ErrorKind::Cancelled, "数据同步已取消"));
    }
    Ok(())
}

impl AppController {
    /// 执行数据同步。每张表执行前重新比对，避免使用过期预览。
    ///
    /// 表级失败写入报告并继续其它表；连接错误与用户取消直接返回 `Err`。
    pub fn execute_data_sync(
        &self,
        request: &DataSyncRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<DataSyncReport> {
        if !request.options.writes_anything() {
            return Err(Error::new(ErrorKind::Query, "至少选择一种同步动作"));
        }
        if request.options.atomic_table && !request.options.overwrite {
            return Err(Error::new(
                ErrorKind::Query,
                "表级事务仅支持覆盖模式（清空目标表后全量写入）",
            ));
        }
        if request.options.atomic_table && request.options.export_sql_path.is_some() {
            return Err(Error::new(
                ErrorKind::Query,
                "表级事务与导出迁移 SQL 不能同时使用：事务回滚后脚本会与实际目标数据不一致",
            ));
        }
        let (source_config, target_config) =
            self.sync_endpoint_configs(&request.source, &request.target)?;
        ensure_data_sync_comparable(&source_config, &target_config)?;
        crate::ensure_read_only_allows(&target_config, WriteIntent::Mutation)?;

        let source_tables = inventory::endpoint_tables(self, &request.source)?;
        let target_tables = inventory::endpoint_tables(self, &request.target)?;
        let selected = selected_tables(&request.tables);

        // 断点续传：签名一致才允许；覆盖模式没有块游标，不支持续传。
        let signature = data_sync_signature(request);
        let resume = match request.options.resume_from_run {
            Some(_) if request.options.overwrite => {
                return Err(Error::new(
                    ErrorKind::Query,
                    "覆盖模式不支持断点续传，请重新开始同步",
                ));
            }
            Some(run_id) => {
                let plan = self.load_sync_resume(run_id)?;
                if plan.signature != signature {
                    return Err(Error::new(
                        ErrorKind::Query,
                        "上次任务的连接、表或选项与当前不一致，无法续传",
                    ));
                }
                progress(&format!("从执行记录 #{run_id} 继续未完成的表"));
                Some(plan)
            }
            None => None,
        };

        // 执行账本：逐块记录写了什么，成败都不影响同步本身。
        let ledger = SyncLedger::begin(
            self.sync_run_storage.clone(),
            &endpoint_label(&source_config, &request.source),
            &endpoint_label(&target_config, &request.target),
            &signature,
        );

        // 导出迁移 SQL：只追加成功执行的语句；写入失败按任务失败处理。
        let mut script = match request.options.export_sql_path.as_deref() {
            Some(path) => Some(MigrationScriptFile::create(path)?),
            None => None,
        };
        let mut tables = Vec::new();
        // 闭包借用 `script`；用块作用域收回借用，才能在循环后 finish 文件。
        {
            let mut emit_script = |sql: &str| -> fluxdb_core::Result<()> {
                match script.as_mut() {
                    Some(script) => script.write(sql),
                    None => Ok(()),
                }
            };
            for (key, source_path) in &source_tables {
                if !selected.is_empty() && !selected.contains(key) {
                    continue;
                }
                if let Err(error) = check_cancel(cancel) {
                    ledger.finish("canceled", &tables);
                    return Err(error);
                }
                let resume_table = resume.as_ref().and_then(|plan| plan.tables.get(key));
                if resume_table.is_some_and(|table| table.completed) {
                    progress(&format!("{key}: 上次已完成，跳过"));
                    let report = TableSyncReport::skipped(
                        key.clone(),
                        source_path.name.clone(),
                        "上次同步已完成，本次跳过",
                    );
                    ledger.record_table(&report);
                    tables.push(report);
                    continue;
                }
                // 游标是上一次完整块的上界；解析失败时从表头重来，宁可多扫不跳过数据。
                let start_cursor = resume_table
                    .and_then(|table| table.cursor.as_deref())
                    .and_then(|json| serde_json::from_str::<fluxdb_core::CellValue>(json).ok());
                progress(&format!("同步 {key}"));
                let result = self.sync_one_table(
                    &source_config,
                    &target_config,
                    source_path,
                    inventory::resolve(
                        &target_tables,
                        source_path,
                        key,
                        &request.options.table_map,
                    ),
                    key,
                    request,
                    start_cursor.as_ref(),
                    &ledger,
                    cancel,
                    progress,
                    &mut emit_script,
                );
                match result {
                    Ok(report) => {
                        ledger.record_table(&report);
                        tables.push(report);
                    }
                    Err(error) if error.kind == ErrorKind::Cancelled => {
                        ledger.finish("canceled", &tables);
                        return Err(error);
                    }
                    Err(error) => {
                        let report = TableSyncReport::failed(
                            key.clone(),
                            source_path.name.clone(),
                            error.message,
                        );
                        ledger.record_table(&report);
                        tables.push(report);
                    }
                }
            }
        }
        if let Some(mut script) = script {
            script.finish()?;
        }
        let report = DataSyncReport {
            tables,
            run_id: ledger.run_id(),
        };
        ledger.finish(report_status(&report), &report.tables);
        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn sync_one_table(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source: &ObjectPath,
        existing_target: Option<&ObjectPath>,
        key: &str,
        request: &DataSyncRequest,
        start_cursor: Option<&fluxdb_core::CellValue>,
        ledger: &SyncLedger,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
        script: &mut dyn FnMut(&str) -> fluxdb_core::Result<()>,
    ) -> fluxdb_core::Result<TableSyncReport> {
        let target = match existing_target {
            Some(path) => path.clone(),
            None if request.options.create_missing => {
                let path = inventory::target_path(
                    &request.target,
                    target_config.kind,
                    &request.options.table_map,
                    key,
                    source,
                );
                structure_sql::ensure_target_table(source_config, target_config, source, &path)?;
                progress(&format!("{key}: 已按源端 DDL 建表"));
                path
            }
            None => {
                return Ok(TableSyncReport::skipped(
                    key.to_string(),
                    source.name.clone(),
                    TableSkipReason::TargetMissing.message(),
                ));
            }
        };

        let source_columns = self.table_columns(source_config, source)?;
        let source_names = source_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        // 写入器按映射后的字段名匹配目标列；跳过的字段在这里一并过滤。
        let column_map = request.options.column_map_for(key);
        let mapped_names = mapped_source_names(&source_names, &column_map);
        // 唯一键冲突策略只影响 INSERT 语句；比对与预览结果不变。
        let mut writer = TableRowWriter::new_with_skips(
            target_config,
            &target,
            &mapped_names,
            &request.options.skip_columns,
        )?
        .with_conflict(request.options.insert_conflict);

        if request.options.atomic_table {
            // 清空与全部 INSERT 共用一个事务；失败时由写入器 Drop 回滚。
            writer.begin_atomic()?;
        }

        let target_columns = self.table_columns(target_config, &target)?;
        if request.options.overwrite {
            return self.sync_overwrite(
                source_config,
                target_config,
                source,
                &source_names,
                &source_columns,
                &target_columns,
                &mut writer,
                key,
                &request.options,
                ledger,
                cancel,
                progress,
                script,
            );
        }
        self.sync_incremental(
            source_config,
            target_config,
            source,
            &target,
            &source_names,
            &source_columns,
            &target_columns,
            &mut writer,
            key,
            request,
            start_cursor,
            ledger,
            cancel,
            progress,
            script,
        )
    }

    /// 覆盖模式：清空目标表后全量写入，不依赖主键，也不做逐行比对。
    #[allow(clippy::too_many_arguments)]
    fn sync_overwrite(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source: &ObjectPath,
        source_names: &[String],
        source_columns: &[CompletionColumn],
        target_columns: &[CompletionColumn],
        writer: &mut TableRowWriter,
        key: &str,
        options: &DataSyncOptions,
        ledger: &SyncLedger,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
        script: &mut dyn FnMut(&str) -> fluxdb_core::Result<()>,
    ) -> fluxdb_core::Result<TableSyncReport> {
        // 大字段在页里只有摘要，写回前要按行身份重读完整值；没有比对键就无法定位行。
        let lob_present = source_columns
            .iter()
            .any(|column| lob_type(column.type_name.as_deref()));
        let plan = match sync_plan(source_names, target_columns, options, key) {
            Ok(plan) => Some(plan),
            Err(_) if !lob_present => None,
            Err(reason) => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "覆盖模式同步大字段需要可用的比对键（主键或手填键列）：{}",
                        key_plan_error(reason)
                    ),
                ));
            }
        };
        let digest = match plan.as_ref() {
            Some(plan) => lob_digest_plan(
                source_config.kind,
                target_config.kind,
                plan,
                source_columns,
                target_columns,
            ),
            None => LobDigest {
                source_names: Vec::new(),
                target_names: Vec::new(),
                columns: Vec::new(),
            },
        };
        writer.truncate(cancel)?;
        script(&writer.render_clear_script())?;
        progress(&format!("{key}: 已清空目标表"));
        let mut inserted = 0u64;
        let mut failures = RowWriteFailures::new(ledger, key);
        let mut ordinal = 0u64;
        scan_rows(
            self,
            source_config,
            source,
            &[],
            &[],
            source_names,
            &digest.source_names,
            cancel,
            key,
            "源",
            |row| {
                ordinal += 1;
                let mut values = row.to_vec();
                if let Some(plan) = plan.as_ref()
                    && let Err(error) =
                        self.materialize_lob_row(source_config, source, plan, &digest, &mut values)
                {
                    if skippable(&error, options.row_error_policy) {
                        let label = format!("第 {ordinal} 行");
                        progress(&skip_line(key, &label, &error));
                        failures.record(label, &error);
                        return Ok(true);
                    }
                    return Err(error);
                }
                match writer.write_rows(std::slice::from_ref(&values), cancel, &mut |_, _| {}) {
                    Ok(written) => {
                        inserted += written;
                        for sql in writer.render_insert_scripts(std::slice::from_ref(&values))? {
                            script(&sql)?;
                        }
                    }
                    // 覆盖模式没有比对键可用，失败行只按源端序号定位。
                    Err(error) if skippable(&error, options.row_error_policy) => {
                        let label = format!("第 {ordinal} 行");
                        progress(&skip_line(key, &label, &error));
                        failures.record(label, &error);
                    }
                    Err(error) => return Err(error),
                }
                Ok(true)
            },
        )?;
        // 全部批次写完才提交；上面任何一步失败都会让写入器 Drop 回滚整表。
        writer.commit_atomic()?;
        Ok(TableSyncReport {
            key: key.to_string(),
            name: source.name.clone(),
            inserted,
            updated: 0,
            deleted: 0,
            skipped: false,
            error: None,
            failed_rows: failures.total,
            failures: failures.details,
        })
    }

    /// 增量模式：主键比对后按 INSERT / UPDATE / DELETE 分批写入。
    #[allow(clippy::too_many_arguments)]
    fn sync_incremental(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source: &ObjectPath,
        target: &ObjectPath,
        source_names: &[String],
        source_columns: &[CompletionColumn],
        target_columns: &[CompletionColumn],
        writer: &mut TableRowWriter,
        key: &str,
        request: &DataSyncRequest,
        start_cursor: Option<&fluxdb_core::CellValue>,
        ledger: &SyncLedger,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
        script: &mut dyn FnMut(&str) -> fluxdb_core::Result<()>,
    ) -> fluxdb_core::Result<TableSyncReport> {
        let options = &request.options;
        let target_names = target_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let plan = sync_plan(source_names, target_columns, options, key).map_err(key_plan_error)?;
        ensure_compatible_columns(&plan, source_columns, target_columns)
            .map_err(|message| Error::new(ErrorKind::Unsupported, message))?;

        // 逐块比对 + 写入：单块内存只与块大小相关，超大表不再受整表索引限制。
        let pageable = key_not_null(source_columns, &plan.key_source_names[0])
            && key_not_null(target_columns, &plan.key_target_names[0]);
        let digest = lob_digest_plan(
            source_config.kind,
            target_config.kind,
            &plan,
            source_columns,
            target_columns,
        );
        let context = ChunkContext {
            plan: &plan,
            source_names,
            target_names: &target_names,
            pageable,
            source_digests: &digest.source_names,
            target_digests: &digest.target_names,
        };
        let batch_rows = options.batch_rows();
        let policy = options.row_error_policy;
        let mut pacer = WritePacer::new(batch_rows);
        let mut failures = RowWriteFailures::new(ledger, key);
        let mut deleted = 0u64;
        let mut cursor: Option<CellValue> = start_cursor.cloned();
        let mut chunk_rows = DEFAULT_CHUNK_ROWS;
        let mut table_ledger = ledger.table(key, options);
        loop {
            check_cancel(cancel)?;
            let chunk_started = Instant::now();
            let (inserted_before, updated_before, deleted_before) =
                (pacer.inserted, pacer.updated, deleted);
            let key_from = cursor.as_ref().map(CellValue::display_label);
            let mut engine = TableDiff::new(plan.clone());
            let walk = self.walk_chunk(
                source_config,
                target_config,
                source,
                target,
                &context,
                cursor.as_ref(),
                chunk_rows,
                None,
                cancel,
                key,
                &mut engine,
                |engine, row| {
                    // 键值本身转换失败（如源端 id 是文本、目标端是整数）时，
                    // 连「这一行属于哪一行」都判定不了，只能按策略跳过并记录。
                    let action = match engine.absorb_source(row) {
                        Ok(action) => action,
                        Err(error) if skippable(&error, policy) => {
                            let label = source_key_label(&plan, row);
                            progress(&skip_line(key, &label, &error));
                            failures.record(label, &error);
                            return Ok(());
                        }
                        Err(error) => return Err(error),
                    };
                    match action {
                        data_diff::SourceRowAction::Insert if options.insert => {
                            if let Some(mut normalized) = coerce_row_or_record(
                                &plan,
                                row,
                                policy,
                                &mut failures,
                                progress,
                                key,
                            )? {
                                if !self.materialize_or_record(
                                    source_config,
                                    source,
                                    &plan,
                                    &digest,
                                    &mut normalized,
                                    policy,
                                    &mut failures,
                                    progress,
                                    key,
                                )? {
                                    return Ok(());
                                }
                                let label = source_key_label(&plan, &normalized);
                                pacer.push_insert(label, normalized);
                            }
                        }
                        data_diff::SourceRowAction::Update if options.update => {
                            if let Some(mut normalized) = coerce_row_or_record(
                                &plan,
                                row,
                                policy,
                                &mut failures,
                                progress,
                                key,
                            )? {
                                if !self.materialize_or_record(
                                    source_config,
                                    source,
                                    &plan,
                                    &digest,
                                    &mut normalized,
                                    policy,
                                    &mut failures,
                                    progress,
                                    key,
                                )? {
                                    return Ok(());
                                }
                                let label = source_key_label(&plan, &normalized);
                                pacer.push_update(
                                    label,
                                    build_update(&plan, &target_names, &normalized)?,
                                );
                            }
                        }
                        _ => {}
                    }
                    pacer.flush(
                        writer,
                        target_config,
                        target,
                        cancel,
                        progress,
                        key,
                        false,
                        &mut failures,
                        policy,
                        script,
                    )?;
                    Ok(())
                },
            )?;
            // 本块写完再删本块多余行：两边的键集合已在本块内闭合。
            pacer.flush(
                writer,
                target_config,
                target,
                cancel,
                progress,
                key,
                true,
                &mut failures,
                policy,
                script,
            )?;
            let outcome = engine.finish();
            if options.delete && !outcome.deleted_keys.is_empty() {
                let keys = outcome
                    .deleted_keys
                    .into_iter()
                    .map(|values| (key_label(&plan.key_target_names, &values), values))
                    .collect::<Vec<_>>();
                deleted += apply_deletes(
                    writer,
                    target_config,
                    target,
                    &plan.key_target_names,
                    keys,
                    batch_rows,
                    &mut failures,
                    policy,
                    progress,
                    key,
                    script,
                )?;
            }
            let key_to = if walk.done {
                None
            } else {
                walk.cursor.as_ref().map(CellValue::display_label)
            };
            // 精确游标与展示文本分开存：续传不能靠格式化后的字符串反推类型。
            let key_to_value = if walk.done {
                None
            } else {
                walk.cursor
                    .as_ref()
                    .and_then(|value| serde_json::to_string(value).ok())
            };
            table_ledger.record(
                key_from,
                key_to,
                key_to_value,
                pacer.inserted - inserted_before,
                pacer.updated - updated_before,
                deleted - deleted_before,
                chunk_started.elapsed().as_millis() as u64,
                &outcome.samples,
            );
            if walk.done {
                break;
            }
            cursor = walk.cursor;
            chunk_rows = walk.chunk_rows;
        }
        Ok(TableSyncReport {
            key: key.to_string(),
            name: source.name.clone(),
            inserted: pacer.inserted,
            updated: pacer.updated,
            deleted,
            skipped: false,
            error: None,
            failed_rows: failures.total,
            failures: failures.details,
        })
    }

    fn table_columns(
        &self,
        config: &ConnectionConfig,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        connector_for(config)?.list_completion_columns(
            path.database.as_deref(),
            path.schema.as_deref(),
            &path.name,
        )
    }
}

/// 每批写入的缓冲。攒够 `batch_rows` 就刷一次，避免单条 SQL 过大或内存堆积。
///
/// 行都带键展示文本：写入失败时用它定位到具体行，记进报告和日志。
struct WritePacer {
    batch_rows: usize,
    inserts: Vec<(String, Vec<CellValue>)>,
    updates: Vec<(String, RowUpdate)>,
    inserted: u64,
    updated: u64,
}

impl WritePacer {
    fn new(batch_rows: usize) -> Self {
        Self {
            batch_rows,
            inserts: Vec::new(),
            updates: Vec::new(),
            inserted: 0,
            updated: 0,
        }
    }

    fn push_insert(&mut self, label: String, row: Vec<CellValue>) {
        self.inserts.push((label, row));
    }

    fn push_update(&mut self, label: String, update: RowUpdate) {
        self.updates.push((label, update));
    }

    #[allow(clippy::too_many_arguments)]
    fn flush(
        &mut self,
        writer: &TableRowWriter,
        target_config: &ConnectionConfig,
        target: &ObjectPath,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
        key: &str,
        force: bool,
        failures: &mut RowWriteFailures<'_>,
        policy: RowErrorPolicy,
        script: &mut dyn FnMut(&str) -> fluxdb_core::Result<()>,
    ) -> fluxdb_core::Result<()> {
        if !self.inserts.is_empty() && (force || self.inserts.len() >= self.batch_rows) {
            let batch = std::mem::take(&mut self.inserts);
            let rows = batch.iter().map(|(_, row)| row.clone()).collect::<Vec<_>>();
            // 先渲染脚本再执行：不支持的字段值在写目标库之前就失败。
            let insert_scripts = writer.render_insert_scripts(&rows)?;
            match writer.write_rows(&rows, cancel, &mut |_, _| {}) {
                Ok(written) => {
                    self.inserted += written;
                    for sql in &insert_scripts {
                        script(sql)?;
                    }
                    progress(&format!("{key}: 已写入 {written} 行"));
                }
                Err(error) if skippable(&error, policy) => {
                    // 批量语句一行坏全批回滚：退回逐行写，只丢真正失败的行。
                    for (label, row) in batch {
                        let row_scripts =
                            writer.render_insert_scripts(std::slice::from_ref(&row))?;
                        match writer.write_rows(std::slice::from_ref(&row), cancel, &mut |_, _| {})
                        {
                            Ok(written) => {
                                self.inserted += written;
                                for sql in &row_scripts {
                                    script(sql)?;
                                }
                            }
                            Err(row_error) if row_error.kind == ErrorKind::Cancelled => {
                                return Err(row_error);
                            }
                            Err(row_error) => {
                                progress(&skip_line(key, &label, &row_error));
                                failures.record(label, &row_error);
                            }
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }
        if !self.updates.is_empty() && (force || self.updates.len() >= self.batch_rows) {
            let updates = std::mem::take(&mut self.updates);
            let count = updates.len() as u64;
            let batch = updates
                .iter()
                .map(|(_, update)| update.clone())
                .collect::<Vec<_>>();
            // 先渲染脚本再执行：不支持的字段值在写目标库之前就失败。
            let update_scripts = batch
                .iter()
                .map(|update| writer.render_update_script(update))
                .collect::<fluxdb_core::Result<Vec<_>>>()?;
            match apply_updates(target_config, target, batch) {
                Ok(()) => {
                    self.updated += count;
                    for sql in &update_scripts {
                        script(sql)?;
                    }
                    progress(&format!("{key}: 已更新 {count} 行"));
                }
                Err(error) if skippable(&error, policy) => {
                    for (label, update) in updates {
                        let update_script = writer.render_update_script(&update)?;
                        match apply_updates(target_config, target, vec![update]) {
                            Ok(()) => {
                                self.updated += 1;
                                script(&update_script)?;
                            }
                            Err(row_error) if row_error.kind == ErrorKind::Cancelled => {
                                return Err(row_error);
                            }
                            Err(row_error) => {
                                progress(&skip_line(key, &label, &row_error));
                                failures.record(label, &row_error);
                            }
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// 跳过策略命中：取消信号必须原样上抛，不能当成普通行错误吞掉。
fn skippable(error: &Error, policy: RowErrorPolicy) -> bool {
    policy == RowErrorPolicy::Skip && error.kind != ErrorKind::Cancelled
}

fn skip_line(key: &str, label: &str, error: &Error) -> String {
    format!("{key}: 跳过失败行 {label} · {}", error.message)
}

/// 跳过写入失败的累计器：总数 + 有上限的报告明细 + 逐条落本地的账本。
///
/// 报告里只留 [`FAILURE_DETAIL_LIMIT`] 条，避免大表把内存撑爆；账本另行逐条写入，
/// 回看时能拿到全量失败行。
struct RowWriteFailures<'a> {
    total: u64,
    details: Vec<RowWriteFailure>,
    ledger: Option<(&'a SyncLedger, &'a str)>,
}

impl<'a> RowWriteFailures<'a> {
    fn new(ledger: &'a SyncLedger, table_key: &'a str) -> Self {
        Self {
            total: 0,
            details: Vec::new(),
            ledger: Some((ledger, table_key)),
        }
    }

    fn record(&mut self, key: String, error: &Error) {
        self.total += 1;
        if let Some((ledger, table_key)) = self.ledger {
            ledger.record_failure(table_key, &key, &error.message);
        }
        if self.details.len() < FAILURE_DETAIL_LIMIT {
            self.details.push(RowWriteFailure {
                key,
                message: error.message.clone(),
            });
        }
    }
}

/// 把源行规范化到目标类型；`Skip` 策略下转换失败按行记录并返回 `None`。
///
/// 类型转换发生在写入缓冲之前，和写语句失败一样会卡住整张表，
/// 所以同样受 `row_error_policy` 管控。
fn coerce_row_or_record(
    plan: &ColumnPlan,
    row: &[CellValue],
    policy: RowErrorPolicy,
    failures: &mut RowWriteFailures<'_>,
    progress: &mut dyn FnMut(&str),
    key: &str,
) -> fluxdb_core::Result<Option<Vec<CellValue>>> {
    match plan.coerce_source_row(row) {
        Ok(normalized) => Ok(Some(normalized)),
        Err(error) if skippable(&error, policy) => {
            // 转换失败时拿不到规范化后的值，直接用源行原文拼键展示文本。
            let label = source_key_label(plan, row);
            progress(&skip_line(key, &label, &error));
            failures.record(label, &error);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// 源行的键展示文本；源行与键列下标都按源表字段顺序。
fn source_key_label(plan: &ColumnPlan, row: &[CellValue]) -> String {
    let values = plan
        .key_pairs
        .iter()
        .filter_map(|(source_index, _)| row.get(*source_index).cloned())
        .collect::<Vec<_>>();
    key_label(&plan.key_source_names, &values)
}

/// 键列展示文本 `id=42, name=x`；超长值截断，避免把大字段塞进日志。
fn key_label(names: &[String], values: &[CellValue]) -> String {
    names
        .iter()
        .zip(values)
        .map(|(name, value)| {
            let mut text = value.display_label();
            if text.chars().count() > 40 {
                text = text.chars().take(40).collect::<String>() + "…";
            }
            format!("{name}={text}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 把源行转成目标端的 UPDATE：身份用键列，写入非键的同名字段。
fn build_update(
    plan: &data_diff::ColumnPlan,
    target_names: &[String],
    row: &[CellValue],
) -> fluxdb_core::Result<RowUpdate> {
    let mut identity = std::collections::BTreeMap::new();
    for (source_index, target_index) in &plan.key_pairs {
        identity.insert(
            target_names[*target_index].clone(),
            row_value(row, *source_index)?,
        );
    }
    let mut cells = Vec::with_capacity(plan.compare_pairs.len());
    for (source_index, target_index) in &plan.compare_pairs {
        cells.push(fluxdb_core::CellUpdate::new(
            target_names[*target_index].clone(),
            row_value(row, *source_index)?,
        ));
    }
    Ok(RowUpdate {
        identity: RowIdentity { values: identity },
        cells,
    })
}

fn row_value(row: &[CellValue], index: usize) -> fluxdb_core::Result<CellValue> {
    row.get(index)
        .cloned()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "行值与字段数量不一致"))
}

fn apply_updates(
    config: &ConnectionConfig,
    target: &ObjectPath,
    updates: Vec<RowUpdate>,
) -> fluxdb_core::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    connector_for(config)?
        .apply_changes(&DataChangeSet {
            object: target.clone(),
            inserts: Vec::new(),
            updates,
            deletes: Vec::new(),
            insert_intents: None,
        })
        .map(|_| ())
}

/// 批量删除；`Skip` 策略下失败的行单独重试，成功多少返回多少。
#[allow(clippy::too_many_arguments)]
fn apply_deletes(
    writer: &TableRowWriter,
    config: &ConnectionConfig,
    target: &ObjectPath,
    key_names: &[String],
    keys: Vec<(String, Vec<CellValue>)>,
    batch_rows: usize,
    failures: &mut RowWriteFailures<'_>,
    policy: RowErrorPolicy,
    progress: &mut dyn FnMut(&str),
    table_key: &str,
    script: &mut dyn FnMut(&str) -> fluxdb_core::Result<()>,
) -> fluxdb_core::Result<u64> {
    let mut deleted = 0u64;
    for chunk in keys.chunks(batch_rows.max(1)) {
        // 先渲染脚本再执行：不支持的字段值在删目标行之前就失败。
        let chunk_scripts = chunk
            .iter()
            .map(|(_, values)| writer.render_delete_script(&row_identity(key_names, values)))
            .collect::<fluxdb_core::Result<Vec<_>>>()?;
        match apply_delete_chunk(config, target, key_names, chunk) {
            Ok(()) => {
                deleted += chunk.len() as u64;
                for sql in &chunk_scripts {
                    script(sql)?;
                }
            }
            Err(error) if skippable(&error, policy) => {
                for (label, values) in chunk {
                    let row_script =
                        writer.render_delete_script(&row_identity(key_names, values))?;
                    match apply_delete_chunk(
                        config,
                        target,
                        key_names,
                        std::slice::from_ref(&(label.clone(), values.clone())),
                    ) {
                        Ok(()) => {
                            deleted += 1;
                            script(&row_script)?;
                        }
                        Err(row_error) if row_error.kind == ErrorKind::Cancelled => {
                            return Err(row_error);
                        }
                        Err(row_error) => {
                            progress(&skip_line(table_key, label, &row_error));
                            failures.record(label.clone(), &row_error);
                        }
                    }
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(deleted)
}

fn apply_delete_chunk(
    config: &ConnectionConfig,
    target: &ObjectPath,
    key_names: &[String],
    keys: &[(String, Vec<CellValue>)],
) -> fluxdb_core::Result<()> {
    let deletes = keys
        .iter()
        .map(|(_, values)| row_identity(key_names, values))
        .collect::<Vec<_>>();
    connector_for(config)?
        .apply_changes(&DataChangeSet {
            object: target.clone(),
            inserts: Vec::new(),
            updates: Vec::new(),
            deletes,
            insert_intents: None,
        })
        .map(|_| ())
}

/// 用键列名与键值组装行标识；DELETE 执行与脚本导出共用同一份映射。
fn row_identity(key_names: &[String], values: &[CellValue]) -> RowIdentity {
    let mut identity = std::collections::BTreeMap::new();
    for (name, value) in key_names.iter().zip(values) {
        identity.insert(name.clone(), value.clone());
    }
    RowIdentity { values: identity }
}

fn key_plan_error(reason: KeyPlanError) -> Error {
    match reason {
        KeyPlanError::NoKey => Error::new(ErrorKind::Unsupported, TableSkipReason::NoKey.message()),
        KeyPlanError::MissingColumn(column) => Error::new(
            ErrorKind::Unsupported,
            format!("比对键 {column} 在某一端不存在"),
        ),
        KeyPlanError::NoSharedColumns => Error::new(
            ErrorKind::Unsupported,
            TableSkipReason::NoSharedColumns.message(),
        ),
    }
}

/// 账本里显示的端点文本：`连接名/库[/schema]`。只用于回看，不参与连接解析。
fn endpoint_label(config: &ConnectionConfig, endpoint: &SyncEndpoint) -> String {
    match endpoint.schema() {
        Some(schema) => format!("{}/{}/{}", config.name, endpoint.database.trim(), schema),
        None => format!("{}/{}", config.name, endpoint.database.trim()),
    }
}

/// 续传签名：两端端点、表清单与除续传本身外的全部选项。
///
/// 用 `Debug` 输出即可：结构体字段固定、集合都是 `BTreeMap`/`Vec`，同输入必得同结果。
fn data_sync_signature(request: &DataSyncRequest) -> String {
    let mut options = request.options.clone();
    options.resume_from_run = None;
    format!(
        "{:?}|{:?}|{:?}|{:?}",
        request.source, request.target, request.tables, options
    )
}

fn selected_tables(tables: &[String]) -> BTreeSet<String> {
    tables
        .iter()
        .map(|table| table.trim().to_string())
        .filter(|table| !table.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{create_sqlite, execute};
    use super::*;
    use fluxdb_core::{ConnectionConfig, ConnectionId};

    fn column(name: &str, type_name: &str, primary_key: bool) -> CompletionColumn {
        CompletionColumn {
            database: Some("main".into()),
            schema: None,
            table: "items".into(),
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable: !primary_key,
            primary_key,
            comment: None,
            stable: None,
        }
    }

    #[test]
    fn chunk_size_shrinks_only_when_rows_themselves_are_slow() {
        // 4096 行花 3s：固定成本（建连/元数据）占大头，每行 0.73ms，不应减半。
        assert_eq!(next_chunk_rows(4096, Duration::from_secs(3)), 4096);
        // 4096 行花 20s：每行 4.9ms，确实慢，减半。
        assert_eq!(next_chunk_rows(4096, Duration::from_secs(20)), 2048);
        // 小块也遵守下限。
        assert_eq!(next_chunk_rows(512, Duration::from_secs(20)), 512);
        // 很快的块翻倍，上限不超过连接器单页。
        assert_eq!(next_chunk_rows(1024, Duration::from_millis(100)), 2048);
        assert_eq!(next_chunk_rows(4096, Duration::from_millis(100)), 4096);
    }

    #[test]
    fn structure_compatibility_allows_mysql_family_only() {
        use DatabaseKind::*;
        assert!(structure_kinds_compatible(MySql, TiDb));
        assert!(structure_kinds_compatible(MariaDb, MySql));
        assert!(structure_kinds_compatible(Postgres, Postgres));
        assert!(!structure_kinds_compatible(Sqlite, Postgres));
        assert!(!structure_kinds_compatible(Postgres, Kingbase));
    }

    #[test]
    fn plan_applies_column_map_and_skips() {
        let source = vec![
            "old_id".to_string(),
            "amount".to_string(),
            "note".to_string(),
        ];
        let target = vec![
            column("id", "bigint", true),
            column("amount", "text", false),
            column("note", "text", false),
        ];
        let options = DataSyncOptions {
            key_columns: vec!["old_id".into()],
            skip_columns: vec!["note".into()],
            column_map: BTreeMap::from([("old_id".to_string(), "id".to_string())]),
            ..DataSyncOptions::default()
        };
        let plan = sync_plan(&source, &target, &options, "orders").expect("plan");

        assert_eq!(plan.key_target_names, vec!["id".to_string()]);
        assert_eq!(plan.compare_pairs, vec![(1, 1)], "note 被跳过");
        assert_eq!(
            plan.target_families[0],
            super::super::column_types::ColumnFamily::Integer
        );
        assert_eq!(
            plan.target_families[1],
            super::super::column_types::ColumnFamily::Text
        );
    }

    #[test]
    fn table_column_map_overrides_global_and_is_scoped() {
        let source = vec!["old_id".to_string(), "old_name".to_string()];
        let target = vec![column("id", "bigint", true), column("name", "text", false)];
        let options = DataSyncOptions {
            column_map: BTreeMap::from([("old_id".to_string(), "wrong_id".to_string())]),
            table_column_map: BTreeMap::from([(
                "orders".to_string(),
                BTreeMap::from([
                    ("old_id".to_string(), "id".to_string()),
                    ("old_name".to_string(), "name".to_string()),
                ]),
            )]),
            ..DataSyncOptions::default()
        };

        let plan = sync_plan(&source, &target, &options, "orders").expect("plan");
        assert_eq!(plan.key_target_names, vec!["id".to_string()]);
        assert_eq!(plan.compare_pairs, vec![(1, 1)]);

        // 其它表仍只看到全局映射；全局值与目标列不匹配时应报键缺失而不是误写。
        assert!(sync_plan(&source, &target, &options, "others").is_err());
    }

    #[test]
    fn table_key_columns_override_global_keys() {
        let source = vec!["id".to_string(), "tenant_id".to_string()];
        let target = vec![
            column("id", "bigint", true),
            column("tenant_id", "text", false),
        ];
        let options = DataSyncOptions {
            key_columns: vec!["id".into()],
            table_key_columns: BTreeMap::from([(
                "orders".to_string(),
                vec!["id".to_string(), "tenant_id".to_string()],
            )]),
            ..DataSyncOptions::default()
        };

        // 表级比对键覆盖全局，复合键按给定顺序生效。
        let plan = sync_plan(&source, &target, &options, "orders").expect("plan");
        assert_eq!(
            plan.key_target_names,
            vec!["id".to_string(), "tenant_id".to_string()]
        );

        // 未单独设置的表继续沿用全局比对键。
        let plan = sync_plan(&source, &target, &options, "others").expect("plan");
        assert_eq!(plan.key_target_names, vec!["id".to_string()]);
    }

    #[test]
    fn table_skip_columns_override_global() {
        let source = vec!["id".to_string(), "note".to_string(), "amount".to_string()];
        let target = vec![
            column("id", "bigint", true),
            column("note", "text", false),
            column("amount", "text", false),
        ];
        let options = DataSyncOptions {
            skip_columns: vec!["note".into()],
            table_skip_columns: BTreeMap::from([("orders".to_string(), vec!["amount".into()])]),
            ..DataSyncOptions::default()
        };

        // 表级跳过整组覆盖全局：orders 只跳 amount，note 重新参与比对。
        let plan = sync_plan(&source, &target, &options, "orders").expect("plan");
        assert_eq!(plan.compare_pairs, vec![(1, 1)]);

        // 未单独设置的表继续跳过全局列表里的 note。
        let plan = sync_plan(&source, &target, &options, "others").expect("plan");
        assert_eq!(plan.compare_pairs, vec![(2, 2)]);
    }

    #[test]
    fn incompatible_families_block_the_table() {
        let source = vec![column("id", "bytea", false)];
        let target = vec![column("id", "integer", false)];
        let options = DataSyncOptions {
            key_columns: vec!["id".into()],
            ..DataSyncOptions::default()
        };
        let plan = sync_plan(&["id".to_string()], &target, &options, "t").expect("plan");
        let message = ensure_compatible_columns(&plan, &source, &target).expect_err("冲突");
        assert!(message.contains("id"), "{message}");
    }

    #[test]
    fn sqlite_preview_reports_insert_update_delete() {
        let stamp = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-source-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-target-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-target", &target_path);

        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, amount INTEGER)",
            ),
            (
                &source,
                "INSERT INTO items (id, name, amount) VALUES (1, 'new', 10), (2, 'same', 20), (3, 'changed', 30)",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, amount INTEGER)",
            ),
            (
                &target,
                "INSERT INTO items (id, name, amount) VALUES (2, 'same', 20), (3, 'old', 30), (4, 'gone', 40)",
            ),
        ] {
            execute(config, sql);
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_data_sync(&request, &cancel, &mut |_| {})
            .expect("preview");

        assert_eq!(preview.tables.len(), 1);
        let table = &preview.tables[0];
        assert_eq!(table.state, TableDiffState::Different, "{table:?}");
        assert_eq!(table.inserted, 1);
        assert_eq!(table.updated, 1);
        assert_eq!(table.deleted, 1);
        assert_eq!(table.unchanged, 1);
        assert_eq!(table.key_columns, vec!["id".to_string()]);
        assert!(table.error.is_none(), "{table:?}");
        assert_eq!(preview.total_changed(), 3);

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    /// 显式表配对：源表名与目标表不同名时，写入必须落到用户指定目标表。
    #[test]
    fn sqlite_explicit_table_map_writes_to_renamed_target() {
        let stamp = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-map-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-map-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-map-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-map-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'a'), (2, 'b')",
            ),
            (
                &target,
                "CREATE TABLE items_old (id INTEGER PRIMARY KEY, name TEXT)",
            ),
        ] {
            execute(config, sql);
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                insert: true,
                table_map: BTreeMap::from([("items".to_string(), "items_old".to_string())]),
                ..DataSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_data_sync(&request, &cancel, &mut |_| {})
            .expect("preview");
        assert_eq!(preview.tables.len(), 1);
        assert_eq!(preview.tables[0].inserted, 2, "{:?}", preview.tables[0]);

        controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        // 目标端另建一张同名表即可验证是否写错位置。
        execute(
            &target,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
        );
        let mut renamed_path = items_path(&target);
        renamed_path.name = "items_old".into();
        let renamed = target_rows(&target, &renamed_path);
        assert_eq!(renamed.len(), 2, "写入必须落到显式配对的目标表");
        let original = target_rows(&target, &items_path(&target));
        assert!(original.is_empty(), "不能回退写同名表：{original:?}");

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    #[test]
    fn sqlite_preview_skips_table_without_key() {
        let stamp = format!("{}-nokey", std::process::id());
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-nokey-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-nokey-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-nokey-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-nokey-target", &target_path);
        for config in [&source, &target] {
            execute(config, "CREATE TABLE logs (message TEXT)");
            execute(config, "INSERT INTO logs (message) VALUES ('x')");
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["logs".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_data_sync(&request, &cancel, &mut |_| {})
            .expect("preview");
        assert_eq!(preview.tables[0].skip, Some(TableSkipReason::NoKey));

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    fn target_rows(
        config: &ConnectionConfig,
        path: &fluxdb_core::ObjectPath,
    ) -> Vec<Vec<fluxdb_core::CellValue>> {
        connector_for(config)
            .expect("connector")
            .load_data(path, 0, 100, &[], &[])
            .expect("load target")
            .rows
            .into_iter()
            .map(|row| row.values)
            .collect()
    }

    fn items_path(config: &ConnectionConfig) -> fluxdb_core::ObjectPath {
        fluxdb_core::ObjectPath {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            name: "items".into(),
            kind: fluxdb_core::ObjectKind::Table,
        }
    }

    #[test]
    fn sqlite_execute_applies_insert_update_and_optional_delete() {
        let stamp = format!(
            "{}-exec",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-exec-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-exec-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-exec-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-exec-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'new'), (2, 'same'), (3, 'changed')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (
                &target,
                "INSERT INTO items (id, name) VALUES (2, 'same'), (3, 'old'), (4, 'gone')",
            ),
        ] {
            execute(config, sql);
        }

        let mut request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        assert_eq!(report.total_inserted(), 1);
        assert_eq!(report.total_updated(), 1);
        assert_eq!(report.total_deleted(), 0, "delete is off by default");
        assert!(report.failed_tables().is_empty(), "{report:?}");

        let rows = target_rows(&target, &items_path(&target));
        assert_eq!(rows.len(), 4, "row 4 stays without delete");
        assert!(
            rows.iter()
                .any(|row| row[0] == fluxdb_core::CellValue::I64(1))
        );
        assert!(rows.iter().any(|row| {
            row[0] == fluxdb_core::CellValue::I64(3)
                && row[1] == fluxdb_core::CellValue::Text("changed".into())
        }));

        request.options.delete = true;
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute with delete");
        assert_eq!(report.total_deleted(), 1);
        let rows = target_rows(&target, &items_path(&target));
        assert_eq!(rows.len(), 3);
        assert!(
            !rows
                .iter()
                .any(|row| row[0] == fluxdb_core::CellValue::I64(4))
        );

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    #[test]
    fn sqlite_execute_skips_conflicting_rows_or_stops_by_policy() {
        let stamp = format!(
            "{}-skiprow",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-skip-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-skip-tgt-{stamp}.db"));
        let ledger_root = std::env::temp_dir().join(format!("fluxdb-sync-skip-ledger-{stamp}"));
        let mut controller = AppController::new();
        controller.set_sync_run_storage(fluxdb_storage::FileStorage::new(&ledger_root));
        let source = create_sqlite(&mut controller, "sync-skip-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-skip-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'dup'), (2, 'dup')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, UNIQUE(name))",
            ),
        ] {
            execute(config, sql);
        }

        let mut request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        let table = &report.tables[0];
        assert_eq!(table.inserted, 1, "同唯一键的第二行应被跳过：{table:?}");
        assert_eq!(table.failed_rows, 1, "{table:?}");
        assert_eq!(table.failures.len(), 1, "{table:?}");
        assert!(table.failures[0].key.contains("id=2"), "{table:?}");
        assert!(
            table.failures[0].message.to_lowercase().contains("unique"),
            "失败原因要保留数据库原文：{table:?}"
        );
        assert!(table.error.is_none(), "跳过失败行不算表级失败：{table:?}");
        assert_eq!(target_rows(&target, &items_path(&target)).len(), 1);

        // 失败行逐条落本地账本，回看时拿到与报告一致的定位键与数据库原文。
        let run = controller
            .load_sync_runs(1)
            .into_iter()
            .next()
            .expect("账本应记录本次任务");
        let detail = controller.load_sync_run_detail(run.id).expect("账本详情");
        assert_eq!(detail.failures.len(), 1, "{detail:?}");
        assert!(detail.failures[0].row_key.contains("id=2"), "{detail:?}");
        assert!(
            detail.failures[0].message.to_lowercase().contains("unique"),
            "{detail:?}"
        );

        // 同一张表改成「停止」策略：首行写入失败即按表级失败返回。
        request.options.row_error_policy = RowErrorPolicy::Stop;
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute with stop policy");
        let table = &report.tables[0];
        assert!(table.error.is_some(), "{table:?}");
        assert_eq!(table.failed_rows, 0, "{table:?}");

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
        let _ = std::fs::remove_dir_all(ledger_root);
    }

    #[test]
    fn resume_plan_exposes_chunk_cursor_and_signature() {
        let root = std::env::temp_dir().join(format!(
            "fluxdb-sync-cursor-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let store = fluxdb_storage::FileStorage::new(&root);
        let mut controller = AppController::new();
        controller.set_sync_run_storage(store.clone());
        let run = store.begin_sync_run("s", "t", "sig-1", 1).expect("begin");
        store
            .save_sync_run_chunk(
                run,
                &fluxdb_storage::SyncRunChunkRecord {
                    table_key: "t".into(),
                    ordinal: 1,
                    key_from: None,
                    key_to: Some("42".into()),
                    key_to_value: Some(r#"{"I64":42}"#.into()),
                    inserted: 1,
                    updated: 0,
                    deleted: 0,
                    elapsed_ms: 3,
                },
                &[],
            )
            .expect("chunk");

        let plan = controller.load_sync_resume(run).expect("resume plan");
        assert_eq!(plan.signature, "sig-1");
        let table = &plan.tables["t"];
        assert!(!table.completed, "还有后续块，不能算完成");
        let cursor: fluxdb_core::CellValue =
            serde_json::from_str(table.cursor.as_deref().expect("cursor")).expect("json");
        assert_eq!(cursor, fluxdb_core::CellValue::I64(42));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sqlite_resume_skips_completed_tables_and_rejects_changed_options() {
        let stamp = format!(
            "{}-resume",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-resume-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-resume-tgt-{stamp}.db"));
        let ledger_root = std::env::temp_dir().join(format!("fluxdb-sync-resume-ledger-{stamp}"));
        let mut controller = AppController::new();
        controller.set_sync_run_storage(fluxdb_storage::FileStorage::new(&ledger_root));
        let source = create_sqlite(&mut controller, "sync-resume-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-resume-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE a (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (&source, "INSERT INTO a (id, name) VALUES (1, 'x')"),
            (
                &source,
                "CREATE TABLE b (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (&source, "INSERT INTO b (id, name) VALUES (1, 'y')"),
            (
                &target,
                "CREATE TABLE a (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (
                &target,
                "CREATE TABLE b (id INTEGER PRIMARY KEY, name TEXT)",
            ),
        ] {
            execute(config, sql);
        }

        let mut request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["a".into(), "b".into()],
            options: DataSyncOptions::default(),
        };
        // 表 b 开始后置取消位：a 已完整落账，b 中断，模拟「跑到一半被取消」。
        let cancel = AtomicBool::new(false);
        let error = controller
            .execute_data_sync(&request, &cancel, &mut |message| {
                if message == "同步 b" {
                    cancel.store(true, Ordering::Relaxed);
                }
            })
            .expect_err("应被取消");
        assert_eq!(error.kind, ErrorKind::Cancelled);
        let run_id = controller.latest_sync_run_id().expect("账本应有任务号");

        let cancel = AtomicBool::new(false);
        request.options.resume_from_run = Some(run_id);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("resume");
        assert!(report.tables[0].skipped, "表 a 上次已完成：{:?}", report);
        assert_eq!(report.tables[0].inserted, 0, "{report:?}");
        assert_eq!(report.tables[1].inserted, 1, "{report:?}");

        // 选项变化后签名不一致：拒绝续传，避免按旧游标跳过数据。
        request.options.batch_rows = 7;
        let error = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect_err("参数变化应拒绝续传");
        assert!(error.message.contains("无法续传"), "{error:?}");

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
        let _ = std::fs::remove_dir_all(ledger_root);
    }

    #[test]
    fn sqlite_execute_skips_rows_with_uncoercible_values() {
        let stamp = format!(
            "{}-coerce",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-coerce-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-coerce-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-coerce-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-coerce-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id TEXT PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES ('1', 'ok'), ('abc', 'bad')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
        ] {
            execute(config, sql);
        }

        let mut request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        let table = &report.tables[0];
        assert_eq!(table.inserted, 1, "可转换的行照常写入：{table:?}");
        assert_eq!(table.failed_rows, 1, "{table:?}");
        assert!(table.failures[0].key.contains("id=abc"), "{table:?}");
        assert!(
            table.failures[0].message.contains("不是有效的整数"),
            "{table:?}"
        );

        // 改成「停止」策略：类型转换失败按表级失败返回。
        request.options.row_error_policy = RowErrorPolicy::Stop;
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute with stop policy");
        assert!(report.tables[0].error.is_some(), "{:?}", report.tables[0]);

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    #[test]
    fn sqlite_execute_overwrite_replaces_target_rows() {
        let stamp = format!(
            "{}-overwrite",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-ow-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-ow-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-ow-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-ow-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'a'), (2, 'b')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (
                &target,
                "INSERT INTO items (id, name) VALUES (9, 'stale'), (10, 'stale')",
            ),
        ] {
            execute(config, sql);
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                overwrite: true,
                ..DataSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        assert_eq!(report.total_inserted(), 2);
        let rows = target_rows(&target, &items_path(&target));
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row[0] != fluxdb_core::CellValue::I64(9))
        );

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    /// 表级事务：中途一批失败时回滚，目标表保持写入前数据（含已清空的前置步骤）。
    #[test]
    fn sqlite_atomic_overwrite_rolls_back_whole_table_on_failure() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-atomic-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-atomic-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-atomic-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-atomic-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            // 第二行违反目标端 CHECK，用来触发写入中途失败。
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'a'), (200, 'boom')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL CHECK (id < 100))",
            ),
            (&target, "INSERT INTO items (id, name) VALUES (9, 'stale')"),
        ] {
            execute(config, sql);
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                overwrite: true,
                atomic_table: true,
                // 逐行提交，确保第一行已经写入后第二行才失败。
                batch_rows: 1,
                row_error_policy: RowErrorPolicy::Stop,
                ..DataSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        assert!(report.tables[0].error.is_some(), "{:?}", report.tables[0]);

        // 清空与第一行插入都在同一事务里，失败后应回到原始目标数据。
        let rows = target_rows(&target, &items_path(&target));
        assert_eq!(rows.len(), 1, "整表应回滚，实际保留 {rows:?}");
        assert_eq!(rows[0][0], CellValue::I64(9));

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    /// 表级事务只允许覆盖模式，其它模式提前拒绝。
    #[test]
    fn atomic_table_rejects_incremental_mode() {
        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(ConnectionId(1), "main", None),
            target: crate::SyncEndpoint::new(ConnectionId(2), "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                overwrite: false,
                atomic_table: true,
                ..DataSyncOptions::default()
            },
        };
        let controller = AppController::new();
        let cancel = AtomicBool::new(false);
        let error = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect_err("incremental atomic must be rejected");
        assert!(error.message.contains("覆盖模式"), "{error:?}");
    }

    /// 摘要计划：只在同类型库、同类字段之间启用，跨类型/跨类别一律不摘要。
    #[test]
    fn lob_digest_plan_requires_same_kind_and_matching_families() {
        let source_columns = vec![
            completion_column("id", "INTEGER", true),
            completion_column("body", "BLOB", false),
            completion_column("note", "TEXT", false),
        ];
        let target_columns = vec![
            completion_column("id", "INTEGER", true),
            completion_column("body", "BLOB", false),
            completion_column("note", "TEXT", false),
        ];
        let plan = data_diff::plan_columns(
            &["id".into(), "body".into(), "note".into()],
            &["id".into(), "body".into(), "note".into()],
            &["id".into()],
        )
        .expect("column plan");

        let same_kind = lob_digest_plan(
            DatabaseKind::Sqlite,
            DatabaseKind::Sqlite,
            &plan,
            &source_columns,
            &target_columns,
        );
        assert_eq!(same_kind.source_names, vec!["body", "note"]);
        assert_eq!(same_kind.columns.len(), 2);
        assert!(same_kind.columns[0].binary);
        assert!(!same_kind.columns[1].binary);

        let cross_kind = lob_digest_plan(
            DatabaseKind::Sqlite,
            DatabaseKind::MySql,
            &plan,
            &source_columns,
            &target_columns,
        );
        assert!(cross_kind.is_empty());

        // 一端二进制、一端文本：不摘要，避免「哈希文本 vs 正文」的假差异。
        let mixed_target = vec![
            completion_column("id", "INTEGER", true),
            completion_column("body", "TEXT", false),
            completion_column("note", "TEXT", false),
        ];
        let mixed = lob_digest_plan(
            DatabaseKind::Sqlite,
            DatabaseKind::Sqlite,
            &plan,
            &source_columns,
            &mixed_target,
        );
        assert_eq!(mixed.source_names, vec!["note"]);
    }

    fn completion_column(name: &str, type_name: &str, primary_key: bool) -> CompletionColumn {
        CompletionColumn {
            database: Some("main".into()),
            schema: None,
            table: "files".into(),
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable: !primary_key,
            primary_key,
            comment: None,
            stable: None,
        }
    }

    /// 唯一键冲突：Ignore 不写冲突行，Overwrite 按唯一键用源行覆盖。
    #[test]
    fn sqlite_insert_conflict_modes_ignore_or_overwrite_target_rows() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-conflict-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-conflict-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-conflict-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-conflict-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (&source, "CREATE UNIQUE INDEX items_name ON items(name)"),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (&target, "CREATE UNIQUE INDEX items_name ON items(name)"),
            (&source, "INSERT INTO items (id, name) VALUES (1, 'a')"),
            // 目标端已有同 name 的唯一键行，源端插入必然冲突。
            (&target, "INSERT INTO items (id, name) VALUES (2, 'a')"),
        ] {
            execute(config, sql);
        }
        let mut request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);

        request.options.insert_conflict = fluxdb_core::InsertConflictMode::Ignore;
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("ignore sync");
        assert_eq!(report.total_inserted(), 0, "{report:?}");
        assert_eq!(report.tables[0].failed_rows, 0, "{report:?}");
        let rows = |config: &ConnectionConfig| -> Vec<i64> {
            let result = connector_for(config)
                .expect("connector")
                .execute(&fluxdb_core::QueryRequest {
                    connection_id: config.id,
                    database: Some("main".into()),
                    schema: None,
                    session_id: None,
                    text: "SELECT id FROM items ORDER BY id".into(),
                    mode: fluxdb_core::QueryMode::All,
                    options: fluxdb_core::QueryExecutionOptions::default(),
                })
                .expect("query");
            result.results[0]
                .rows
                .iter()
                .map(|row| match row.values[0] {
                    CellValue::I64(value) => value,
                    ref other => panic!("unexpected id value: {other:?}"),
                })
                .collect()
        };
        assert_eq!(rows(&target), vec![2], "ignore 不应改动目标行");

        request.options.insert_conflict = fluxdb_core::InsertConflictMode::Overwrite;
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("overwrite sync");
        assert_eq!(report.total_inserted(), 1, "{report:?}");
        assert_eq!(rows(&target), vec![1], "overwrite 应把目标行改成源端行");
    }

    /// 大字段摘要：页里只带哈希，写回前按行身份重读完整值；
    /// 内容未变时必须判为一致，不能因为哈希文本 vs 正文的表示差异重复写。
    #[test]
    fn sqlite_lob_digest_sync_writes_full_values_and_detects_changes() {
        let stamp = format!(
            "{}-lob",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-lob-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-lob-tgt-{stamp}.db"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-lob-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-lob-target", &target_path);
        let ddl = "CREATE TABLE files (id INTEGER PRIMARY KEY, body BLOB, note TEXT)";
        for (config, sql) in [
            (&source, ddl),
            (&target, ddl),
            (
                &source,
                "INSERT INTO files (id, body, note) VALUES (1, zeroblob(200000), 'big')",
            ),
            (
                &source,
                "INSERT INTO files (id, body, note) VALUES (2, X'00FF10', 'small')",
            ),
            (
                &source,
                "INSERT INTO files (id, body, note) VALUES (3, NULL, 'empty')",
            ),
        ] {
            execute(config, sql);
        }
        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["files".into()],
            options: DataSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("first sync");
        assert_eq!(report.total_inserted(), 3, "{report:?}");

        let length = |sql: &str| -> i64 {
            let result = connector_for(&target)
                .expect("connector")
                .execute(&fluxdb_core::QueryRequest {
                    connection_id: target.id,
                    database: Some("main".into()),
                    schema: None,
                    session_id: None,
                    text: sql.into(),
                    mode: fluxdb_core::QueryMode::All,
                    options: fluxdb_core::QueryExecutionOptions::default(),
                })
                .expect("query");
            match result.results[0].rows[0].values[0] {
                CellValue::I64(value) => value,
                ref other => panic!("unexpected length value: {other:?}"),
            }
        };
        assert_eq!(
            length("SELECT length(body) FROM files WHERE id = 1"),
            200000
        );
        assert_eq!(length("SELECT length(body) FROM files WHERE id = 2"), 3);
        // NULL 大字段不应触发「读取完整值」，仍按 NULL 写入。
        assert_eq!(length("SELECT body IS NULL FROM files WHERE id = 3"), 1);

        // 第二次同步：摘要一致，整表不应再写任何行。
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("second sync");
        assert_eq!(report.total_inserted(), 0, "{report:?}");
        assert_eq!(report.total_updated(), 0, "{report:?}");

        // 源端内容变化后必须重新写入，并写回完整新值而不是摘要。
        execute(
            &source,
            "UPDATE files SET body = randomblob(150000) WHERE id = 1",
        );
        let report = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("third sync");
        assert_eq!(report.total_updated(), 1, "{report:?}");
        assert_eq!(
            length("SELECT length(body) FROM files WHERE id = 1"),
            150000
        );

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    /// 覆盖模式：脚本按实际执行顺序记录清空与逐行 INSERT。
    #[test]
    fn sqlite_export_script_records_overwrite_in_order() {
        let stamp = format!("{}-export-overwrite", std::process::id());
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-exp-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-exp-tgt-{stamp}.db"));
        let script_path = std::env::temp_dir().join(format!("fluxdb-sync-exp-{stamp}.sql"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-exp-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-exp-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'a'), (2, 'it''s')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (&target, "INSERT INTO items (id, name) VALUES (9, 'stale')"),
        ] {
            execute(config, sql);
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                overwrite: true,
                export_sql_path: Some(script_path.clone()),
                ..DataSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        let script = std::fs::read_to_string(&script_path).expect("script");

        assert_eq!(target_rows(&target, &items_path(&target)).len(), 2);
        let delete = script.find("DELETE FROM \"items\";").expect(&script);
        let first = script.find("VALUES (1, 'a');").expect(&script);
        let second = script.find("VALUES (2, 'it''s');").expect(&script);
        assert!(
            delete < first && first < second,
            "语句顺序必须与执行顺序一致: {script}"
        );
        assert!(
            !script.contains("stale"),
            "清空语句不应把旧行写进脚本: {script}"
        );

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
        let _ = std::fs::remove_file(script_path);
    }

    /// 增量模式：INSERT / UPDATE / DELETE 都按执行成功写入脚本。
    #[test]
    fn sqlite_export_script_records_incremental_changes() {
        let stamp = format!("{}-export-inc", std::process::id());
        let source_path = std::env::temp_dir().join(format!("fluxdb-sync-inc-src-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-sync-inc-tgt-{stamp}.db"));
        let script_path = std::env::temp_dir().join(format!("fluxdb-sync-inc-{stamp}.sql"));
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "sync-inc-source", &source_path);
        let target = create_sqlite(&mut controller, "sync-inc-target", &target_path);
        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (
                &source,
                "INSERT INTO items (id, name) VALUES (1, 'new'), (2, 'same'), (3, 'changed')",
            ),
            (
                &target,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
            ),
            (
                &target,
                "INSERT INTO items (id, name) VALUES (2, 'same'), (3, 'old'), (4, 'gone')",
            ),
        ] {
            execute(config, sql);
        }

        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(source.id, "main", None),
            target: crate::SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                insert: true,
                update: true,
                delete: true,
                export_sql_path: Some(script_path.clone()),
                ..DataSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        let script = std::fs::read_to_string(&script_path).expect("script");

        assert!(script.contains("INSERT INTO \"items\""), "{script}");
        assert!(script.contains("VALUES (1, 'new');"), "{script}");
        assert!(
            script.contains("UPDATE \"items\" SET \"name\" = 'changed' WHERE \"id\" = 3;"),
            "{script}"
        );
        assert!(
            script.contains("DELETE FROM \"items\" WHERE \"id\" = 4;"),
            "{script}"
        );
        assert!(
            !script.contains("'gone'"),
            "只记录删除条件，不记录旧值: {script}"
        );

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
        let _ = std::fs::remove_file(script_path);
    }

    /// 表级事务回滚后脚本会与目标数据不一致，后端必须拒绝组合使用。
    #[test]
    fn export_script_rejects_atomic_table() {
        let controller = AppController::new();
        let request = DataSyncRequest {
            source: crate::SyncEndpoint::new(ConnectionId(1), "main", None),
            target: crate::SyncEndpoint::new(ConnectionId(1), "main", None),
            tables: vec!["items".into()],
            options: DataSyncOptions {
                overwrite: true,
                atomic_table: true,
                export_sql_path: Some(std::env::temp_dir().join("unused.sql")),
                ..DataSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        let error = controller
            .execute_data_sync(&request, &cancel, &mut |_| {})
            .expect_err("atomic + export");
        assert!(error.message.contains("不能同时使用"), "{}", error.message);
    }
}
