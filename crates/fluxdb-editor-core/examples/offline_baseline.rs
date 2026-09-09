//! 离线性能场景（DM-008、DM-920~928）。
//!
//! 不启动 GPUI，覆盖大文档、局部编辑、SQL 语法、粘贴边界和 viewport 查询，
//! 输出单次基线以及重复样本的 p50/p95/p99。

use fluxdb_editor_core::{DisplayMap, EditorBuffer, Offset, Range, SoftWrap, VisualLine};
use std::time::Instant;

fn sql_document(rows: usize) -> String {
    (0..rows)
        .map(|i| format!("SELECT id, name, created_at FROM user_account WHERE id = {i};\n"))
        .collect()
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples
        .get(((samples.len().saturating_sub(1)) as f64 * p).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

fn timed<F: FnOnce()>(f: F) -> f64 {
    let start = Instant::now();
    f();
    start.elapsed().as_secs_f64() * 1e3
}

fn run_case(name: &str, text: String) {
    let snap_ms = timed(|| {
        let _ = EditorBuffer::new_from(&text).snapshot();
    });
    let buffer = EditorBuffer::new_from(&text);
    let snap = buffer.snapshot();
    let bytes = text.len();
    let lines = snap.line_count();
    let no_ms = timed(|| {
        let _ = DisplayMap::new(snap.clone(), SoftWrap::None, 80, Vec::new());
    });
    let wrap_t = Instant::now();
    let wrap_map = DisplayMap::new(snap.clone(), SoftWrap::EditorWidth, 80, Vec::new());
    let wrap_ms = wrap_t.elapsed().as_secs_f64() * 1e3;
    let view_t = Instant::now();
    let total = wrap_map.visual_row_count();
    let rows: Vec<VisualLine> = wrap_map.visual_lines(total / 2, total / 2 + 30).collect();
    let view_ms = view_t.elapsed().as_secs_f64() * 1e3;

    let mut buffer2 = buffer;
    let edit_t = Instant::now();
    let edit_offset: Offset = bytes.saturating_sub(2) as Offset;
    let change = buffer2.edit(
        Range::new(edit_offset, edit_offset),
        "x",
        edit_offset + 1,
        edit_offset + 1,
        true,
    );
    let mut map2 = wrap_map;
    map2.apply_change(buffer2.snapshot(), &change.changes[0], Vec::new());
    let edit_ms = edit_t.elapsed().as_secs_f64() * 1e3;
    println!(
        "{name:>10}: lines={lines:>7} bytes={bytes:>9} | snap={snap_ms:8.3}ms no-wrap={no_ms:8.3}ms wrap={wrap_ms:8.3}ms viewport30={view_ms:8.4}ms rows={} edit1={edit_ms:8.4}ms",
        rows.len()
    );
}

fn run_edit_samples() {
    let seed = sql_document(2_000);
    let mut samples = Vec::with_capacity(100);
    let mut state = 0x1234_5678u64;
    for i in 0..100 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut buffer = EditorBuffer::new_from(&seed);
        let snapshot = buffer.snapshot();
        let offset = (state as usize % snapshot.len().saturating_sub(1)).min(snapshot.len());
        let replacement = if i % 3 == 0 { "" } else { "x" };
        let start = offset.saturating_sub(1);
        let end = (offset + if replacement.is_empty() { 0 } else { 1 }).min(snapshot.len());
        let elapsed = timed(|| {
            let edit = buffer.edit(
                Range::new(start, end),
                replacement,
                start + replacement.len(),
                start + replacement.len(),
                true,
            );
            let mut map = DisplayMap::new(buffer.snapshot(), SoftWrap::EditorWidth, 80, Vec::new());
            if let Some(change) = edit.changes.first() {
                map.apply_change(buffer.snapshot(), change, Vec::new());
                let _ = map.visual_lines(900, 930).count();
            }
        });
        samples.push(elapsed);
    }
    println!(
        "random-edit-100: p50={:.3}ms p95={:.3}ms p99={:.3}ms",
        percentile(&mut samples, 0.50),
        percentile(&mut samples, 0.95),
        percentile(&mut samples, 0.99)
    );
}

fn run_sql_shapes() {
    let cases = [
        (
            "select",
            "SELECT u.id, COUNT(*) FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.id;",
        ),
        ("insert", "INSERT INTO users (id, name) VALUES (1, '张三');"),
        (
            "ddl",
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT); ALTER TABLE t ADD COLUMN age INT; CREATE INDEX ix_t_name ON t(name);",
        ),
        (
            "cte",
            "WITH active AS (SELECT id FROM users WHERE enabled = true) SELECT * FROM active a JOIN users u ON u.id = a.id;",
        ),
        (
            "comments",
            "-- line\n# hash\n/* block */\n// slash\nSELECT 1;",
        ),
    ];
    for (name, text) in cases {
        let snap = EditorBuffer::new_from(text).snapshot();
        let start = Instant::now();
        let map = DisplayMap::new(snap, SoftWrap::EditorWidth, 48, Vec::new());
        let rows = map.visual_row_count();
        let queried = map.visual_lines(0, rows.min(8)).count();
        println!(
            "sql-{name:>8}: rows={rows:>3} viewport={queried:>2} elapsed={:.3}ms",
            start.elapsed().as_secs_f64() * 1e3
        );
    }
}

fn run_edge_cases() {
    let create = "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\n";
    let mut empty = EditorBuffer::new_from("");
    let edit = empty.edit(Range::new(0, 0), create, create.len(), create.len(), false);
    assert_eq!(empty.snapshot().line_count(), 2, "末尾换行只能产生一个空行");
    assert!(!edit.changes.is_empty());

    let unicode = "\tSELECT 😀, 名称 FROM t WHERE x = 1;\n".repeat(128);
    let snap = EditorBuffer::new_from(&unicode).snapshot();
    let map = DisplayMap::new_with_tab_size(snap, SoftWrap::EditorWidth, 32, Vec::new(), 4);
    assert!(map.visual_lines(0, 16).count() > 0);
    let long = "SELECT ".to_owned() + &"x".repeat(100_000);
    let long_map = DisplayMap::new(
        EditorBuffer::new_from(&long).snapshot(),
        SoftWrap::EditorWidth,
        80,
        Vec::new(),
    );
    assert!(long_map.visual_row_count() > 1, "超长单行必须可换行");
    println!(
        "edge-cases: empty-paste=ok unicode-tab=ok long-line-rows={}",
        long_map.visual_row_count()
    );
}

fn main() {
    println!("machine_arch = {}", std::env::consts::ARCH);
    run_case("16k-lines", sql_document(16_000));
    run_case("1MB", sql_document(15_500));
    run_case("10MB", sql_document(155_000));
    run_edit_samples();
    run_sql_shapes();
    run_edge_cases();
}
