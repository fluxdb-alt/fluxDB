// ER 结构快照重绑（er-design.md §5.2）。
//
// 结构刷新后，把旧关系端点/列按固定顺序重绑到新快照，逐列独立进行，**不存在按相似度
// 猜测的第 5 步**：猜错会让关系静默指向另一列，比 unresolved 让用户手工重绑严重得多。
// 本文件为纯数据重绑逻辑 + 结果枚举，均 serde、可单测，不依赖 UI / Connector。
//
// 稳定对象标识（如 PG attrelid/attnum）由宿主从数据库读取作为匹配证据；重建检测：
// 实体稳定标识变化但限定名不变 → 按限定名重绑结构，同时关系标 needs_review（提示用户
// 确认是同一表而非同名新表，不自动继承也不自动丢弃）。

// 本文件被 include! 进 core lib.rs（crate root scope）；serde 的 Serialize/Deserialize
// 与既有 include! 文件共享 crate-root 导入，不得在此重复 use（避免 E0252）。

/// 数据库稳定对象标识（仅作匹配证据，需考虑删除重建/标识复用）。
pub type ErStableId = u64;

/// 重绑用实体视图（来自结构快照，host 填充稳定标识与列稳定标识）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRebindEntity {
    pub entity_id: String,
    /// 原始限定名（PG schema.table，其余裸名），大小写/引号规则由方言处理，不统一转小写。
    pub qualified_name: String,
    pub stable_id: Option<ErStableId>,
    pub columns: Vec<ErRebindColumn>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRebindColumn {
    pub column_id: String,
    pub name: String,
    pub stable_id: Option<ErStableId>,
}

/// 重绑结果：实体是否匹配 + 每列的重绑状态。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRebindEntityOutcome {
    /// 匹配到的新实体 ID；None = 实体未找到（关系 unresolved，记录缺失端点）。
    pub matched_entity: Option<String>,
    /// 匹配到的旧实体 stable_id 是否变化（限定名相同但底层对象变了 → needs_review）。
    pub entity_needs_review: bool,
    /// 每列重绑：旧 column_id → 新状态。
    pub columns: Vec<ErRebindColumnOutcome>,
    /// 实体未找到时置 true（§5.2 第 4 步；关系 unresolved，不自动继承）。
    pub entity_unresolved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErRebindColumnOutcome {
    pub old_column_id: String,
    /// 匹配到的新 column_id；None = 实体在、列不存在（unresolved）。
    pub new_column_id: Option<String>,
    /// 列未找到（实体在、列名不存在）→ unresolved，记录缺失列（§5.2 第 3 步）。
    pub unresolved: bool,
}

/// 按 §5.2 四步把旧实体`old_entity`（及其用到的旧列`columns_used`）重绑到新快照`new_entities`。
///
/// 逐列独立判定（每步固定顺序，无第 5 步相似度猜测）：
/// 1. 实体：稳定对象标识直接重绑（且未被重建——见下面 needs_review）。
/// 2. 实体：`qualified_name` 完全一致直接重绑。
/// 3. 实体匹配后，列：稳定标识 → 列名；列名不存在 → unresolved（记录缺失列）。
/// 4. 实体限定名/稳定标识都没有 → entity_unresolved（不相似猜测）。
pub fn rebind_entity(
    old_entity: &ErRebindEntity,
    columns_used: &[String],
    new_entities: &[ErRebindEntity],
) -> ErRebindEntityOutcome {
    // 先按稳定标识，再按限定名找新实体。
    let by_stable = old_entity.stable_id.and_then(|sid| {
        new_entities.iter().find(|n| n.stable_id == Some(sid))
    });
    let by_name = new_entities
        .iter()
        .find(|n| n.qualified_name == old_entity.qualified_name);
    // 实体选择：稳定性优先；若无 stable 则用限定名。重建检测：stable 变了但限定名在 →
    // 以限定名重绑并置 needs_review（不自动继承/丢弃，§5.2 重建检测）。
    let (matched, entity_needs_review) = match (by_stable, by_name) {
        (Some(s), _) => (Some(s), false),
        (None, Some(n)) => {
            let stable_changed = old_entity.stable_id.is_some()
                && old_entity.stable_id != n.stable_id;
            (Some(n), stable_changed)
        }
        (None, None) => (None, false),
    };
    let Some(matched_entity) = matched else {
        return ErRebindEntityOutcome {
            matched_entity: None,
            entity_needs_review: false,
            columns: Vec::new(),
            entity_unresolved: true,
        };
    };
    // 逐列重绑（旧列 → 新列）。
    let mut columns = Vec::with_capacity(columns_used.len());
    for old_col in columns_used {
        // old_entity 的列稳定标识索引。
        let old_col_meta = old_entity.columns.iter().find(|c| &c.column_id == old_col);
        let new_by_stable = old_col_meta
            .and_then(|c| c.stable_id)
            .and_then(|sid| matched_entity.columns.iter().find(|n| n.stable_id == Some(sid)));
        let new_by_name = matched_entity
            .columns
            .iter()
            .find(|n| n.name == old_col_meta.map(|c| c.name.as_str()).unwrap_or(old_col.as_str()));
        let new_col = new_by_stable.or(new_by_name);
        match new_col {
            Some(nc) => columns.push(ErRebindColumnOutcome {
                old_column_id: old_col.clone(),
                new_column_id: Some(nc.column_id.clone()),
                unresolved: false,
            }),
            None => columns.push(ErRebindColumnOutcome {
                old_column_id: old_col.clone(),
                new_column_id: None,
                unresolved: true,
            }),
        }
    }
    ErRebindEntityOutcome {
        matched_entity: Some(matched_entity.entity_id.clone()),
        entity_needs_review,
        columns,
        entity_unresolved: false,
    }
}

#[cfg(test)]
mod er_rebind_tests {
    use super::*;

    fn ent(id: &str, qn: &str, stable: Option<u64>, cols: &[(&str, Option<u64>)]) -> ErRebindEntity {
        ErRebindEntity {
            entity_id: id.into(),
            qualified_name: qn.into(),
            stable_id: stable,
            columns: cols
                .iter()
                .map(|(n, s)| ErRebindColumn {
                    column_id: format!("{id}-{n}"),
                    name: n.to_string(),
                    stable_id: *s,
                })
                .collect(),
        }
    }

    #[test]
    fn table_rename_same_object_auto_rebinds_entity_and_columns() {
        // 表改名但对象未变（PG oid 不变）：稳定标识命中 → 实体自动重绑到新名，
        // 列名未变 → 逐列按名重绑到新命名空间，全部 resolved 不 unresolved。
        let old = ent("db:public:orders", "public.orders", Some(100), &[("id", None), ("customer_id", None)]);
        // 新快照：表改名 orders → sales_orders，稳定标识仍是 100，列名不变。
        let new = vec![ent(
            "db:public:sales_orders",
            "public.sales_orders",
            Some(100),
            &[("id", None), ("customer_id", None)],
        )];
        let out = rebind_entity(
            &old,
            &["db:public:orders-id".to_string(), "db:public:orders-customer_id".to_string()],
            &new,
        );
        assert!(!out.entity_unresolved);
        assert!(!out.entity_needs_review, "同对象改名不应需人工确认");
        assert_eq!(out.matched_entity, Some("db:public:sales_orders".into()));
        assert_eq!(out.columns.len(), 2);
        assert_eq!(out.columns[0].new_column_id, Some("db:public:sales_orders-id".into()));
        assert!(!out.columns[0].unresolved);
        assert_eq!(out.columns[1].new_column_id, Some("db:public:sales_orders-customer_id".into()));
    }

    #[test]
    fn no_stable_id_rename_is_unresolved_not_auto_bound() {
        // 无稳定标识（MySQL/SQLite）表改名：旧名不在新快照 → entity_unresolved，
        // 绝不按相似/位置自动接去别的表。
        let old = ent("db:main:orders", "orders", None, &[("id", None)]);
        let new = vec![ent("db:main:sales_orders", "sales_orders", None, &[("id", None)])];
        let out = rebind_entity(&old, &["db:main:orders-id".to_string()], &new);
        assert!(out.entity_unresolved, "无稳定标识改名应 unresolved，不自动绑");
        assert_eq!(out.matched_entity, None);
    }

    #[test]
    fn stable_id_matches_directly() {
        let old = ent("old", "public.orders", Some(100), &[("tenant_id", Some(11))]);
        let new = vec![ent("new", "public.orders", Some(100), &[("tenant_id", Some(11))])];
        let out = rebind_entity(&old, &["old-tenant_id".to_string()], &new);
        assert!(!out.entity_unresolved);
        assert!(!out.entity_needs_review);
        assert_eq!(out.matched_entity, Some("new".into()));
        assert_eq!(out.columns[0].new_column_id, Some("new-tenant_id".into()));
        assert!(!out.columns[0].unresolved);
    }

    #[test]
    fn name_match_when_no_stable() {
        let old = ent("old", "orders", None, &[("id", None)]);
        let new = vec![ent("new", "orders", None, &[("id", None)])];
        let out = rebind_entity(&old, &["old-id".to_string()], &new);
        assert_eq!(out.matched_entity, Some("new".into()));
        assert!(!out.columns[0].unresolved);
    }

    #[test]
    fn rebuild_detection_marks_needs_review() {
        // 限定名同但 stable 变了（删除重建的同名新表）→ 按名重绑结构，但需人工确认非自动继承。
        let old = ent("old", "public.orders", Some(50), &[("id", Some(1))]);
        let new = vec![ent("new", "public.orders", Some(999), &[("id", Some(2))])];
        let out = rebind_entity(&old, &["old-id".to_string()], &new);
        assert!(out.entity_needs_review, "重建检测：stable 变但名同 → needs_review");
        assert_eq!(out.matched_entity, Some("new".into()));
    }

    #[test]
    fn missing_entity_unresolved() {
        let old = ent("old", "public.gone", Some(7), &[("id", Some(1))]);
        let new = vec![ent("new", "public.other", Some(8), &[("id", Some(1))])];
        let out = rebind_entity(&old, &["old-id".to_string()], &new);
        assert!(out.entity_unresolved);
        assert_eq!(out.matched_entity, None);
        assert!(out.columns.is_empty());
    }

    #[test]
    fn missing_column_unresolved_not_name_guessed() {
        // 实体在、列改名 → 列 unresolved（不按相似/位置猜测到另一列，§5.2 无第5步）。
        let old = ent("old", "orders", Some(1), &[("customer_id", Some(10))]);
        let new = vec![ent(
            "new",
            "orders",
            Some(1),
            &[("buyer_id", Some(20)), ("id", Some(30))],
        )];
        let out = rebind_entity(&old, &["old-customer_id".to_string()], &new);
        assert!(!out.entity_unresolved);
        assert!(out.columns[0].unresolved, "列不存在 → unresolved，绝不静默接到 buyer_id");
        assert_eq!(out.columns[0].new_column_id, None);
    }

    #[test]
    fn column_stable_beats_name() {
        // 列改名但 stable 在 → 稳定标识重绑（第 1 步优先级高于列名）。
        let old = ent("old", "orders", Some(1), &[("customer_id", Some(10))]);
        let new = vec![ent(
            "new",
            "orders",
            Some(1),
            &[("buyer_id", Some(10)), ("id", Some(30))],
        )];
        let out = rebind_entity(&old, &["old-customer_id".to_string()], &new);
        assert!(!out.columns[0].unresolved);
        assert_eq!(out.columns[0].new_column_id, Some("new-buyer_id".into()));
    }
}
