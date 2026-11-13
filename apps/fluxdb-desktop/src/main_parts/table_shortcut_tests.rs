mod table_shortcut_tests {
    use fluxdb_core::{ConnectionId, ObjectKind, Settings, TableShortcutGroup, TableShortcutTarget};

    use crate::{
        ShortcutConflict, TABLE_SHORTCUT_GROUP_LIMIT, TableShortcutSpecError,
        shortcut_specs_match, table_shortcut_conflict, table_shortcut_group_bindable,
        validate_table_shortcut_spec,
    };

    fn group(id: &str, keys: &str) -> TableShortcutGroup {
        TableShortcutGroup {
            id: id.into(),
            name: String::new(),
            keys: keys.into(),
            targets: vec![TableShortcutTarget {
                connection_id: ConnectionId(1),
                connection_name: "local".into(),
                database: Some("app".into()),
                schema: None,
                name: "orders".into(),
                kind: ObjectKind::Table,
            }],
        }
    }

    /// 修饰键顺序不影响比较，大小写也不影响键名。
    #[test]
    fn shortcut_specs_match_ignores_modifier_order() {
        assert!(shortcut_specs_match("cmd-shift-2", "shift-cmd-2"));
        assert!(!shortcut_specs_match("cmd-shift-2", "cmd-2"));
    }

    /// 表级快捷键必须带 ⌘/⌃/⌥，裸键与纯功能键都应被拒绝。
    #[test]
    fn table_shortcut_requires_modifier() {
        assert_eq!(
            validate_table_shortcut_spec("2"),
            Err(TableShortcutSpecError::NeedsModifier)
        );
        assert_eq!(
            validate_table_shortcut_spec("shift"),
            Err(TableShortcutSpecError::NeedsModifier)
        );
        assert!(validate_table_shortcut_spec("alt-2").is_ok());
        assert!(validate_table_shortcut_spec("cmd-shift-k").is_ok());
    }

    /// 应用快捷键与保留键位都要拦截；⌥1–⌥9 已分配给标签编号直达。
    #[test]
    fn table_shortcut_conflicts_with_app_and_reserved_keys() {
        let settings = Settings::default();
        assert!(matches!(
            table_shortcut_conflict(&settings, "cmd-s", None),
            Some(ShortcutConflict::AppShortcut(_))
        ));
        assert!(matches!(
            table_shortcut_conflict(&settings, "alt-3", None),
            Some(ShortcutConflict::Reserved(_))
        ));
        assert!(table_shortcut_conflict(&settings, "cmd-shift-9", None).is_none());
    }

    /// 同一键位重复时仅第一组可绑定；编辑自身时忽略自己的 id。
    #[test]
    fn duplicate_group_keys_bind_only_first_group() {
        let settings = Settings {
            table_shortcut_groups: vec![group("a", "cmd-shift-8"), group("b", "cmd-shift-8")],
            ..Settings::default()
        };
        assert!(table_shortcut_group_bindable(&settings, 0));
        assert!(!table_shortcut_group_bindable(&settings, 1));
        assert!(matches!(
            table_shortcut_conflict(&settings, "cmd-shift-8", Some("a")),
            Some(ShortcutConflict::TableGroup(_))
        ));
    }

    /// 组数量上限是防御性常量，避免设置页无限增长。
    #[test]
    fn group_limit_is_32() {
        assert_eq!(TABLE_SHORTCUT_GROUP_LIMIT, 32);
    }
}
