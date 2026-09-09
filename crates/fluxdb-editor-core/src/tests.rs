//! 内核集成测试：版本保护、旧任务结果丢弃、中文/emoji/CRLF/长文本/跨行替换。

use crate::{
    CompletionController, CompletionItem, CompletionKind, DisplayMap, EditorBuffer, Fold, Point,
    Range, Selection, SoftWrap, TextChange,
};

#[test]
fn editor_event_changed_carries_version() {
    let mut buf = EditorBuffer::new_from("hello");
    let edit = buf.edit(Range::new(5, 5), " world", 11, 11, false);
    assert_eq!(edit.changes.len(), 1);
    let change = &edit.changes[0];
    assert_eq!(change.old_range, Range::new(5, 5));
    assert_eq!(change.new_text, " world");
    assert_eq!(change.version, 1);
    assert!(!change.full_document);
    assert_eq!(buf.version(), 1);
}

#[test]
fn full_document_change_is_marked_for_host_sync() {
    let change = TextChange::full_document("select 1".to_string(), 3);
    assert!(change.full_document);
    assert_eq!(change.old_range, Range::new(0, 0));
    assert_eq!(change.new_text, "select 1");
}

#[test]
fn buffer_undo_redo_restores_selection_positions() {
    let mut buf = EditorBuffer::new_from("sel");
    let edit = buf.edit_with_selection(Range::new(0, 3), "SELECT", 6, 6, 3, 3, true);
    assert_eq!(edit.cursor, 6);
    assert_eq!(buf.undo(), Some((3, 3)));
    assert_eq!(buf.to_string(), "sel");
    assert_eq!(buf.redo(), Some((6, 6)));
    assert_eq!(buf.to_string(), "SELECT");
}

#[test]
fn old_syntax_result_cannot_overwrite_newer_text() {
    // 模拟：后台任务拿到 v1 的快照，但 buffer 已推进到 v2。
    let mut buf = EditorBuffer::new_from("select 1");
    let snapshot_v1 = buf.snapshot();
    assert_eq!(snapshot_v1.version(), 0);
    buf.edit(Range::new(0, 0), "x", 1, 1, false);
    assert_eq!(buf.version(), 1);
    // 旧快照仍读到旧文本（隔离），且版本已知；宿主据此丢弃旧任务结果。
    assert_eq!(snapshot_v1.to_string(), "select 1");
}

#[test]
fn cross_line_replace_and_line_index() {
    let mut buf = EditorBuffer::new_from("ab\ncd\nef\n");
    // 替换 "\ncd\n"（offset 2..6）为 "XY"，剩下一行 "abXYef\n"。
    buf.edit(Range::new(2, 6), "XY", 4, 4, false);
    assert_eq!(buf.to_string(), "abXYef\n");
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.offset_to_point(2), Point::new(0, 2));
    // 换行符位于 offset 6（仍属第一行）；第二行从 offset 7 开始。
    assert_eq!(buf.offset_to_point(6), Point::new(0, 6));
    assert_eq!(buf.offset_to_point(7), Point::new(1, 0));
    assert_eq!(buf.offset_to_point(4), Point::new(0, 4));
}

#[test]
fn crlf_handling_in_line_index() {
    let buf = EditorBuffer::new_from("a\r\nb\r\n");
    // 行号按 \n 计算；CR 保留在行内。
    assert_eq!(buf.line_count(), 3);
    assert_eq!(buf.offset_to_point(3), Point::new(1, 0));
}

#[test]
fn chinese_and_emoji_roundtrip() {
    let mut buf = EditorBuffer::new_from("你好😀世界");
    // 字节长度正确。
    assert_eq!(buf.len(), "你好😀世界".len());
    // UTF-16 列转换。
    let point = Point::new(0, "你好😀".len());
    assert_eq!(buf.utf16_column_at(point), 2 + 2);
    // 追加后编辑正确。
    buf.edit(
        Range::new(buf.len(), buf.len()),
        "!",
        buf.len() + 1,
        buf.len() + 1,
        false,
    );
    assert_eq!(buf.to_string(), "你好😀世界!");
}

#[test]
fn long_line_and_wrap_display() {
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("line {i} with some padding text\n"));
    }
    let snap = EditorBuffer::new_from(&text).snapshot();
    let map = DisplayMap::new(snap.clone(), SoftWrap::None, 80, Vec::new());
    // 200 内容行 + 1 末尾空行。
    assert_eq!(map.visual_row_count(), 201);
    // 折叠 start=50, end=100：折叠内部行 51..=100 共 50 行被跳过。
    let folds = vec![Fold {
        start_row: 50,
        end_row: 100,
    }];
    let map = DisplayMap::new(snap, SoftWrap::None, 80, folds);
    assert_eq!(map.visual_row_count(), 201 - 50);
}

#[test]
fn completion_controller_request_ids_monotonic() {
    let mut c = CompletionController::new();
    let a = c.new_request(0, 0, "se".into(), false);
    let b = c.new_request(0, 1, "sel".into(), false);
    assert!(b.request_id > a.request_id);
    assert_eq!(a.buffer_version, b.buffer_version);
}

#[test]
fn selection_range_normalization() {
    let sel = Selection::new(10, 3);
    assert_eq!(sel.range(), Range::new(3, 10));
    assert!(!sel.is_empty());
    let point = Selection::new(5, 5);
    assert!(point.is_empty());
}

#[test]
fn completion_item_scores_by_prefix() {
    let kw = CompletionItem::new("select", CompletionKind::Keyword);
    let tbl = CompletionItem::new("t_selection", CompletionKind::Table);
    // 都命中 "sel"，但 keyword 前缀匹配得分更高。
    let (s_kw, s_tbl) = (kw.match_score("sel"), tbl.match_score("sel"));
    assert!(s_kw.is_some() && s_tbl.is_some());
    assert!(s_kw.unwrap() > s_tbl.unwrap());
    // 不命中的返回 None。
    assert!(tbl.match_score("zzz").is_none());
}

#[test]
fn profile_auto_completion_uses_trigger_chars_and_prefix() {
    let mut profile = crate::EditorProfile::default();
    profile.completion_trigger = crate::CompletionTrigger::Auto;
    profile.completion_trigger_chars = vec!['.', ' '];

    let typed = TextChange::new(Range::new(3, 3), "s".into(), 1);
    assert!(profile.should_auto_complete_after_edit(&typed, 1, true));

    let dot = TextChange::new(Range::new(3, 3), ".".into(), 2);
    assert!(profile.should_auto_complete_after_edit(&dot, 0, false));

    let deleted = TextChange::new(Range::new(2, 3), String::new(), 3);
    assert!(!profile.should_auto_complete_after_edit(&deleted, 0, false));

    let pasted = TextChange::new(Range::new(3, 3), "select".into(), 4);
    assert!(!profile.should_auto_complete_after_edit(&pasted, 6, true));
}

#[test]
fn profile_auto_completion_honors_min_prefix() {
    let mut profile = crate::EditorProfile::default();
    profile.completion_trigger = crate::CompletionTrigger::Auto;
    profile.completion_min_prefix = 2;

    let typed = TextChange::new(Range::new(0, 0), "s".into(), 1);
    assert!(!profile.should_auto_complete_after_edit(&typed, 1, true));
    assert!(profile.should_auto_complete_after_edit(&typed, 2, true));
}
