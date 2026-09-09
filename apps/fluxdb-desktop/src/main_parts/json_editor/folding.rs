// json_editor/folding.rs —— 折叠路径编码与折叠状态保留策略。
//
// 折叠状态以「路径集合」记录（见 `JsonFoldPath`）；文本重新解析后，仅保留仍然存在的路径，
// 已不存在的路径（例如字段被删除）自动丢弃，避免渲染异常。


/// 把折叠路径编码为稳定字符串，作为 `BTreeSet` 的成员。
///
/// object 层用 `o:<key>`、array 层用 `i:<index>`，以 `\u{1f}`（单元分隔符）连接，
/// 避免 `:` 或 `/` 出现在 key 里时产生歧义。
pub(crate) fn json_fold_path_key(path: &[JsonKey]) -> String {
    path.iter()
        .map(JsonKey::key_id)
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

/// 折叠路径集合：按给定节点列表，保留「仍然存在」的路径。
///
/// `old` 为上一次的折叠集合；`nodes` 为重新解析后的全部可折叠节点。折叠判断以节点自身
/// 完整路径为准，子路径随父节点一起折叠不影响其存在性判断。
pub(crate) fn preserve_fold_paths(
    old: &BTreeSet<String>,
    nodes: &[JsonFoldNode],
) -> BTreeSet<String> {
    if old.is_empty() {
        return BTreeSet::new();
    }
    let alive = nodes
        .iter()
        .map(|n| json_fold_path_key(&n.path))
        .collect::<BTreeSet<_>>();
    old.iter()
        .filter(|path| alive.contains(*path))
        .cloned()
        .collect()
}

/// 折叠/展开节点：若路径已折叠则展开，否则折叠。返回更新后的集合。
pub(crate) fn toggle_fold(current: &BTreeSet<String>, path: &[JsonKey]) -> BTreeSet<String> {
    let key = json_fold_path_key(path);
    let mut next = current.clone();
    if next.contains(&key) {
        next.remove(&key);
    } else {
        next.insert(key);
    }
    next
}

#[cfg(test)]
mod json_editor_folding_tests {
    use super::*;

    #[test]
    fn fold_path_key_roundtrips_object_and_index() {
        let path = vec![JsonKey::Object("user".into()), JsonKey::Index(2)];
        let key = json_fold_path_key(&path);
        assert_eq!(key, "o:user\u{1f}i:2");
    }

    #[test]
    fn preserve_keeps_only_alive_paths() {
        let old = BTreeSet::from([
            "o:user".to_string(),
            "o:tags".to_string(),
            "o:user\u{1f}o:address".to_string(),
        ]);
        let nodes = vec![
            JsonFoldNode {
                path: vec![JsonKey::Object("user".into())],
                start_line: 0,
                end_line: 5,
                folded: false,
            },
            JsonFoldNode {
                path: vec![JsonKey::Object("address".into())],
                start_line: 1,
                end_line: 2,
                folded: false,
            },
        ];
        // `o:tags`/`o:user...address` 已不存在；注意 `o:user` 仍存在应保留。
        let kept = preserve_fold_paths(&old, &nodes);
        assert!(kept.contains("o:user"));
        assert!(!kept.contains("o:tags"));
        assert!(!kept.contains("o:user\u{1f}o:address"));
    }

    #[test]
    fn toggle_fold_adds_then_removes() {
        let empty = BTreeSet::new();
        let one = toggle_fold(&empty, &[JsonKey::Object("a".into())]);
        assert!(one.contains("o:a"));
        let back = toggle_fold(&one, &[JsonKey::Object("a".into())]);
        assert!(back.is_empty());
    }
}
