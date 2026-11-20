//! 数据同步分页预览：真实 SQLite 库上验证「一块一块推进、计数可累加、游标收敛」。

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use fluxdb_app::{
    AppCommand, AppController, DataSyncOptions, DataSyncRequest, SyncEndpoint, TableDiffState,
};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, QueryExecutionOptions, QueryMode,
    QueryRequest,
};

fn execute(config: &ConnectionConfig, text: &str) {
    let result = connector_for(config)
        .expect("SQLite connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            session_id: None,
            text: text.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        })
        .expect("SQLite execute");
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        panic!("SQLite SQL failed: {}\n{}", summary.message, text);
    }
}

fn sqlite_config(id: u64, name: &str, path: std::path::PathBuf) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(id),
        name: name.into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path,
            read_only: false,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }
}

fn temp_db(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("fluxdb-sync-paging-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(format!("{tag}.sqlite"))
}

fn seed(config: &ConnectionConfig, rows: usize, extra: &str) {
    execute(
        config,
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    );
    // `rows` 行基础数据；extra 用来制造差异或越过区间尾部的行。
    execute(
        config,
        &format!(
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < {rows})
             INSERT INTO items (id, name) SELECT n, 'v' || n FROM seq"
        ),
    );
    execute(config, extra);
}

#[test]
fn data_sync_preview_walks_pages_and_sums_to_full_diff() {
    let source = sqlite_config(9_921, "源", temp_db("src"));
    let target = sqlite_config(9_922, "目标", temp_db("dst"));
    seed(
        &source,
        1500,
        "INSERT INTO items (id, name) VALUES (1700, 'v1700')",
    );
    seed(
        &target,
        1500,
        "INSERT INTO items (id, name) VALUES (1600, 'v1600')",
    );
    // 目标端：900 缺失（新增候选）、700 内容不同（更新候选）。
    execute(&target, "DELETE FROM items WHERE id = 900");
    execute(&target, "UPDATE items SET name = 'old700' WHERE id = 700");

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
    ]));
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, "main", None),
        target: SyncEndpoint::new(target.id, "main", None),
        tables: vec!["items".into()],
        options: DataSyncOptions::default(),
    };
    let cancel = AtomicBool::new(false);

    // 首页（全量第一块）足以覆盖 1500 行，不应再有后续页。
    let first = controller
        .preview_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据预览失败");
    assert_eq!(first.tables.len(), 1);
    assert!(
        first.tables[0].page_done,
        "小表应一次比完: {:?}",
        first.tables[0]
    );

    // 强制 512 行一块，逐页推进后计数必须与整表比对结果一致。
    let mut cursor = None;
    let mut pages = 0;
    let (mut inserted, mut updated, mut deleted, mut unchanged) = (0, 0, 0, 0);
    loop {
        let page = controller
            .preview_data_sync_page(
                &request,
                "items",
                cursor.as_ref(),
                512,
                &cancel,
                &mut |_| {},
            )
            .expect("分页预览失败");
        pages += 1;
        inserted += page.inserted;
        updated += page.updated;
        deleted += page.deleted;
        unchanged += page.unchanged;
        assert_ne!(page.state, TableDiffState::Skipped, "可比对的表不应被跳过");
        if page.page_done {
            assert!(page.cursor.is_none(), "收尾页不应再给游标");
            break;
        }
        cursor = page.cursor.clone();
        assert!(cursor.is_some(), "未完成的分页必须带游标");
        assert!(pages < 20, "分页必须收敛，实际 {pages} 页");
    }

    assert!(pages > 1, "512 行一块时不应一页比完，实际 {pages} 页");
    let full = &first.tables[0];
    assert_eq!(
        (inserted, updated, deleted, unchanged),
        (full.inserted, full.updated, full.deleted, full.unchanged),
        "{pages} 页累加结果必须等于整表比对"
    );
    assert_eq!(
        (inserted, updated, deleted, unchanged),
        (2, 1, 1, 1498),
        "{pages} 页累加结果"
    );
}

/// 目标端读完（甚至为空）后，源端剩余行全部是新增：预览直接计数收尾，
/// 不再逐块读完整张源表（千万行级大表原来会卡几十分钟）。
#[test]
fn data_sync_preview_counts_tail_after_target_exhausted() {
    let source = sqlite_config(9_941, "源", temp_db("src_exhaust"));
    let target = sqlite_config(9_942, "目标", temp_db("dst_exhaust"));
    seed(&source, 1500, "SELECT 1");
    // 目标端建表后清空：目标端第一块就没有行，上界始终为空。
    seed(&target, 1, "DELETE FROM items");

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
    ]));
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, "main", None),
        target: SyncEndpoint::new(target.id, "main", None),
        tables: vec!["items".into()],
        options: DataSyncOptions::default(),
    };
    let cancel = AtomicBool::new(false);

    // 首页就能判定目标端已空：512 行一块时，剩余 988 行走 COUNT 直接计入。
    let first = controller
        .preview_data_sync_page(&request, "items", None, 512, &cancel, &mut |_| {})
        .expect("分页预览失败");
    assert!(first.page_done, "目标端读完后应一次收尾: {first:?}");
    assert!(first.cursor.is_none(), "收尾页不应再给游标");
    assert_eq!(first.inserted, 1500, "剩余行都必须计为新增");
}

/// 超过预览上限（10 万行）的表只比首块，进第四步不能被整表比对卡住。
#[test]
fn data_sync_preview_limits_big_table_to_first_chunk() {
    let source = sqlite_config(9_961, "源", temp_db("src_big"));
    let target = sqlite_config(9_962, "目标", temp_db("dst_big"));
    seed(&source, 120_000, "SELECT 1");
    seed(&target, 100_000, "SELECT 1");

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
    ]));
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, "main", None),
        target: SyncEndpoint::new(target.id, "main", None),
        tables: vec!["items".into()],
        options: DataSyncOptions::default(),
    };
    let cancel = AtomicBool::new(false);
    let preview = controller
        .preview_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据预览失败");
    let table = &preview.tables[0];
    assert!(!table.page_done, "大表预览不应一次比完: {table:?}");
    assert_eq!(table.compared_rows(), 4096, "只比首块: {table:?}");
    assert!(table.cursor.is_some(), "未比完必须带游标");
}

/// 目标端稀疏（行数远少于源端）时，块大小必须由源端决定：
/// 按目标端第 4096 行的键切块会把单块放大到几万行源数据，预览「下一页」要等很久。
#[test]
fn data_sync_preview_caps_chunk_by_source_when_target_sparse() {
    let source = sqlite_config(9_971, "源", temp_db("src_sparse"));
    let target = sqlite_config(9_972, "目标", temp_db("dst_sparse"));
    seed(&source, 120_000, "SELECT 1");
    // 目标端只留 1/20 的行：目标第 4096 行的 id 是 81920，落在源端八万多行之后。
    seed(&target, 120_000, "DELETE FROM items WHERE id % 20 != 0");

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
    ]));
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, "main", None),
        target: SyncEndpoint::new(target.id, "main", None),
        tables: vec!["items".into()],
        options: DataSyncOptions::default(),
    };
    let cancel = AtomicBool::new(false);

    let first = controller
        .preview_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据预览失败");
    let table = &first.tables[0];
    assert!(!table.page_done, "大表预览不应一次比完: {table:?}");
    assert_eq!(table.compared_rows(), 4096, "首块只应是源端一块: {table:?}");
    let cursor = table.cursor.clone().expect("未比完必须带游标");

    let page = controller
        .preview_data_sync_page(&request, "items", Some(&cursor), 4096, &cancel, &mut |_| {})
        .expect("分页预览失败");
    assert!(!page.page_done, "稀疏目标端后面还有数据: {page:?}");
    assert_eq!(page.compared_rows(), 4096, "每页同样只比源端一块: {page:?}");
    assert_ne!(page.cursor, Some(cursor), "游标必须前进");

    // 一路翻到底：所有行恰好比一次，源端独有的 11.4 万行全部计为新增。
    let (mut inserted, mut unchanged, mut pages) = (table.inserted, table.unchanged, 1);
    let mut next = page;
    while !next.page_done {
        pages += 1;
        inserted += next.inserted;
        unchanged += next.unchanged;
        let cursor = next.cursor.clone().expect("未比完必须带游标");
        next = controller
            .preview_data_sync_page(&request, "items", Some(&cursor), 4096, &cancel, &mut |_| {})
            .expect("分页预览失败");
        assert!(pages < 60, "分页必须收敛，实际 {pages} 页");
    }
    inserted += next.inserted;
    unchanged += next.unchanged;
    assert_eq!((inserted, unchanged), (114_000, 6_000), "{pages} 页累加");
}

/// 执行阶段目标端为空时仍要逐块把真实行写进去（预览的计数加速不能影响写入）。
#[test]
fn data_sync_execution_fills_empty_target() {
    let source = sqlite_config(9_951, "源", temp_db("src_fill"));
    let target = sqlite_config(9_952, "目标", temp_db("dst_fill"));
    seed(&source, 1500, "SELECT 1");
    seed(&target, 1, "DELETE FROM items");

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
    ]));
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, "main", None),
        target: SyncEndpoint::new(target.id, "main", None),
        tables: vec!["items".into()],
        options: DataSyncOptions::default(),
    };
    let cancel = AtomicBool::new(false);
    let report = controller
        .execute_data_sync(&request, &cancel, &mut |_| {})
        .expect("执行失败");
    assert!(report.failed_tables().is_empty(), "{report:?}");
    assert_eq!(report.tables[0].inserted, 1500);
    assert_eq!(read_names(&target).len(), 1500, "目标端必须写满");
}

/// 执行阶段同样逐块推进：超过一块的增量同步必须写全、删准，且计数与预览一致。
#[test]
fn data_sync_execution_streams_chunks() {
    let source = sqlite_config(9_931, "源", temp_db("exec_src"));
    let target = sqlite_config(9_932, "目标", temp_db("exec_dst"));
    seed(
        &source,
        5000,
        "INSERT INTO items (id, name) VALUES (6000, 'v6000')",
    );
    seed(
        &target,
        5000,
        "INSERT INTO items (id, name) VALUES (5500, 'v5500')",
    );
    execute(&target, "DELETE FROM items WHERE id = 900");
    execute(&target, "UPDATE items SET name = 'old700' WHERE id = 700");

    let mut controller = AppController::new();
    // 接上本地账本存储，顺带验证执行记录落盘。
    let store_root = std::env::temp_dir().join(format!(
        "fluxdb-sync-ledger-{}-{}",
        std::process::id(),
        "exec"
    ));
    let _ = std::fs::remove_dir_all(&store_root);
    let storage = fluxdb_storage::FileStorage::new(&store_root);
    controller.set_sync_run_storage(storage.clone());
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
    ]));
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, "main", None),
        target: SyncEndpoint::new(target.id, "main", None),
        tables: vec!["items".into()],
        options: DataSyncOptions {
            delete: true,
            ..DataSyncOptions::default()
        },
    };
    let cancel = AtomicBool::new(false);
    let report = controller
        .execute_data_sync(&request, &cancel, &mut |_| {})
        .expect("执行失败");
    assert!(report.failed_tables().is_empty(), "{report:?}");
    let run_id = report.run_id.expect("执行后必须有本地记录");
    let runs = storage.load_sync_runs(10).expect("读取执行记录");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        controller.load_sync_runs(20).len(),
        1,
        "记录列表与存储读取一致"
    );
    assert_eq!(
        (runs[0].inserted, runs[0].updated, runs[0].deleted),
        (2, 1, 1),
        "账本汇总必须与报告一致"
    );
    let tables = storage.load_sync_run_tables(run_id).expect("读取表汇总");
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].state, "synced");
    let chunks = storage.load_sync_run_chunks(run_id).expect("读取块明细");
    assert!(!chunks.is_empty(), "流式执行必须逐块落盘");
    let samples = storage.load_sync_run_samples(run_id).expect("读取样本");
    assert!(
        samples
            .iter()
            .any(|sample| sample.action == "update" && sample.row_key.contains("700")),
        "更新样本应能定位到具体行: {samples:?}"
    );
    // 记录查看页走的就是这条路径：控制器按 run_id 组装完整快照。
    let detail = controller
        .load_sync_run_detail(run_id)
        .expect("读取执行记录快照");
    assert_eq!(detail.run.id, run_id);
    assert_eq!(detail.tables.len(), tables.len());
    assert_eq!(detail.chunks.len(), chunks.len());
    assert_eq!(detail.samples.len(), samples.len());
    assert_eq!(
        (
            report.tables[0].inserted,
            report.tables[0].updated,
            report.tables[0].deleted
        ),
        (2, 1, 1),
        "{report:?}"
    );

    // 目标端必须与源端完全一致：多出的 5500 被删，缺失的 900 / 6000 被补。
    let rows = read_names(&target);
    assert_eq!(rows.len(), 5001, "行数");
    assert_eq!(rows.get(&900).map(String::as_str), Some("v900"));
    assert_eq!(rows.get(&700).map(String::as_str), Some("v700"));
    assert!(rows.contains_key(&6000), "源端独有的行必须写入");
    assert!(!rows.contains_key(&5500), "目标端多余的行必须删除");
}

/// 整表读取（分页导出，避免查询默认页大小把结果截在 100 行）。
fn read_names(config: &ConnectionConfig) -> BTreeMap<i64, String> {
    let mut rows = BTreeMap::new();
    connector_for(config)
        .expect("SQLite connector")
        .export_pages(
            &fluxdb_core::ObjectPath {
                connection_id: config.id,
                database: Some("main".into()),
                schema: None,
                name: "items".into(),
                kind: fluxdb_core::ObjectKind::Table,
            },
            &[],
            &[],
            &|| false,
            &mut |page| {
                for row in &page.rows {
                    let id = match &row.values[0] {
                        fluxdb_core::CellValue::I64(id) => *id,
                        other => panic!("意外的 id 类型: {other:?}"),
                    };
                    let name = match &row.values[1] {
                        fluxdb_core::CellValue::Text(name) => name.clone(),
                        other => panic!("意外的 name 类型: {other:?}"),
                    };
                    rows.insert(id, name);
                }
                true
            },
        )
        .expect("SQLite 分页读取");
    rows
}
