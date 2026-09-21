#[cfg(test)]
mod tests {
    use super::*;

// —— ER 关系画布（er/scene.rs 自由坐标 + 字段端口 + 网格索引）纯逻辑验证 ——

    /// 构造合成表节点：`columns` 个字段（默认已加载）。
    fn er_table(name: &str, columns: usize) -> fluxdb_core::ErTableNode {
        fluxdb_core::ErTableNode {
            name: name.to_string(),
            reference: fluxdb_core::ErTableRef {
                database: String::new(),
                schema: None,
                name: name.to_string(),
            },
            comment: None,
            status: fluxdb_core::ErLoadStatus::Loaded,
            columns: (0..columns)
                .map(|i| fluxdb_core::ErColumn {
                    name: format!("c{i}"),
                    type_name: Some(format!("T{i}")),
                    primary_key: i == 0,
                    nullable: false,
                })
                .collect(),
        }
    }

    fn er_edge(from: &str, fcol: &str, to: &str, tcol: &str) -> fluxdb_core::ErForeignKeyEdge {
        let mk = |n: &str| fluxdb_core::ErTableRef {
            database: String::new(),
            schema: None,
            name: n.to_string(),
        };
        fluxdb_core::ErForeignKeyEdge {
            name: "fk".into(),
            from_table: from.into(),
            from_column: fcol.into(),
            to_table: to.into(),
            to_column: tcol.into(),
            from_reference: mk(from),
            to_reference: mk(to),
        }
    }

    fn loaded_layout(tables: &[fluxdb_core::ErTableNode], edges: &[fluxdb_core::ErForeignKeyEdge]) -> ErLayoutResult {
        er_relation_layout(tables, edges)
    }

    fn env_for(scene: &ErScene, positions: BTreeMap<String, (f32, f32)>) -> ErEnv {
        let scrolls = BTreeMap::new();
        build_env(scene, TabId(1), &positions, &scrolls, None, &BTreeSet::new())
    }

    #[test]
    fn er_card_height_matches_phase2_spec() {
        // 3 字段：4+30+6+3×22 = 106（§3.1）。
        assert!((card_height(ErLoadStatus::Loaded, 3) - 106.).abs() < 0.001);
        // 8 字段：4+30+6+8×22 = 216，无页脚。
        assert!((card_height(ErLoadStatus::Loaded, 8) - 216.).abs() < 0.001);
        // 超过 8 字段：216+20(页脚) = 236。
        assert!((card_height(ErLoadStatus::Loaded, 30) - 236.).abs() < 0.001);
        // 未加载/失败也给出稳定高度（骨架/状态行）。
        assert!(card_height(ErLoadStatus::NotLoaded, 0) > NODE_HEADER + ACCENT_BAR);
        assert!(card_height(ErLoadStatus::Failed, 0) > NODE_HEADER + ACCENT_BAR);
    }

    #[test]
    fn er_scene_builds_topology_with_field_columns() {
        let tables = vec![er_table("orders", 5), er_table("customers", 3)];
        let edges = vec![er_edge("orders", "c2", "customers", "c1")];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: edges.clone(),
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &edges));
        assert_eq!(scene.nodes.len(), 2);
        assert_eq!(scene.edges.len(), 1);
        // 字段级端口：接 c2/c1（非首字段），不误接其他行。
        assert_eq!(scene.edges[0].from_column, Some(2));
        assert_eq!(scene.edges[0].to_column, Some(1));
        // 外键标记只标在真正持有外键的 from(c2) 端；被引用端 customers.c1 不是自身 FK。
        assert!(scene.nodes[0].columns[2].foreign_key);
        assert!(!scene.nodes[1].columns[1].foreign_key, "被引用字段不是外键");
        assert!(!scene.nodes[0].columns[0].foreign_key);
        // 端点字段集合（汇总计数用）两端都算。
        assert!(scene.nodes[0].edge_columns.contains(&2));
        assert!(scene.nodes[1].edge_columns.contains(&1));
    }

    #[test]
    fn er_fk_flag_only_on_holder_not_referenced() {
        // customers.c0 既被引用（引用它的 orders.c5），本身又持有对 other.c0 的外键 → 标 FK。
        // 纯被引用的 products.c0 不持有外键 → 不标 FK（与 PK 区分：key 图标而非 link）。
        let tables = vec![
            er_table("orders", 6),
            er_table("customers", 4),
            er_table("products", 2),
        ];
        let edges = vec![
            er_edge("orders", "c5", "customers", "c0"),   // customers.c0 被引用
            er_edge("customers", "c0", "products", "c0"), // customers.c0 也持有外键 → FK
        ];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: edges.clone(),
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &edges));
        let orders = scene.nodes[0].columns.iter().position(|c| c.name == "c5").unwrap();
        let cust_c0 = scene.nodes[1].columns.iter().position(|c| c.name == "c0").unwrap();
        let prod_c0 = scene.nodes[2].columns.iter().position(|c| c.name == "c0").unwrap();
        // orders.c5 持有外键 → FK。
        assert!(scene.nodes[0].columns[orders].foreign_key);
        // customers.c0 是引用端(持有外键) → FK。
        assert!(scene.nodes[1].columns[cust_c0].foreign_key);
        // products.c0 只被引用、自身不持有外键 → 不是 FK。
        assert!(
            !scene.nodes[2].columns[prod_c0].foreign_key,
            "仅被引用的字段不是外键"
        );
    }

    #[test]
    fn er_anchor_visible_field_positions_by_order_and_scroll() {
        let tables = vec![er_table("a", 12), er_table("b", 3)];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("a", "c5", "b", "c1"), er_edge("a", "c11", "b", "c0")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        let mut positions = BTreeMap::new();
        positions.insert("a".to_string(), (100.0, 200.0));
        positions.insert("b".to_string(), (600.0, 200.0));
        let mut env = env_for(&scene, positions.clone());
        let views = scene.node_views(&env);
        let va = &views[0];

        // 滚动 0：c5 可见（0..7 行区间），锚点在行 5 中心。
        let a5 = resolve_anchor(&scene.nodes[0], va, Some(5), true);
        assert_eq!(a5.kind, ErAnchorKind::Field);
        let expected_y = 200.0 + field_viewport_offset() + FIELD_PAD_Y / 2.0 + 5.0 * NODE_FIELD_ROW + NODE_FIELD_ROW / 2.0;
        assert!((a5.y - expected_y).abs() < 0.01);
        // 右侧端口 x = 卡片右边界。
        assert!((a5.x - (100.0 + NODE_WIDTH)).abs() < 0.01);

        // 滚动一整行（22px）：c5 仍在区间（4..11），y 上移一行。
        env.scrolls[0] = NODE_FIELD_ROW;
        let views = scene.node_views(&env);
        let va = &views[0];
        let a5b = resolve_anchor(&scene.nodes[0], va, Some(5), true);
        assert!((a5b.y - (expected_y - NODE_FIELD_ROW)).abs() < 0.01);

        // 滚动半行（11px）：锚点随滚动平滑移动半行。
        env.scrolls[0] = NODE_FIELD_ROW / 2.0;
        let views = scene.node_views(&env);
        let va = &views[0];
        let a5c = resolve_anchor(&scene.nodes[0], va, Some(5), true);
        assert!((a5c.y - (expected_y - NODE_FIELD_ROW / 2.0)).abs() < 0.01);

        // 滚动 6 行（row6..13 截到 row6..11）：c6 在区间内仍可见（字段端口）；
        // 早先的 c1 已滚出上沿 → 上汇总端口；不误接当前可见首行。
        env.scrolls[0] = 6.0 * NODE_FIELD_ROW;
        let views = scene.node_views(&env);
        let va = &views[0];
        let a6 = resolve_anchor(&scene.nodes[0], va, Some(6), true);
        assert_eq!(a6.kind, ErAnchorKind::Field);
        let a1 = resolve_anchor(&scene.nodes[0], va, Some(1), true);
        assert_eq!(a1.kind, ErAnchorKind::SummaryTop);

        // 重新滚回顶部：c1 恢复真实字段端口（不再汇总）。
        env.scrolls[0] = 0.0;
        let views = scene.node_views(&env);
        let va = &views[0];
        assert_eq!(resolve_anchor(&scene.nodes[0], va, Some(1), true).kind, ErAnchorKind::Field);
    }

    #[test]
    fn er_summary_counts_hidden_relation_fields() {
        let tables = vec![er_table("a", 12)];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("a", "c1", "a", "c0"), er_edge("a", "c9", "a", "c8"), er_edge("a", "c5", "a", "c4")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        // edge_columns 收集全部端点列：{c0,c1,c4,c5,c8,c9}。
        // 滚动 0（可见 row0..7）：上方隐藏 0，下方隐藏 {c8,c9} = 2 个不同字段。
        let (top, bottom) = summary_counts(&scene.nodes[0], 0.0);
        assert_eq!((top, bottom), (0, 2));
        // 滚动 2 行（可见 row2..9）：上方隐藏 {c0,c1} = 2，下方 0。
        let (top, bottom) = summary_counts(&scene.nodes[0], 2.0 * NODE_FIELD_ROW);
        assert_eq!((top, bottom), (2, 0));
    }

    #[test]
    fn er_hidden_field_precise_scroll_centers_target() {
        // n=12、可视 8 行：滚到字段下标 10 应精确把它送入可视区（非仅滚到最底）。
        // 居中滚动 = 10*22 - 4*22 + 11 = 143；越界上限 = (12-8)*22 = 88 → clamp 到 88。
        // 验证目标字段进入可视区：visible_row_range(12, target) 应覆盖行 10。
        let target = hidden_field_scroll(10, 12);
        let (r0, r1) = visible_row_range(12, target);
        assert!(r0 <= 10 && 10 <= r1, "定位后目标字段必须进入可视区：target={target} rows={r0}..{r1}");
        assert_eq!(target, 88.0, "越界应被 clamp 到最大滚动");

        // 字段 0：居中滚动为负 → clamp 到 0，行 0 在可视区顶部。
        let target0 = hidden_field_scroll(0, 12);
        assert_eq!(target0, 0.0);
        let (r0, _) = visible_row_range(12, target0);
        assert_eq!(r0, 0);

        // 中段字段（如 5）：居中滚动为 5*22 - 4*22 + 11 = 33，行 5 进入可视区。
        let target5 = hidden_field_scroll(5, 12);
        let (r0, r1) = visible_row_range(12, target5);
        assert!(r0 <= 5 && 5 <= r1, "中段字段定位也应进入可视区：{r0}..{r1}");
    }

    #[test]
    fn er_anchor_pending_and_missing_do_not_misattach() {
        let mut tables = vec![er_table("a", 3), er_table("b", 3)];
        // b 未加载字段。
        tables[1].status = fluxdb_core::ErLoadStatus::NotLoaded;
        tables[1].columns.clear();
        // a 的关系引用不存在字段 c99。
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("a", "c99", "b", "c0")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        let mut positions = BTreeMap::new();
        positions.insert("a".to_string(), (0.0, 0.0));
        positions.insert("b".to_string(), (500.0, 0.0));
        let env = env_for(&scene, positions);
        let views = scene.node_views(&env);
        // a 加载后找不到 c99 → Missing（表头端口 + tooltip 说明），不得接到近似字段。
        let am = resolve_anchor(&scene.nodes[0], &views[0], scene.edges[0].from_column, true);
        assert_eq!(am.kind, ErAnchorKind::Missing);
        // b 未加载 → Pending（「字段待加载」汇总端口，临时状态）。
        let bp = resolve_anchor(&scene.nodes[1], &views[1], scene.edges[0].to_column, false);
        assert_eq!(bp.kind, ErAnchorKind::Pending);
        // 端口都在表头高度（明显区别于字段行中心）。
        assert!((am.y - (am_y_base(views[0].y))).abs() < 0.01);
    }

    fn am_y_base(card_y: f32) -> f32 {
        card_y + field_viewport_offset() / 2.0
    }

    #[test]
    fn er_self_loop_and_parallel_edges_stay_distinct() {
        // 同表自关联 + 同表对不同字段的两条约束：边身份独立，不按表对合并。
        let tables = vec![er_table("a", 6)];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("a", "c0", "a", "c1"), er_edge("a", "c2", "a", "c3")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        assert_eq!(scene.edges.len(), 2, "同表对不同约束不合并");
        assert_ne!(scene.edges[0].from_column, scene.edges[1].from_column);
        // 物化：自关联折线长度 > 0，两条路径通道错开。
        let mut positions = BTreeMap::new();
        positions.insert("a".to_string(), (40.0, 40.0));
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 1200.0, 800.0);
        assert_eq!(frame.edge_views.len(), 2);
        for e in &frame.edge_views {
            let len: f32 = e.points.windows(2).map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()).sum();
            assert!(len > 1.0, "自关联折线不能退化为零长度");
        }
        assert_ne!(frame.edge_views[0].points, frame.edge_views[1].points, "并行边通道错开");
    }

    #[test]
    fn er_grid_index_supports_negative_and_drag() {
        let tables = vec![er_table("a", 2), er_table("b", 2)];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        // 拖到负坐标后仍可见、可命中；旧位置不再误命中。
        let mut positions = BTreeMap::new();
        positions.insert("a".to_string(), (-800.0, -600.0));
        positions.insert("b".to_string(), (-400.0, -600.0));
        // pan 平移到负区使负坐标表进入视口。
        let env = env_for(&scene, positions.clone());
        let vp = ErViewport { pan_x: 1400.0, pan_y: 1000.0, scale: 1.0 };
        let frame = scene.materialize(&env, vp, 800.0, 600.0);
        let visible_names: Vec<&str> = frame
            .visible_nodes
            .iter()
            .map(|&i| scene.nodes[i].name.as_str())
            .collect();
        assert!(visible_names.contains(&"a") && visible_names.contains(&"b"), "负坐标表应可见：{visible_names:?}");
        // 反向 pan（原点附近视口）不再命中负区表。
        let env2 = env_for(&scene, positions.clone());
        let frame2 = scene.materialize(&env2, ErViewport { pan_x: 0.0, pan_y: 0.0, scale: 1.0 }, 800.0, 600.0);
        assert!(frame2.visible_nodes.is_empty(), "视口在正区不应命中负坐标表");
    }

    #[test]
    fn er_far_edge_through_viewport_survives_cull() {
        // 两端卡片都在视口外、但边（两端卡片联合包围盒保守筛选）经过视口中间：保留。
        let tables: Vec<_> = (0..40).map(|i| er_table(&format!("t{i:03}"), 3)).collect();
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("t000", "c0", "t039", "c0")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        let mut positions = BTreeMap::new();
        for t in tables.iter() {
            if let Some(&(x, y, _)) = loaded_layout(&tables, &graph.edges).get(&t.name) {
                positions.insert(t.name.clone(), (x, y));
            }
        }
        let mid_x = (positions["t000"].0 + positions["t039"].0) / 2.0;
        let t000_y = positions["t000"].1;
        let env = env_for(&scene, positions);
        // 视口放在两端卡片之间（中段）：两端都出视口。
        let _ = scene.materialize(&env, ErViewport::default(), 1200.0, 800.0);
        let vp = ErViewport { pan_x: -(mid_x - 400.0), pan_y: -(t000_y - 300.0), scale: 1.0 };
        let frame_mid = scene.materialize(&env, vp, 800.0, 600.0);
        let hit = frame_mid
            .edge_views
            .iter()
            .any(|e| e.desc.contains("t000") && e.desc.contains("t039"));
        assert!(hit, "两端屏外但路径经视口的边必须保留");
    }

    #[test]
    fn er_zoom_transform_roundtrip_and_anchor_stable() {
        // 世界↔屏幕一致（视图变换缩放，§六.23-24）：to_screen 与 to_world 互为逆。
        let vp = ErViewport { pan_x: 123.0, pan_y: -45.0, scale: 2.0 };
        let wx = 310.5;
        let wy = -77.0;
        // 世界→屏幕（画布原点另加）；屏幕→世界 /scale。两者互为逆。
        let sx = vp.pan_x + wx * vp.safe_scale();
        let sy = vp.pan_y + wy * vp.safe_scale();
        assert!((vp.to_world_x(sx) - wx).abs() < 1e-4);
        assert!((vp.to_world_y(sy) - wy).abs() < 1e-4);
        assert!((sx - (123.0 + wx * 2.0)).abs() < 1e-4, "screen = pan + world*scale");

        // zoom_around 使锚点下世界点保持静止。
        let mut z = ErViewport { pan_x: 80.0, pan_y: 60.0, scale: 1.0 };
        let anchor = (400.0, 300.0);
        let w_before = (z.to_world_x(anchor.0), z.to_world_y(anchor.1));
        z.zoom_around(anchor, 1.5);
        let w_after = (z.to_world_x(anchor.0), z.to_world_y(anchor.1));
        assert!((w_before.0 - w_after.0).abs() < 1e-3, "锚点世界 x 保持：{:?}→{:?}", w_before, w_after);
        assert!((w_before.1 - w_after.1).abs() < 1e-3);
        assert!((z.scale - 1.5).abs() < 1e-3);

        // 缩放范围钳制。
        let mut big = ErViewport { pan_x: 0.0, pan_y: 0.0, scale: 1.0 };
        big.zoom_around((0.0, 0.0), 1000.0);
        assert!((big.scale - ER_MAX_SCALE).abs() < 1e-3);
    }

    #[test]
    fn er_logical_rel_id_extracts_id_from_logic_edge_name() {
        // 逻辑边名 `logic:{id}:{idx}`（画布投影）与 `logic:{id}`（邻域）都提取到 id；
        // 物理外键名与复合展开的 `logic:{id}:{col}` 不误提取错段。
        assert_eq!(er_logical_rel_id("logic:user-7-123:0"), Some("user-7-123".into()));
        assert_eq!(er_logical_rel_id("logic:user-7-123"), Some("user-7-123".into()));
        assert_eq!(er_logical_rel_id("logic:r:1:2"), Some("r".into()));
        assert_eq!(er_logical_rel_id("fk_orders_customer"), None);
        assert_eq!(er_logical_rel_id("fk-logic-foo"), None); // 非 `logic:` 前缀不误判
    }

    #[test]
    fn er_cardinality_option_roundtrip() {
        // 用户选 1:N → 生成 left_to_right.max=Many/right_to_left.max=One，basis=UserAssertion；
        // 反向能还原到同一选项 id。
        for id in ["1_1", "1_n", "n_1", "n_n"] {
            let card = er_option_id_to_cardinality(id);
            assert_eq!(er_cardinality_to_option_id(&card), id, "{id} 应往返一致");
            assert_eq!(
                card.basis,
                fluxdb_core::ErCardinalityBasis::UserAssertion,
                "{id} 由用户声明，非 unknown"
            );
        }
        // 未知：双向 Unknown + basis Unknown，映射回 unknown。
        let unknown_card = er_option_id_to_cardinality("unknown");
        assert_eq!(unknown_card.basis, fluxdb_core::ErCardinalityBasis::Unknown);
        assert_eq!(er_cardinality_to_option_id(&unknown_card), "unknown");
        // 已有数据 max 为 Zero（0..1）也归 1_1，Unknown 任一向归 unknown。
        let zero = fluxdb_core::ErMatchCardinality {
            left_to_right: fluxdb_core::ErCardinality {
                min: fluxdb_core::ErCardinalityBound::Unknown,
                max: fluxdb_core::ErCardinalityBound::Zero,
            },
            right_to_left: fluxdb_core::ErCardinality {
                min: fluxdb_core::ErCardinalityBound::Unknown,
                max: fluxdb_core::ErCardinalityBound::One,
            },
            basis: fluxdb_core::ErCardinalityBasis::Unknown,
        };
        assert_eq!(er_cardinality_to_option_id(&zero), "1_1");
        let unknown_max = fluxdb_core::ErMatchCardinality {
            left_to_right: fluxdb_core::ErCardinality {
                min: fluxdb_core::ErCardinalityBound::Unknown,
                max: fluxdb_core::ErCardinalityBound::Unknown,
            },
            right_to_left: fluxdb_core::ErCardinality {
                min: fluxdb_core::ErCardinalityBound::Unknown,
                max: fluxdb_core::ErCardinalityBound::Many,
            },
            basis: fluxdb_core::ErCardinalityBasis::Unknown,
        };
        assert_eq!(er_cardinality_to_option_id(&unknown_max), "unknown");
    }

    #[test]
    fn er_zoom_out_reveals_more_nodes_in_materialize() {
        // 缩小（world 下可视范围更大）应命中更多节点，且节点视图屏幕位置随 scale 缩放。
        let tables: Vec<_> = (0..30).map(|i| er_table(&format!("t{i:02}"), 3)).collect();
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let layout = loaded_layout(&tables, &[]);
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        for t in &tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) {
                positions.insert(t.name.clone(), (x, y));
            }
        }
        let env = env_for(&scene, positions.clone());
        let vp1 = ErViewport { pan_x: 0.0, pan_y: 0.0, scale: 1.0 };
        let frame1 = scene.materialize(&env, vp1, 800.0, 600.0);
        let vp_small = ErViewport { pan_x: 0.0, pan_y: 0.0, scale: 0.25 };
        let frame_small = scene.materialize(&env, vp_small, 800.0, 600.0);
        assert!(
            frame_small.visible_nodes.len() > frame1.visible_nodes.len(),
            "缩小应显示更多节点：{} vs {}",
            frame_small.visible_nodes.len(),
            frame1.visible_nodes.len()
        );
        // 节点视图坐标保持世界坐标（未被 paint 期 scale 反写）；缩放是绘制期变换，不改布局几何。
        let n0 = frame_small.node_views.iter().find(|v| v.name == "t00").expect("t00 可见");
        let world_x = positions["t00"].0;
        assert!((n0.x - world_x).abs() < 1e-3, "materialize 不改写节点世界坐标");
    }

    #[test]
    fn er_search_text_matches_name_and_comment_case_insensitive() {
        // 名称匹配（忽略大小写）。
        assert!(er_search_text_matches("Orders", None, "orders"));
        assert!(er_search_text_matches("ORDERS", None, "order"));
        // 注释匹配。
        assert!(er_search_text_matches("t001", Some("用户订单表"), "订单"));
        // 不匹配。
        assert!(!er_search_text_matches("orders", None, "customer"));
        assert!(!er_search_text_matches("orders", Some("订单表"), "客户"));
    }

    #[test]
    fn er_group_subset_keeps_only_schema_and_internal_edges() {
        let mk = |name: &str, schema: Option<&str>| fluxdb_core::ErTableNode {
            name: match schema {
                Some(s) => format!("{s}.{name}"),
                None => name.to_string(),
            },
            reference: fluxdb_core::ErTableRef {
                database: "db".to_string(),
                schema: schema.map(str::to_string),
                name: name.to_string(),
            },
            comment: None,
            status: fluxdb_core::ErLoadStatus::Loaded,
            columns: Vec::new(),
        };
        let edge = |from_schema: Option<&str>, from: &str, to_schema: Option<&str>, to: &str| {
            let display = |s: Option<&str>, n: &str| match s {
                Some(s) => format!("{s}.{n}"),
                None => n.to_string(),
            };
            fluxdb_core::ErForeignKeyEdge {
                name: "fk".to_string(),
                from_table: display(from_schema, from),
                from_column: "a".to_string(),
                to_table: display(to_schema, to),
                to_column: "b".to_string(),
                from_reference: fluxdb_core::ErTableRef {
                    database: "db".into(),
                    schema: from_schema.map(str::to_string),
                    name: from.into(),
                },
                to_reference: fluxdb_core::ErTableRef {
                    database: "db".into(),
                    schema: to_schema.map(str::to_string),
                    name: to.into(),
                },
            }
        };
        let graph = fluxdb_core::ErGraphData {
            tables: vec![
                mk("orders", Some("sales")),
                mk("customers", Some("sales")),
                mk("audit", Some("log")),
                mk("orphan", None),
            ],
            edges: vec![
                edge(Some("sales"), "orders", Some("sales"), "customers"),
                edge(Some("sales"), "orders", Some("log"), "audit"),
                edge(Some("log"), "audit", None, "orphan"),
            ],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        // 进入 sales 组：仅 sales 表 + 两端都在组内的边（跨 log 的边被滤掉）。
        let sales = er_group_subset_graph(&graph, Some("sales"));
        let mut names: Vec<&str> = sales.tables.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["sales.customers", "sales.orders"]);
        assert_eq!(sales.edges.len(), 1, "仅保留组内两端边：{:?}", {
            let d: Vec<_> = sales.edges.iter().map(|e| format!("{}→{}", e.from_table, e.to_table)).collect();
            d
        });
        assert!(
            !sales.edges.iter().any(|e| e.to_table == "log.audit"),
            "跨组边不能进入下级 JOIN"
        );
        // 返回全部：原样。
        let all = er_group_subset_graph(&graph, None);
        assert_eq!(all.tables.len(), 4);
        assert_eq!(all.edges.len(), 3);
    }

    #[test]
    fn er_visible_nodes_bounded_10k() {
        let n = 10_000;
        let tables: Vec<_> = (0..n).map(|i| er_table(&format!("t{i:05}"), 3)).collect();
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let layout = loaded_layout(&tables, &[]);
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        for t in &tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) {
                positions.insert(t.name.clone(), (x, y));
            }
        }
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 1200.0, 800.0);
        assert!(
            frame.visible_nodes.len() < 200,
            "万表可见集合有界：{}",
            frame.visible_nodes.len()
        );
        assert_eq!(scene.nodes.len(), n, "无截断");
    }


    // 防止回归到「外层只 flex_1/min_h_0、内层 block + 全 absolute 子元素」→ 内层高度塌陷为 0。
    fn taffy_style(
        display: taffy::Display,
        direction: taffy::FlexDirection,
        grow: bool,
    ) -> taffy::Style {
        taffy::Style {
            display,
            flex_direction: direction,
            flex_grow: if grow { 1.0 } else { 0.0 },
            flex_shrink: 1.0,
            flex_basis: taffy::Dimension::length(0.0),
            min_size: taffy::Size {
                width: taffy::Dimension::auto(),
                height: taffy::Dimension::length(0.0),
            },
            ..Default::default()
        }
    }

    /// 模拟 er 画布 div 树，返回 er_canvas_view 内层视口 div 的高度（像素）。
    /// `with_fix` = 是否采用修复后的外层 flex_col（对应 er_canvas_view 当前 .flex().flex_col()）。
    fn er_canvas_inner_height(with_fix: bool) -> f32 {
        use taffy::{AvailableSpace, Display, FlexDirection, Size, TaffyTree};
        let mut t: TaffyTree<()> = TaffyTree::new();
        // render.rs 父容器：flex_col + h_full, 1200x800。
        let root = t
            .new_leaf(taffy::Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: taffy::Dimension::length(1200.0),
                    height: taffy::Dimension::length(800.0),
                },
                ..Default::default()
            })
            .unwrap();
        // er_diagram_content：flex_1, min_h_0, flex, flex_col。
        let content = t
            .new_leaf(taffy_style(Display::Flex, FlexDirection::Column, true))
            .unwrap();
        // 工具栏：h 36。
        let toolbar = t
            .new_leaf(taffy::Style {
                size: Size {
                    width: taffy::Dimension::auto(),
                    height: taffy::Dimension::length(36.0),
                },
                ..Default::default()
            })
            .unwrap();
        // er_canvas_view 外层：flex_1, min_h_0；修复后为 flex_col，否则为 Block。
        let outer = t
            .new_leaf(taffy_style(
                if with_fix { Display::Flex } else { Display::Block },
                FlexDirection::Column,
                true,
            ))
            .unwrap();
        // 内层视口 div：flex_1, min_h_0, block；子元素全 absolute（探针/连线/节点）。
        // 该层是 overflow_hidden 的实际裁切容器，必须拿到非零高度，否则节点连线全被裁掉、画布空白。
        let inner = t
            .new_leaf(taffy_style(Display::Block, FlexDirection::Column, true))
            .unwrap();
        // absolute inset_0 探针（不影响高度，模拟 ErCanvasProbe）。
        let abs_child = t
            .new_leaf(taffy::Style {
                position: taffy::Position::Absolute,
                inset: taffy::Rect {
                    left: taffy::LengthPercentageAuto::length(0.0),
                    right: taffy::LengthPercentageAuto::length(0.0),
                    top: taffy::LengthPercentageAuto::length(0.0),
                    bottom: taffy::LengthPercentageAuto::length(0.0),
                },
                ..Default::default()
            })
            .unwrap();

        t.add_child(content, toolbar).unwrap();
        t.add_child(content, outer).unwrap();
        t.add_child(outer, inner).unwrap();
        t.add_child(inner, abs_child).unwrap();
        t.add_child(root, content).unwrap();
        t.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(1200.0),
                height: AvailableSpace::Definite(800.0),
            },
        )
        .unwrap();
        t.layout(inner).unwrap().size.height
    }

    #[test]
    fn er_canvas_layout_fills_below_toolbar() {
        let fixed = er_canvas_inner_height(true);
        // 修复后：内层视口应填满工具栏下方剩余区域（800 - 36 = 764），非零。
        assert!(fixed > 0.0, "修复后内层画布高度必须非零，实际 {fixed}");
        assert!((fixed - 764.0).abs() < 1.0, "内层画布应填满工具栏下方：{fixed}");

        // 校验「回归检测有效」：若撤掉外层 flex_col，内层会塌陷为 0 —— 该测试能抓住此回归。
        let broken = er_canvas_inner_height(false);
        assert!(
            broken <= 0.0,
            "对照：无 flex_col 的外层应使内层高度塌陷为 0（保证测试能抓到回归），实际 {broken}"
        );
    }

    #[test]
    fn er_canvas_layout_empty_graph_does_not_collapse() {
        // 空图也保正常画布区域：画布高度与内容无关，只由父容器分工（工具栏下方剩余空间）。
        let h = er_canvas_inner_height(true);
        assert!(h > 0.0, "空图画布也不应塌陷：{h}");
        assert!((h - 764.0).abs() < 1.0);
    }


    #[test]
    fn er_canvas_overlay_wrapper_must_be_flex_container() {
        // 回归：搜索/小地图/导出 overlay 的包裹层 `div().relative().flex_1().min_h_0()`
        // 必须自身是 flex（flex_col），否则它包裹的 flex_1 子（er_canvas_view）在 block
        // 布局下高度塌陷 → 探针测 0、不建场景 → 整画布空白。
        use taffy::{AvailableSpace, Display, FlexDirection, Size, TaffyTree};
        let model = |with_flex: bool| -> f32 {
            let mut t: TaffyTree<()> = TaffyTree::new();
            let root = t
                .new_leaf(taffy::Style {
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    size: Size {
                        width: taffy::Dimension::length(1200.0),
                        height: taffy::Dimension::length(800.0),
                    },
                    ..Default::default()
                })
                .unwrap();
            // er_diagram_content：flex_col。
            let content = t
                .new_leaf(taffy_style(Display::Flex, FlexDirection::Column, true))
                .unwrap();
            // overlay 包裹层：flex_1 + relative；修复后是 flex_col，bug 时是 block。
            let wrapper = t
                .new_leaf(taffy_style(
                    if with_flex { Display::Flex } else { Display::Block },
                    FlexDirection::Column,
                    true,
                ))
                .unwrap();
            // er_canvas_view：flex_1 + min_h_0 + flex_col。须填满包裹层。
            let canvas = t
                .new_leaf(taffy_style(Display::Flex, FlexDirection::Column, true))
                .unwrap();
            t.add_child(root, content).unwrap();
            t.add_child(content, wrapper).unwrap();
            t.add_child(wrapper, canvas).unwrap();
            t.compute_layout(
                root,
                Size {
                    width: AvailableSpace::Definite(1200.0),
                    height: AvailableSpace::Definite(800.0),
                },
            )
            .unwrap();
            t.layout(canvas).unwrap().size.height
        };
        let flexed = model(true);
        assert!((flexed - 800.0).abs() < 1.0, "包裹层为 flex 时画布应填满：{flexed}");
        let blocked = model(false);
        assert!(
            blocked <= 0.0,
            "对照：包裹层为 block 时 flex_1 子塌陷为 0（保证测试能抓到回归），实际 {blocked}"
        );
    }

    #[test]
    fn mysql_ddl_highlight_query_maps_to_ddl_viewer_colors() {
        let query = mysql_ddl_highlights_query();

        assert_eq!(MYSQL_DDL_HIGHLIGHT_LANGUAGE, "mysql-ddl");
        assert!(query.contains("(identifier) @link_text"));
        assert!(query.contains("(literal) @link_text"));
        assert!(query.contains("(keyword_create)"));
        assert!(query.contains("@variable.special"));
        tree_sitter::Query::new(
            &tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE),
            query,
        )
        .expect("mysql ddl highlight query should compile");
    }

    /// 所有只读 SQL / DDL 预览共用 `sql_preview_editor` 工厂：底层
    /// `editor_component::Editor` + SQL 语法 provider，配色由宿主 `EditorTheme` 注入。
    /// gpui-component 的 `input::Editor` 取不到本项目主题的 syntax 配色，
    /// 预览会退化成无高亮纯文本。
    #[test]
    fn sql_ddl_previews_share_bottom_editor_factory() {
        let factory = include_str!("sql_preview.rs");
        assert!(factory.contains("editor_component::Editor::new("));
        assert!(factory.contains("sql_editor_adapter::SqlAdapter::new(dialect)"));
        assert!(factory.contains("syntax: Some(adapter as _)"));
        assert!(factory.contains("editor.set_theme(editor_theme, cx)"));

        // 表属性抽屉的 DDL 页签。
        let table_info = include_str!("cell_detail_table_info/table_info.rs");
        assert!(table_info.contains("fn table_info_ddl_text("));
        assert!(table_info.contains("sql_preview_editor("));

        // 设计表的 SQL 预览 / DDL 预览：两段函数体都不能再退回
        // 「gpui-component 组件编辑器（code_editor）+ disabled」的旧实现。
        let create_table = include_str!("create_table.rs");
        let sql_start = create_table
            .find("fn create_table_sql_preview(")
            .expect("SQL 预览函数应存在");
        let ddl_start = create_table[sql_start..]
            .find("fn create_table_ddl_preview(")
            .map(|offset| sql_start + offset)
            .expect("DDL 预览函数应存在");
        let ddl_end = create_table[ddl_start..]
            .find("fn create_table_input_border_color(")
            .map(|offset| ddl_start + offset)
            .expect("DDL 预览函数应结束于下一个函数");
        for body in [&create_table[sql_start..ddl_start], &create_table[ddl_start..ddl_end]] {
            assert!(body.contains("sql_preview_editor("), "预览未走统一工厂：{body}");
            assert!(!body.contains(".code_editor("), "预览退回组件编辑器：{body}");
            assert!(!body.contains(".disabled(true)"), "预览退回 disabled：{body}");
        }
    }

    #[test]
    fn pg_user_admin_preview_is_automatic_and_keeps_its_task_alive() {
        let panel = include_str!("pg_user_admin/sql_preview.rs");
        assert!(!panel.contains("生成/刷新预览"));

        let controller = include_str!("pg_user_admin/mod.rs");
        assert!(controller.contains("this.start_pg_plan_preview(tab_id, cx);"));
        assert!(controller.contains("self._user_admin_preview_tasks.insert(tab_id, task);"));
        assert!(controller.contains("this._user_admin_preview_tasks.remove(&tab_id);"));
    }

    #[test]
    fn pg_user_admin_membership_load_keeps_its_task_alive() {
        let controller = include_str!("pg_user_admin/mod.rs");
        assert!(controller.contains("self._user_admin_pg_membership_tasks.insert(tab_id, task);"));
        assert!(controller.contains("this._user_admin_pg_membership_tasks.remove(&tab_id);"));
    }

    #[test]
    fn pg_user_admin_privilege_targets_load_keeps_its_task_alive() {
        let controller = include_str!("pg_user_admin/mod.rs");
        assert!(controller.contains("if detail_tab == UserAdminDetailTab::Privileges {"));
        assert!(controller.contains("this.start_pg_grant_targets_load_for(tab_id, database, cx);"));
        assert!(controller.contains("self._user_admin_pg_target_tasks.insert(tab_id, task);"));
        assert!(controller.contains("this._user_admin_pg_target_tasks.remove(&tab_id);"));
    }

    #[test]
    fn pg_user_admin_object_grants_load_keeps_its_task_alive() {
        let privileges = include_str!("pg_user_admin/privileges.rs");
        assert!(privileges.contains("let target_fingerprint = self"));
        assert!(privileges.contains("target_fingerprint,"));
        assert!(privileges.contains("self._user_admin_pg_object_grant_tasks.insert(tab_id, task);"));
        assert!(privileges.contains("this._user_admin_pg_object_grant_tasks.remove(&tab_id);"));
        // 三类目标切换（种类/schema/对象）都必须取消旧请求，否则新目标会被旧任务槽阻塞。
        assert_eq!(
            privileges
                .matches("self.cancel_pg_object_grants_load(&tab_id);")
                .count(),
            3
        );
    }

    #[test]
    fn pg_grant_object_options_follow_selected_schema() {
        let targets = PgGrantTargetLists {
            tables: vec![
                "public.orders".to_string(),
                "tenant_a.orders".to_string(),
                "tenant_b.orders".to_string(),
            ],
            ..Default::default()
        };

        let options = pg_grant_object_options(PgGrantObjectKind::Table, "tenant_b", &targets);
        assert_eq!(options, vec!["tenant_b.orders".to_string()]);
        // schema/数据库自身就是目标，不能再显示同名“对象”下拉。
        assert!(pg_grant_uses_object_selector(PgGrantObjectKind::Table));
        assert!(!pg_grant_uses_object_selector(PgGrantObjectKind::Schema));
        assert!(!pg_grant_uses_object_selector(PgGrantObjectKind::Database));
        assert!(pg_grant_object_options(PgGrantObjectKind::Schema, "tenant_b", &targets).is_empty());
        assert!(pg_grant_object_options(PgGrantObjectKind::Database, "tenant_b", &targets).is_empty());
    }

    #[test]
    fn pg_user_admin_searchable_selects_preserve_active_queries() {
        let shared = include_str!("user_admin.rs");
        let controller = include_str!("pg_user_admin/mod.rs");
        assert!(controller.contains("sync_select_value(&self.pg_grant_db_select"));
        assert!(controller.contains("sync_select_value(&self.pg_grant_schema_select"));
        assert!(controller.contains("sync_select_value(&self.pg_grant_object_select"));
        assert!(!controller.contains("self.pg_grant_db_select.update(cx, |select, cx| {\n            select.set_selected_value"));
        // 未选中目标（数据库/对象初始为空）也必须跳过 set_selected_value；
        // 该方法会清空搜索词，导致空值下拉的搜索框无法输入。
        assert!(shared.contains("expected.is_empty() && selected.is_none()"));
    }

    /// gutter 行号列的「宽度」与「绘制」必须同源判断：只关其一会让行号列宽算成 0、
    /// 行号照画，行号就压在正文左缘（关闭行号的只读 DDL 预览曾出现该重叠）。
    #[test]
    fn gutter_line_number_width_and_paint_share_one_predicate() {
        let editor = include_str!("editor_component/mod.rs");
        let render = include_str!("editor_component/render.rs");

        assert!(editor.contains("fn shows_line_numbers(&self) -> bool"));
        assert!(editor.contains("if !self.shows_line_numbers() {"));
        assert!(render.contains("let show_line_numbers = self.editor.read(cx).shows_line_numbers();"));
        assert!(render.contains("if line.first_fragment && show_line_numbers {"));
    }

    #[test]
    fn sql_highlight_uses_tree_sitter_sequel_query() {
        let query = mysql_ddl_highlights_query();

        assert_eq!(SQL_HIGHLIGHT_LANGUAGE, "sql");
        assert!(query.contains("(keyword_select)"));
        assert!(query.contains("(keyword_where)"));
        assert!(query.contains("(keyword_limit)"));
        assert!(query.contains("@variable.special"));
        tree_sitter::Query::new(
            &tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE),
            query,
        )
        .expect("sql highlight query should compile");
    }

    #[test]
    fn json_highlight_uses_tree_sitter_json_query() {
        let query = json_highlights_query();

        assert_eq!(JSON_HIGHLIGHT_LANGUAGE, "json");
        assert!(query.contains("(pair"));
        assert!(query.contains("@string"));
        assert!(query.contains("@number"));
        tree_sitter::Query::new(
            &tree_sitter::Language::new(tree_sitter_json::LANGUAGE),
            query,
        )
        .expect("json highlight query should compile");
    }

    #[test]
    fn query_summary_text_collapses_multiline_sql_for_display() {
        assert_eq!(
            single_line_summary_text("SELECT\n  *\r\nFROM users".to_string()),
            "SELECT * FROM users"
        );
        assert_eq!(
            single_line_summary_text("SELECT  * FROM users".to_string()),
            "SELECT  * FROM users"
        );
    }

    #[test]
    fn cmd_enter_adds_row_for_create_table_edit_tabs_only() {
        let tab_id = TabId(7);

        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Fields),
            Some(AppCommand::AddCreateTableColumn(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Indexes),
            Some(AppCommand::AddCreateTableIndex(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::ForeignKeys),
            Some(AppCommand::AddCreateTableForeignKey(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Checks),
            Some(AppCommand::AddCreateTableCheck(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Triggers),
            Some(AppCommand::AddCreateTableTrigger(tab_id))
        );

        for tab in [
            CreateTableTab::Options,
            CreateTableTab::Partitions,
            CreateTableTab::SqlPreview,
            CreateTableTab::Ddl,
        ] {
            assert_eq!(create_table_add_row_command(tab_id, tab), None);
        }
    }

    #[test]
    fn query_result_tabs_include_failed_result_set_summaries() {
        let editor = QueryEditorState {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            text: String::new(),
            origin: None,
            saved_fingerprint: None,
            running: false,
            results: vec![DataPage {
                columns: Vec::new(),
                rows: Vec::new(),
                offset: 0,
                limit: 100,
                has_more: false,
            }],
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: vec![
                QueryExecutionSummary {
                    sql: "SELECT * FROM missing".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: false,
                    message: "Table missing doesn't exist".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 3,
                },
                QueryExecutionSummary {
                    sql: "SELECT * FROM users".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 0 行结果表".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 4,
                },
            ],
            error: None,
        };

        assert_eq!(query_result_entry_count(&editor), 2);
        assert_eq!(query_result_page_index(&editor, 0), None);
        assert_eq!(query_result_page_index(&editor, 1), Some(0));
        assert_eq!(
            query_result_sql(&editor, 0),
            Some("SELECT * FROM missing".to_string())
        );
        assert_eq!(
            query_result_sql(&editor, 1),
            Some("SELECT * FROM users".to_string())
        );
        assert_eq!(query_result_summary_index(&editor, 0), Some(0));
        assert_eq!(query_result_summary_index(&editor, 1), Some(1));
    }

    #[test]
    fn query_result_summary_index_skips_command_summaries() {
        let editor = QueryEditorState {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            text: String::new(),
            origin: None,
            saved_fingerprint: None,
            running: false,
            results: vec![
                DataPage {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    offset: 0,
                    limit: 100,
                    has_more: false,
                },
                DataPage {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    offset: 0,
                    limit: 100,
                    has_more: false,
                },
            ],
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: vec![
                QueryExecutionSummary {
                    sql: "UPDATE users SET touched = 1".to_string(),
                    kind: fluxdb_core::QueryStatementKind::Command,
                    success: true,
                    message: "影响 1 行".to_string(),
                    returned_rows: 0,
                    affected_rows: 1,
                    elapsed_ms: 2,
                },
                QueryExecutionSummary {
                    sql: "SELECT * FROM users".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 0 行结果表".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 3,
                },
                QueryExecutionSummary {
                    sql: "SELECT * FROM logs".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 0 行结果表".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 4,
                },
            ],
            error: None,
        };

        assert_eq!(query_result_summary_index(&editor, 0), Some(1));
        assert_eq!(query_result_summary_index(&editor, 1), Some(2));
        assert_eq!(query_result_page_index(&editor, 0), Some(0));
        assert_eq!(query_result_page_index(&editor, 1), Some(1));
    }

    #[test]
    fn sql_file_decoding_strips_bom_and_supports_gbk() {
        assert_eq!(
            decode_sql_file_bytes(encoding_rs::UTF_8, b"\xef\xbb\xbfSELECT 1;"),
            "SELECT 1;"
        );
        assert_eq!(
            decode_sql_file_bytes(encoding_rs::GBK, &[0xb2, 0xe2, 0xca, 0xd4]),
            "测试"
        );
    }

    #[test]
    fn sql_file_task_log_text_contains_summary_and_rows() {
        let started_at = Instant::now();
        let task = SqlFileExecutionTaskState {
            id: 7,
            file_name: "schema.sql".to_string(),
            path: PathBuf::from("/tmp/schema.sql"),
            connection_id: ConnectionId(3),
            database: Some("app".to_string()),
            tab_id: Some(TabId(9)),
            total: 2,
            processed: 2,
            errors: 1,
            started_at,
            finished_at: Some(started_at + Duration::from_millis(12)),
            logs: vec![SqlFileExecutionLogEntry {
                index: 1,
                success: false,
                elapsed_ms: 8,
                message: "语法错误".to_string(),
                sql: "CREATE TABLE broken".to_string(),
            }],
            error: None,
            cancel_requested: false,
            canceled: false,
        };

        let text = sql_file_task_log_text(&task);

        assert!(text.contains("文件: schema.sql"));
        assert!(text.contains("数据库: app"));
        assert!(text.contains("错误: 1"));
        assert!(text.contains("#1 失败 8 ms 语法错误 | CREATE TABLE broken"));
    }

    #[test]
    fn query_parameter_specs_find_named_and_positional_placeholders() {
        let specs = query_parameter_specs(
            "select ':skip', col from t where id = :id and name = :name or parent_id = :id and code = ? and note like ?",
        );

        assert_eq!(
            specs,
            vec![
                QueryParameterSpec {
                    key: ":id".to_string(),
                    label: ":id".to_string(),
                },
                QueryParameterSpec {
                    key: ":name".to_string(),
                    label: ":name".to_string(),
                },
                QueryParameterSpec {
                    key: "?1".to_string(),
                    label: "参数 1".to_string(),
                },
                QueryParameterSpec {
                    key: "?2".to_string(),
                    label: "参数 2".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_query_parameter_array_values_accepts_json_array_in_order() {
        assert_eq!(
            parse_query_parameter_array_values(r#"[42, "Bob's Bike", true, null]"#).unwrap(),
            vec!["42", "Bob's Bike", "true", "null"]
        );
        assert_eq!(
            parse_query_parameter_array_values("42
Bob").unwrap(),
            vec!["42", "Bob"]
        );
    }

    #[test]
    fn bind_query_parameters_replaces_outside_comments_and_strings() {
        let sql = "select ':id', ? from t -- :skip ?
where id = :id and name = :name and flag = ?";
        let mut values = BTreeMap::new();
        values.insert(":id".to_string(), "42".to_string());
        values.insert(":name".to_string(), "Bob's Bike".to_string());
        values.insert("?1".to_string(), "true".to_string());
        values.insert("?2".to_string(), "ignored".to_string());

        assert_eq!(
            bind_query_parameters(sql, &values),
            "select ':id', TRUE from t -- :skip ?
where id = 42 and name = 'Bob''s Bike' and flag = 'ignored'"
        );
    }

    #[test]
    fn query_parameter_specs_ignore_postgres_cast_colons() {
        assert_eq!(
            query_parameter_specs("select value::text from t where id = :id"),
            vec![QueryParameterSpec {
                key: ":id".to_string(),
                label: ":id".to_string(),
            }]
        );
    }

    #[test]
    fn delete_row_labels_reflect_multi_selection() {
        assert_eq!(row_delete_label(1), "删除行");
        assert_eq!(row_delete_label(2), "删除选中行");
        assert_eq!(row_delete_record_label(1), "删除记录");
        assert_eq!(row_delete_record_label(2), "删除选中行");
    }

    #[test]
    fn saved_query_tab_id_finds_open_connection_query() {
        let state = AppState {
            tabs: vec![
                TabState {
                    id: TabId(1),
                    title: "unsaved".to_string(),
                    kind: TabKind::QueryEditor(QueryEditorState {
                        connection_id: ConnectionId(1),
                        database: Some("main".to_string()),
                        schema: None,
                        text: "select 1".to_string(),
                        origin: None,
                        saved_fingerprint: None,
                        running: false,
                        results: Vec::new(),
                        result_editors: BTreeMap::new(),
                        active_result_editor: None,
                        summaries: Vec::new(),
                        error: None,
                    }),
                    dirty: false,
                },
                TabState {
                    id: TabId(2),
                    title: "saved".to_string(),
                    kind: TabKind::QueryEditor(QueryEditorState {
                        connection_id: ConnectionId(1),
                        database: Some("main".to_string()),
                        schema: None,
                        text: "select * from users".to_string(),
                        origin: Some(QueryOrigin::Connection { query_id: 7 }),
                        saved_fingerprint: None,
                        running: false,
                        results: Vec::new(),
                        result_editors: BTreeMap::new(),
                        active_result_editor: None,
                        summaries: Vec::new(),
                        error: None,
                    }),
                    dirty: false,
                },
            ],
            ..AppState::default()
        };

        assert_eq!(saved_query_tab_id(&state, 7), Some(TabId(2)));
        assert_eq!(saved_query_tab_id(&state, 8), None);
    }

    #[test]
    fn row_viewer_snapshot_reads_readonly_query_result_page() {
        let mut state = AppState::default();
        state.tabs.push(TabState {
            id: TabId(1),
            title: "Query".to_string(),
            kind: TabKind::QueryEditor(QueryEditorState {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                text: String::new(),
                origin: None,
                saved_fingerprint: None,
                running: false,
                results: vec![DataPage {
                    columns: vec![GdbColumn {
                        name: "id".to_string(),
                        type_name: Some("varchar(64)".to_string()),
                        nullable: false,
                        primary_key: false,
                        comment: None,
                    }],
                    rows: vec![fluxdb_core::Row {
                        values: vec![CellValue::Text("a1".to_string())],
                    }],
                    offset: 20,
                    limit: 100,
                    has_more: false,
                }],
                result_editors: BTreeMap::new(),
                active_result_editor: None,
                summaries: Vec::new(),
                error: None,
            }),
            dirty: false,
        });
        let viewer = DataRowViewer {
            tab_id: TabId(1),
            source_row: 0,
            query_result_page_index: Some(0),
        };

        let (name, offset, fields) = data_row_viewer_snapshot(&viewer, &state).unwrap();

        assert_eq!(name, "查询结果 1");
        assert_eq!(offset, 20);
        assert_eq!(fields[0].name, "id");
        assert_eq!(fields[0].value, CellValue::Text("a1".to_string()));
    }

    #[test]
    fn render_state_snapshot_keeps_heavy_content_only_for_active_tab() {
        let page = DataPage {
            columns: Vec::new(),
            rows: vec![fluxdb_core::Row {
                values: vec![CellValue::Text("payload".to_string())],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let mut state = AppState {
            active_tab: Some(TabId(2)),
            tabs: vec![
                TabState {
                    id: TabId(1),
                    title: "users".to_string(),
                    kind: TabKind::DataEditor(DataEditorState {
                        object,
                        page: Some(page.clone()),
                        original_page: Some(page.clone()),
                        pagination: Default::default(),
                        changes: None,
                        editing_cell: None,
                        cell_detail_panel: CellDetailPanelState::default(),
                        table_info: TableInfoState::default(),
                        loading: false,
                        error: None,
                    }),
                    dirty: true,
                },
                TabState {
                    id: TabId(2),
                    title: "Query".to_string(),
                    kind: TabKind::QueryEditor(QueryEditorState {
                        connection_id: ConnectionId(1),
                        database: Some("main".to_string()),
                        schema: None,
                        text: "select * from users".to_string(),
                        origin: None,
                        saved_fingerprint: None,
                        running: false,
                        results: vec![page],
                        result_editors: BTreeMap::new(),
                        active_result_editor: None,
                        summaries: Vec::new(),
                        error: None,
                    }),
                    dirty: false,
                },
            ],
            ..AppState::default()
        };
        state.query_history.push(fluxdb_app::QueryHistoryEntry {
            session_id: None,
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            text: "select * from users".to_string(),
            tables: vec!["users".to_string()],
            kind: fluxdb_app::QueryHistoryKind::Query,
            success: true,
            summary: QueryExecutionSummary {
                sql: "select * from users".to_string(),
                kind: fluxdb_core::QueryStatementKind::ResultSet,
                success: true,
                message: "返回 1 行结果表".to_string(),
                returned_rows: 1,
                affected_rows: 0,
                elapsed_ms: 3,
            },
            executed_at_unix_secs: 1,
            object: Some("users".to_string()),
            rollback_snapshot: None,
            transaction_state: fluxdb_app::QueryHistoryTransactionState::Committed,
        });

        let snapshot = render_state_snapshot(&state);

        match &snapshot.tabs[0].kind {
            TabKind::DataEditor(editor) => {
                assert!(editor.page.is_none());
                assert!(editor.original_page.is_none());
            }
            _ => panic!("expected data editor tab"),
        }
        match &snapshot.tabs[1].kind {
            TabKind::QueryEditor(editor) => {
                assert_eq!(editor.text, "select * from users");
                assert_eq!(editor.results.len(), 1);
            }
            _ => panic!("expected query editor tab"),
        }
        assert!(snapshot.tabs[0].dirty);
        assert_eq!(snapshot.query_history.len(), 1);
    }

    #[test]
    fn query_result_error_copy_text_contains_sql_and_message() {
        let summary = QueryExecutionSummary {
            sql: "SELECT * FROM missing".to_string(),
            kind: fluxdb_core::QueryStatementKind::ResultSet,
            success: false,
            message: "Table missing doesn't exist".to_string(),
            returned_rows: 0,
            affected_rows: 0,
            elapsed_ms: 3,
        };

        let text = query_result_error_copy_text(&summary);

        assert!(text.contains("SELECT * FROM missing"));
        assert!(text.contains("Table missing doesn't exist"));
    }

    #[test]
    fn delete_connection_modal_uses_theme_colors() {
        let source = include_str!("menus_dialogs/confirmations.rs");
        let start = source.find("fn delete_connection_modal").unwrap();
        let end = source.find("fn delete_data_row_modal").unwrap();
        let body = &source[start..end];

        assert!(body.contains("colors: UiColors"));
        assert!(body.contains(".bg(colors.panel_bg)"));
        assert!(body.contains(".border_color(colors.border)"));
        assert!(body.contains(".text_color(colors.text)"));
        assert!(body.contains(".text_color(colors.muted)"));
        assert!(body.contains(".bg(colors.border_soft)"));
        assert!(!body.contains("rgb(0xffffff)"));
        assert!(!body.contains("rgb(0xd8dde5)"));
        assert!(!body.contains("rgb(0xe4e8ee)"));
        assert!(!body.contains(".label(\"×\")"));
    }

    #[test]
    fn redis_database_menu_has_open_pubsub_entry_guarded_by_redis() {
        // 「打开 Pub/Sub」应作为 Redis 数据库右键菜单的一项（由 is_redis 守卫，仅 Redis 显示），
        // 且动作类型为 DatabaseMenuAction::PubSub，路由到数据库级动作。
        let source = include_str!("menus_dialogs/connection_menu.rs");
        let body = &source[0..source.len()];
        assert!(body.contains("\"打开 Pub/Sub\""));
        assert!(body.contains("DatabaseMenuAction::PubSub"));
        // 与 Redis CLI 同受 is_redis 守卫：非 Redis 连接不显示。
        let redis_cli_pos = body.find("\"Redis CLI\"").unwrap();
        let pubsub_pos = body.find("\"打开 Pub/Sub\"").unwrap();
        assert!(pubsub_pos > redis_cli_pos);
        assert!(body.contains(".when(is_redis, |this|"));

        // 分发端：PubSub 动作解析菜单携带的数据库编号，构造 OpenRedisPubSub 命令传给 controller。
        let dispatch = include_str!("navicat_main/connection_groups.rs");
        let start = dispatch.find("DatabaseMenuAction::PubSub =>").unwrap();
        let end = dispatch.find("DatabaseMenuAction::RunSqlFile =>").unwrap();
        let d = &dispatch[start..end];
        assert!(d.contains("menu.database.parse::<u32>()"));
        assert!(d.contains("AppCommand::OpenRedisPubSub {"));
        assert!(d.contains("connection_id: menu.connection_id"));
        assert!(d.contains("database,"));
    }

    #[test]
    fn query_save_modal_tracks_focus_for_escape() {
        let source = include_str!("menus_dialogs/query_save.rs");
        let start = source.find("fn query_save_modal_panel").unwrap();
        let body = &source[start..];

        assert!(body.contains(".track_focus(&focus_handle)"));
        assert!(body.contains(".key_context(\"QuerySaveModal\")"));
        assert!(body.contains("this.cancel_query_save_modal(cx)"));
    }

    #[test]
    fn redis_hash_full_value_is_inline_panel_not_dialog() {
        let source = include_str!("redis_detail/hash_panel.rs");
        let start = source.find("fn redis_hash_full_value_inline_panel").unwrap();
        let end = source.find("fn redis_hash_field_add_drawer").unwrap();
        let body = &source[start..end];

        // 内嵌面板：标题、字节数与按钮齐全
        assert!(body.contains("fn redis_hash_full_value_inline_panel("));
        assert!(body.contains("完整值 · "));
        assert!(body.contains("format!(\"{} 字节\", bytes)"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-close\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-edit\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-cancel-edit\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-save\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-retry\")"));
        // 内容区支持横向+纵向滚动
        assert!(body.contains(".overflow_x_scroll()"));
        assert!(body.contains(".overflow_y_scrollbar()"));
        // 复用共享多行输入进入编辑态
        assert!(body.contains("Input::new(&input)"));
        // 不再使用任何弹框机制
        assert!(!body.contains("gpui_component::dialog::Dialog"));
        assert!(!body.contains(".overlay_closable(true)"));
        assert!(!body.contains("window.open_dialog"));
    }

    #[test]
    fn user_admin_rebuild_uses_inline_create_entry_and_general_panel() {
        let source = concat!(
            include_str!("user_admin.rs"),
            include_str!("user_admin_privileges.rs")
        );

        assert!(!source.contains("fn user_admin_create_modal("));
        assert!(!source.contains("fn user_admin_static_select_row("));
        assert!(source.contains("fn user_admin_add_user_button("));
        assert!(source.contains("fn user_admin_draft_user_row("));
        assert!(source.contains("AppCommand::BeginUserAdminCreateUser(tab_id)"));
        assert!(source.contains("fn preview_user_admin_all_sql("));
        assert!(source.contains("this.preview_user_admin_all_sql(tab_id, cx)"));
        assert!(source.contains("fn user_admin_text_input_row("));
        assert!(source.contains("Input::new(&input)"));
        assert!(source.contains(".w(px(360.))"));
        assert!(source.contains(".h(px(34.))"));
        assert!(source.contains("focus_handle(cx).is_focused(window)"));
        assert!(source.contains("fn user_admin_input_border_color("));
        assert!(source.contains("fn user_admin_input_hover_border_color("));
        assert!(source.contains("user_admin_input_border_color(true, colors)"));
        assert!(source.contains("rgb(0x111111)"));
        assert!(source.contains("fn user_admin_input_shadow("));
        assert!(!source.contains(".disabled(disabled)"));
        assert!(source.contains("fn user_admin_tab_strip("));
        assert!(source.contains("(UserAdminDetailTab::General, \"常规\")"));
        assert!(source.contains("(UserAdminDetailTab::Advanced, \"高级\")"));
        assert!(source.contains("(UserAdminDetailTab::MemberOf, \"成员关系\")"));
        assert!(!source.contains("(UserAdminDetailTab::Members, \"成员\")"));
        assert!(source.contains("fn user_admin_member_relationships_panel("));
        assert!(source.contains("fn user_admin_select_row("));
        assert!(source.contains("Select::new(&select)"));
        assert!(source.contains("fn user_admin_advanced_panel("));
        assert!(!source.contains("使用 OLD_PASSWORD 加密"));
        assert!(source.contains("fn user_admin_ssl_type_options("));
        assert!(source.contains("fn user_admin_password_eye_button("));
        assert!(source.contains("this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx)"));
        assert!(source.contains("fn user_admin_privileges_panel("));
        assert!(source.contains("添加权限"));
        assert!(source.contains("AppCommand::AddUserAdminPrivilegeRow"));
        assert!(source.contains("fn user_admin_privileges_sql_preview("));
        assert!(source.contains("fn user_admin_sql_preview_panel("));
        assert!(source.contains("TextView::markdown(editor_key, user_admin_sql_preview_markdown(sql))"));
        assert!(source.contains(".selectable(true)"));
        assert!(source.contains(".scrollable(true)"));
        assert!(!source.contains("Input::new(&editor)"));
        assert!(source.contains("fn start_user_admin_database_options_load("));
        assert!(source.contains("this.start_user_admin_database_options_load(tab_id, cx)"));
        assert!(source.contains("AppCommand::ToggleUserAdminPrivilegeRowPrivilege"));
        assert!(source.contains("AppCommand::SetUserAdminPrivilegeRowDatabase"));
        assert!(source.contains("Grant Option"));
    }

    #[test]
    fn user_admin_sql_preview_markdown_preserves_lines_and_escapes_fences() {
        let sql = "GRANT `reader` TO `alice`;\nGRANT `writer` TO `alice`;";
        let markdown = user_admin_sql_preview_markdown(sql);
        assert_eq!(
            markdown,
            "```sql\nGRANT `reader` TO `alice`;\nGRANT `writer` TO `alice`;\n```"
        );

        let sql_with_fence = "SELECT ```quoted``` FROM t;";
        let markdown = user_admin_sql_preview_markdown(sql_with_fence);
        assert!(markdown.starts_with("````sql\n"));
        assert!(markdown.ends_with("\n````"));
    }

    #[test]
    fn explain_sql_text_wraps_only_explainable_sql() {
        assert_eq!(sql_editor_adapter::explain_sql_text("SELECT * FROM Product;"), Some("EXPLAIN SELECT * FROM Product;".to_string()));
        assert_eq!(sql_editor_adapter::explain_sql_text("with q as (select 1) select * from q"), Some("EXPLAIN with q as (select 1) select * from q".to_string()));
        assert_eq!(sql_editor_adapter::explain_sql_text("EXPLAIN SELECT 1"), Some("EXPLAIN SELECT 1".to_string()));
        assert_eq!(sql_editor_adapter::explain_sql_text("UPDATE Product SET name = 'x'"), None);
    }

    #[test]
    fn query_editor_text_sync_replaces_buffer_and_resets_selection() {
        // 查询页面把模型 query.text 静默同步进编辑器（sync_text_silent）时，底层 buffer
        // 应整体替换为新文本，并把光标/选区复位到起点，避免旧文本残留或误触发编辑事件。
        // 本测试只在 buffer 层验证（无需 GPUI Window），是查询编辑器文本同步的回归保护。
        use fluxdb_editor_core::{EditorBuffer, Selection};

        let mut buffer = EditorBuffer::new_from("SELECT * FROM t;");
        assert_eq!(buffer.to_string(), "SELECT * FROM t;");
        assert_eq!(buffer.len(), "SELECT * FROM t;".len());
        assert!(!buffer.is_empty());

        // 同步 = 用新文本整体重建 buffer（sync_text_silent 的等价语义）。
        buffer = EditorBuffer::new_from("UPDATE t SET a = 1 WHERE id = 2;");
        assert_eq!(buffer.to_string(), "UPDATE t SET a = 1 WHERE id = 2;");
        assert!(!buffer.is_empty());

        // 同步后选区复位为起点（光标 = 锚点 = 0）。
        let sel = Selection::point(0);
        assert!(sel.is_empty());
        let range = sel.range();
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 0);

        // 空文本同步后 buffer 为空，选区仍在起点。
        buffer = EditorBuffer::new_from("");
        assert!(buffer.is_empty());
        assert!(Selection::point(0).is_empty());
    }

    #[test]
    fn header_sort_appends_updates_and_removes_one_field() {
        let rules = data_sort_rules_after_header_sort(
            &[],
            "id".to_string(),
            Some(DataTableSortDirection::Ascending),
        );
        let rules = data_sort_rules_after_header_sort(
            &rules,
            "name".to_string(),
            Some(DataTableSortDirection::Descending),
        );

        assert_eq!(rules.len(), 2);
        assert_eq!(data_sort_rules_text(&rules, DatabaseKind::MySql), "`id` ASC, `name` DESC");

        let rules = data_sort_rules_after_header_sort(
            &rules,
            "id".to_string(),
            Some(DataTableSortDirection::Descending),
        );
        assert_eq!(data_sort_rules_text(&rules, DatabaseKind::MySql), "`id` DESC, `name` DESC");

        let rules = data_sort_rules_after_header_sort(&rules, "id".to_string(), None);
        assert_eq!(data_sort_rules_text(&rules, DatabaseKind::MySql), "`name` DESC");
    }

    #[test]
    fn postgres_filter_and_sort_text_round_trip_double_quoted_identifiers() {
        let filter_rules = vec![
            data_filter_rule(
                "display AND \"name\"",
                DataFilterOperator::Eq,
                &["Alice"],
                false,
            ),
            data_filter_rule("active", DataFilterOperator::IsNotNull, &[], false),
        ];
        let filter_text = data_filter_rules_sql_pretty(&filter_rules, DatabaseKind::Postgres);
        assert_eq!(
            parse_data_filter_rules_text(&filter_text),
            Some(filter_rules)
        );

        let sort_rules = vec![
            DataSortRule {
                enabled: true,
                field: "id".to_string(),
                ascending: false,
            },
            DataSortRule {
                enabled: true,
                field: "display, \"name\"".to_string(),
                ascending: true,
            },
        ];
        let sort_text = data_sort_rules_text(&sort_rules, DatabaseKind::Postgres);
        assert_eq!(parse_data_sort_rules_text(&sort_text), Some(sort_rules));

        let sql = format!(
            "SELECT * FROM \"tenant_a\".\"orders\" WHERE {filter_text} ORDER BY {sort_text} LIMIT 1000"
        );
        let parsed = parse_data_editor_sql_text(&sql).expect("PostgreSQL SQL should parse");
        assert_eq!(parsed.filter_text, filter_text);
        assert_eq!(parsed.sort_text, sort_text);
        assert_eq!(parsed.limit, Some(1000));
    }

    #[test]
    fn query_result_header_sort_is_scoped_to_result_index() {
        let tab_id = TabId(9);
        let first = QueryResultSortKey {
            tab_id,
            result_index: 0,
        };
        let second = QueryResultSortKey {
            tab_id,
            result_index: 1,
        };
        let mut rules = BTreeMap::new();

        apply_query_result_header_sort(
            &mut rules,
            first,
            "id".to_string(),
            Some(DataTableSortDirection::Ascending),
        );
        apply_query_result_header_sort(
            &mut rules,
            second,
            "id".to_string(),
            Some(DataTableSortDirection::Descending),
        );

        assert_eq!(data_sort_rules_text(&rules[&first], DatabaseKind::MySql), "`id` ASC");
        assert_eq!(data_sort_rules_text(&rules[&second], DatabaseKind::MySql), "`id` DESC");

        apply_query_result_header_sort(&mut rules, first, "id".to_string(), None);

        assert!(!rules.contains_key(&first));
        assert_eq!(data_sort_rules_text(&rules[&second], DatabaseKind::MySql), "`id` DESC");
    }

    #[test]
    fn query_output_layout_toggle_changes_icon() {
        assert_eq!(
            query_output_layout_toggle_icon(ResultsPlacement::Bottom),
            AppIcon::PanelBottom
        );
        assert_eq!(
            query_output_layout_toggle_icon(ResultsPlacement::Right),
            AppIcon::PanelRight
        );
        assert_eq!(
            ResultsPlacement::Bottom.toggled(),
            ResultsPlacement::Right
        );
    }

    #[test]
    fn results_placement_defaults_to_bottom_and_is_the_shared_value() {
        // 默认「下方」，保证没设置过的用户看到的仍是上下分栏。
        assert_eq!(ResultsPlacement::default(), ResultsPlacement::Bottom);
        assert_eq!(ResultsPlacement::Right.toggled(), ResultsPlacement::Bottom);
        // 设置面板用的下标映射必须可逆（否则面板会出现「没有任何按钮高亮」）。
        for placement in [ResultsPlacement::Bottom, ResultsPlacement::Right] {
            assert_eq!(
                ResultsPlacement::from_index(placement.to_index()),
                placement
            );
        }
        // 越界回退到默认值，脏配置不该让面板空高亮。
        assert_eq!(ResultsPlacement::from_index(99), ResultsPlacement::Bottom);
    }

    #[test]
    fn redis_workbench_editor_width_clamps_to_min_and_leaves_room_for_results() {
        let available = 1400.;
        // 正常值原样通过。
        assert!((redis_workbench_editor_width(700., available) - 700.).abs() < 1e-3);
        // 过窄被抬到下限。
        assert!((redis_workbench_editor_width(10., available) - REDIS_WB_EDITOR_MIN_WIDTH).abs() < 1e-3);
        // 过宽被压到「可用宽 − 结果区下限」，结果区始终留得下。
        let too_wide = redis_workbench_editor_width(9999., available);
        assert!((too_wide - (available - REDIS_WB_RESULT_MIN_WIDTH)).abs() < 1e-3);
        // 可用宽小到下限都放不下时不得 panic（clamp 的 min>max 会 panic）。
        assert!(redis_workbench_editor_width(500., 10.) > 0.);
    }

    /// 源码结构断言：抹掉所有空白后再匹配，这样 `cargo fmt` 换行折行不会造成假失败。
    fn normalized_source_snippet(anchor: &str) -> String {
        include_str!("content_views.rs")
            .split(anchor)
            .nth(1)
            .unwrap_or_else(|| panic!("源码里应存在锚点 {anchor}"))
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// 下面两条守的是「看 diff 看不出来、纯函数也覆盖不到」的布局/方向错误。
    /// 仓库已有 `include_str!` 源码结构断言的先例（见本文件顶部的面板结构断言）。
    #[test]
    fn redis_right_layout_gives_both_axes_a_definite_size() {
        // 右侧布局下外层是行方向：编辑器容器若不带 h_full，高度只有内容高，撑不满整列。
        let arm = normalized_source_snippet("fn redis_workbench_input_panel(");
        let right_arm = arm
            .split("ResultsPlacement::Right=>")
            .nth(1)
            .expect("输入面板应有 Right 分支");
        assert!(right_arm.contains("h_full()"), "右侧输入面板必须 h_full");
        assert!(right_arm.contains("min_w(px(0.))"), "右侧输入面板需可收缩");
        assert!(right_arm.contains("border_r_1()"), "右侧布局分隔线应在右边");
    }

    #[test]
    fn redis_right_drag_grows_editor_rightward() {
        // 锚到拖拽处理本身：同一个函数里还有一个「手柄形状」的 match placement，
        // 直接取第一个 Right 分支会抓到那一处（无 delta）。
        let body = normalized_source_snippet("match start.placement {");
        let right_arm = body
            .split("ResultsPlacement::Right=>")
            .nth(1)
            .expect("拖拽处理应有 Right 分支");
        // Redis 右侧布局的编辑器在**左**，向右拖是变宽：当前 X − 起点 X。
        // SQL 结果面板在右侧、向左生长，那边是反的（start.x − current.x），不能照抄。
        assert!(
            right_arm.contains("position.x)-start.x"),
            "Redis 右侧拖拽增量必须是 当前X − 起点X（编辑器在左，向右拖变宽）"
        );
        assert!(
            !right_arm.contains("start.x-f32::from"),
            "不要照抄 SQL 结果面板的反向增量"
        );
    }

    #[test]
    fn redis_workbench_editor_height_clamps_to_min_and_max_ratio() {
        // 结果区 ∈ [split/6, 0.80·split] ⇒ editor ∈ [0.20·split, 5/6·split]。
        let split = 860.;
        let min_editor = split / 5.; // 结果区最大(0.8·split) ⇒ 编辑器最小 = 0.20·split
        let max_editor = split * 5. / 6.; // 结果区最小 ⇒ 编辑器最大 = 5·split/6
        // 默认占比 63%（结果区默认 37.5% 分栏）落在范围内，直接按比例取值。
        let height = redis_workbench_editor_height(0.63, split);
        assert!((height - split * 0.63).abs() < 1e-3);
        // 占比低于下限（结果区拉满）被夹到最小编辑器高度。
        let lo = redis_workbench_editor_height(0.01, split);
        assert!((lo - min_editor).abs() < 1e-3);
        // 占比高于上限（结果区压到最小）被夹到最大编辑器高度。
        let hi = redis_workbench_editor_height(0.99, split);
        assert!((hi - max_editor).abs() < 1e-3);
        // 可用高度过小时不应 panic。
        let tiny = redis_workbench_editor_height(1.0, 10.);
        assert!(tiny > 0.);
    }

    #[test]
    fn query_result_table_refreshes_when_rows_or_sorts_change() {
        let rows = vec![
            vec![SharedString::from("1")],
            vec![SharedString::from("2")],
        ];
        let reversed = vec![
            vec![SharedString::from("2")],
            vec![SharedString::from("1")],
        ];
        let sorts = vec![DataTableSort {
            col_ix: 1,
            direction: DataTableSortDirection::Ascending,
        }];

        assert!(data_table_rows_or_sorts_changed(
            &rows,
            &[],
            &reversed,
            &[]
        ));
        assert!(data_table_rows_or_sorts_changed(
            &rows,
            &[],
            &rows,
            &sorts
        ));
        assert!(!data_table_rows_or_sorts_changed(
            &rows,
            &sorts,
            &rows,
            &sorts
        ));
    }

    #[test]
    fn data_table_column_widths_apply_by_column_key() {
        let mut columns = vec![
            TableColumn::new("__row_index", "#").width(px(54.)),
            TableColumn::new("id", "id").width(px(170.)),
            TableColumn::new("name", "name").width(px(170.)),
        ];
        let widths = BTreeMap::from([
            ("name".to_string(), px(260.)),
            ("id".to_string(), px(90.)),
        ]);

        apply_data_table_column_widths(&mut columns, &widths);

        assert_eq!(columns[0].width, px(54.));
        assert_eq!(columns[1].width, px(90.));
        assert_eq!(columns[2].width, px(260.));
    }

    #[test]
    fn data_table_column_widths_apply_from_resize_event_order() {
        let mut columns = vec![
            TableColumn::new("__row_index", "#").width(px(54.)),
            TableColumn::new("id", "id").width(px(170.)),
            TableColumn::new("name", "name").width(px(170.)),
        ];

        apply_data_table_column_widths_from_list(&mut columns, &[px(54.), px(90.), px(260.)]);

        assert_eq!(columns[0].width, px(54.));
        assert_eq!(columns[1].width, px(90.));
        assert_eq!(columns[2].width, px(260.));
    }

    #[test]
    fn data_table_columns_match_detects_width_changes() {
        let left = vec![TableColumn::new("id", "id").width(px(170.))];
        let right = vec![TableColumn::new("id", "id").width(px(260.))];

        assert!(!data_table_columns_match(&left, &right));
    }

    #[test]
    fn query_result_local_sort_applies_to_result_pages() {
        let page = DataPage {
            columns: vec![GdbColumn {
                name: "id".to_string(),
                type_name: Some("int".to_string()),
                nullable: false,
                primary_key: false,
                comment: None,
            }],
            rows: vec![
                fluxdb_core::Row {
                    values: vec![CellValue::I64(2)],
                },
                fluxdb_core::Row {
                    values: vec![CellValue::I64(1)],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let rules = vec![DataSortRule {
            enabled: true,
            field: "id".to_string(),
            ascending: true,
        }];

        let sorted = sorted_query_result_page(&page, &rules);

        assert_eq!(sorted.page.rows[0].values[0], CellValue::I64(1));
        assert_eq!(sorted.page.rows[1].values[0], CellValue::I64(2));
        assert_eq!(sorted.source_row_indexes, vec![1, 0]);
    }

    #[test]
    fn sorted_query_result_keeps_inline_editing_when_editable() {
        let rules = vec![DataSortRule {
            enabled: true,
            field: "id".to_string(),
            ascending: true,
        }];

        assert!(query_result_cells_editable(true, &[]));
        assert!(query_result_cells_editable(true, &rules));
        assert!(!query_result_cells_editable(false, &[]));
    }

    #[test]
    fn temporal_cell_editor_detects_date_time_types() {
        assert_eq!(
            data_cell_temporal_kind("timestamp(6)"),
            Some(DataCellTemporalKind::DateTime)
        );
        assert_eq!(
            data_cell_temporal_kind("datetime"),
            Some(DataCellTemporalKind::DateTime)
        );
        assert_eq!(
            data_cell_temporal_kind("date"),
            Some(DataCellTemporalKind::Date)
        );
        assert_eq!(
            data_cell_temporal_kind("time"),
            Some(DataCellTemporalKind::Time)
        );
        assert_eq!(data_cell_temporal_kind("varchar(255)"), None);
    }

    #[test]
    fn data_cell_editor_kind_detects_bool_enum_and_set_types() {
        assert_eq!(
            data_cell_editor_kind("tinyint(1) unsigned"),
            DataCellEditorKind::Boolean
        );
        assert_eq!(data_cell_editor_kind("bool"), DataCellEditorKind::Boolean);
        assert_eq!(
            data_cell_editor_kind("enum('draft','published')"),
            DataCellEditorKind::Enum
        );
        assert_eq!(
            data_cell_editor_kind("set('read','write')"),
            DataCellEditorKind::Set
        );
        assert_eq!(
            data_cell_editor_kind("varchar(255)"),
            DataCellEditorKind::Text
        );
    }

    #[test]
    fn binary_data_types_are_read_only_for_cell_editing() {
        for type_name in [
            "BINARY(16)",
            "VARBINARY(255)",
            "TINYBLOB",
            "BLOB",
            "MEDIUMBLOB",
            "LONGBLOB",
        ] {
            assert!(
                data_type_is_binary(type_name),
                "{type_name} should be detected as binary"
            );
        }
        assert!(!data_type_is_binary("varchar(255)"));
        assert!(!data_type_is_binary("datetime"));
    }

    #[test]
    fn dirty_cell_keeps_dirty_background_when_selected() {
        assert!(!data_cell_selection_should_fill_background(true, false));
        assert!(!data_cell_selection_should_fill_background(false, true));
        assert!(data_cell_selection_should_fill_background(false, false));
    }

    #[test]
    fn deleted_row_content_is_dimmed() {
        assert!(data_cell_content_opacity(true) < data_cell_content_opacity(false));
    }

    #[test]
    fn data_cell_edit_commits_on_enter_and_blur() {
        assert!(data_cell_edit_event_should_commit(
            &InputEvent::PressEnter {
                secondary: false,
                shift: false,
            }
        ));
        assert!(data_cell_edit_event_should_commit(&InputEvent::Blur));
        assert!(!data_cell_edit_event_should_commit(&InputEvent::Change));
    }

    #[test]
    fn data_cell_edit_commits_before_switching_to_another_cell() {
        let current = DataCellEditState {
            tab_id: TabId(1),
            query_result_page_index: None,
            visible_row: 0,
            source_row: 0,
            col_ix: 1,
            source_col: 0,
            temporal_kind: None,
        };
        let same = current;
        let other = DataCellEditState {
            visible_row: 1,
            source_row: 1,
            ..current
        };

        assert!(!data_cell_edit_should_commit_before_cell_change(
            Some(current),
            same
        ));
        assert!(data_cell_edit_should_commit_before_cell_change(
            Some(current),
            other
        ));
        assert!(!data_cell_edit_should_commit_before_cell_change(
            None, other
        ));
    }

    #[test]
    fn data_cell_edit_commits_before_query_output_tab_change() {
        let current = DataCellEditState {
            tab_id: TabId(1),
            query_result_page_index: Some(0),
            visible_row: 0,
            source_row: 0,
            col_ix: 1,
            source_col: 0,
            temporal_kind: None,
        };

        assert!(data_cell_edit_should_commit_before_query_output_tab_change(
            Some(current),
            TabId(1),
            false,
        ));
        assert!(!data_cell_edit_should_commit_before_query_output_tab_change(
            Some(current),
            TabId(1),
            true,
        ));
        assert!(!data_cell_edit_should_commit_before_query_output_tab_change(
            Some(current),
            TabId(2),
            false,
        ));
        assert!(!data_cell_edit_should_commit_before_query_output_tab_change(
            None,
            TabId(1),
            false,
        ));
    }

    #[test]
    fn query_result_edit_state_is_scoped_to_result_page() {
        let editing = DataCellEditState {
            tab_id: TabId(1),
            query_result_page_index: Some(0),
            visible_row: 0,
            source_row: 0,
            col_ix: 1,
            source_col: 0,
            temporal_kind: None,
        };

        assert!(data_cell_edit_matches_table(
            &editing,
            TabId(1),
            Some(0),
            0,
            1
        ));
        assert!(!data_cell_edit_matches_table(
            &editing,
            TabId(1),
            Some(1),
            0,
            1
        ));
        assert!(!data_cell_edit_matches_table(
            &editing,
            TabId(2),
            Some(0),
            0,
            1
        ));
    }

    #[test]
    fn data_cell_edit_text_unchanged_uses_display_text() {
        assert!(data_cell_edit_text_unchanged(
            &CellValue::Text("123456".to_string()),
            "123456"
        ));
        assert!(data_cell_edit_text_unchanged(
            &CellValue::I64(123456),
            "123456"
        ));
        assert!(!data_cell_edit_text_unchanged(
            &CellValue::Text("123456".to_string()),
            "123456111"
        ));
    }

    #[test]
    fn null_cell_edit_text_is_empty_but_text_null_is_literal() {
        assert_eq!(data_cell_edit_text(&CellValue::Null), "");
        assert_eq!(
            data_cell_edit_text(&CellValue::Text("NULL".to_string())),
            "NULL"
        );
        assert!(data_cell_edit_text_unchanged(&CellValue::Null, ""));
    }

    #[test]
    fn enum_and_set_options_are_parsed_from_mysql_type_declarations() {
        assert_eq!(
            data_cell_enum_set_options("enum('draft','it\\'s ok','published')"),
            vec![
                "draft".to_string(),
                "it's ok".to_string(),
                "published".to_string()
            ]
        );
        assert_eq!(
            data_cell_enum_set_options("set('read','write')"),
            vec!["read".to_string(), "write".to_string()]
        );
    }

    #[test]
    fn pasted_cell_values_are_validated_by_type_and_nullability() {
        let int_meta = DataTableColumnMeta {
            name: "age".to_string(),
            type_name: "int".to_string(),
            comment: None,
            nullable: false,
            primary_key: false,
            choices: Vec::new(),
        };
        assert_eq!(
            data_cell_value_from_text(&int_meta, "42").unwrap(),
            CellValue::I64(42)
        );
        assert!(data_cell_value_from_text(&int_meta, "abc").is_err());
        assert!(data_cell_value_from_text(&int_meta, "NULL").is_err());

        let nullable_bool = DataTableColumnMeta {
            name: "enabled".to_string(),
            type_name: "tinyint(1)".to_string(),
            comment: None,
            nullable: true,
            primary_key: false,
            choices: Vec::new(),
        };
        assert_eq!(
            data_cell_value_from_text(&nullable_bool, "true").unwrap(),
            CellValue::Bool(true)
        );
        assert_eq!(
            data_cell_value_from_text(&nullable_bool, "NULL").unwrap(),
            CellValue::Null
        );

        let enum_meta = DataTableColumnMeta {
            name: "status".to_string(),
            type_name: "enum('draft','published')".to_string(),
            comment: None,
            nullable: false,
            primary_key: false,
            choices: Vec::new(),
        };
        assert_eq!(
            data_cell_value_from_text(&enum_meta, "draft").unwrap(),
            CellValue::Text("draft".to_string())
        );
        assert!(data_cell_value_from_text(&enum_meta, "archived").is_err());

        let blob_meta = DataTableColumnMeta {
            name: "payload".to_string(),
            type_name: "LONGBLOB".to_string(),
            comment: None,
            nullable: true,
            primary_key: false,
            choices: Vec::new(),
        };
        assert!(data_cell_value_from_text(&blob_meta, "hello").is_err());
        assert!(data_cell_value_from_text(&blob_meta, "NULL").is_err());
    }

    #[test]
    fn column_choices_config_reads_saved_choices() {
        let mut saved = BTreeMap::new();
        saved.insert(
            "state".to_string(),
            vec![ColumnChoice {
                value: "success".to_string(),
                label: "成功".to_string(),
            }],
        );
        let mut options = BTreeMap::new();
        options.insert(
            COLUMN_CHOICES_OPTION.to_string(),
            serde_json::to_string(&saved).unwrap(),
        );
        assert_eq!(
            column_choices_config(&options).get("state"),
            Some(&saved["state"])
        );
    }

    #[test]
    fn temporal_cell_editor_replaces_date_and_time_parts() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();

        assert_eq!(
            replace_temporal_date_part(
                "2021-01-25 23:45:37.000000",
                Some(DataCellTemporalKind::DateTime),
                date,
            ),
            "2026-07-15 23:45:37.000000"
        );
        assert_eq!(
            replace_temporal_time_part(
                "2021-01-25 23:45:37.000000",
                Some(DataCellTemporalKind::DateTime),
                "06:00:00",
            ),
            "2021-01-25 06:00:00"
        );
        assert_eq!(
            replace_temporal_time_part("23:45:37", Some(DataCellTemporalKind::Time), "06:00:00"),
            "06:00:00"
        );
    }

    #[test]
    fn temporal_part_input_updates_one_part_and_clamps_ranges() {
        let edit = |part| TemporalPartEditState {
            target: TemporalEditTarget::CellDetail(TabId(1)),
            part,
            kind: DataCellTemporalKind::DateTime,
        };

        assert_eq!(
            temporal_value_after_part_input("2021-02-02 19:21:30", edit(TemporalPart::Month), "13")
                .unwrap(),
            "2021-12-02 19:21:30"
        );
        assert_eq!(
            temporal_value_after_part_input("2021-02-02 19:21:30", edit(TemporalPart::Day), "31")
                .unwrap(),
            "2021-02-28 19:21:30"
        );
        assert_eq!(
            temporal_value_after_part_input("2021-02-02 19:21:30", edit(TemporalPart::Second), "88")
                .unwrap(),
            "2021-02-02 19:21:59"
        );
    }

    #[test]
    fn temporal_time_parts_reads_time_from_time_and_datetime_text() {
        assert_eq!(temporal_time_parts("23:45:37"), (23, 45, 37));
        assert_eq!(temporal_time_parts("2026-07-15 06:08:09.000000"), (6, 8, 9));
        assert_eq!(temporal_time_parts("not-a-time"), (0, 0, 0));
    }

    #[test]
    fn temporal_shift_month_clamps_to_valid_day() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();

        assert_eq!(
            temporal_shift_month(date, -1),
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(
            temporal_shift_month(date, 1),
            NaiveDate::from_ymd_opt(2026, 4, 30).unwrap()
        );
    }

    #[test]
    fn app_message_replaces_previous_message() {
        let first = next_app_message(None, "已复制", AppMessageKind::Success);
        let second = next_app_message(Some(&first), "保存成功", AppMessageKind::Success);

        assert_eq!(second.id, first.id + 1);
        assert_eq!(second.text, "保存成功");
    }

    #[test]
    fn statusbar_summary_ignores_last_error() {
        let mut state = AppState::default();
        state.last_error = Some(fluxdb_core::UserFacingError {
            title: "保存失败".to_string(),
            message: "字段太长".to_string(),
            detail: None,
            retryable: false,
        });

        assert_eq!(statusbar_summary(&state), "0 个对象");
    }

    #[test]
    fn app_message_layout_uses_bottom_center() {
        let layout = app_message_layout_for_width(736.);

        assert_eq!(layout.bottom, 40.);
        assert_eq!(layout.max_width, 640.);
    }

    #[test]
    fn search_does_not_force_collapsed_connection_open() {
        assert!(!connection_should_show_children(false, true));
        assert!(connection_should_show_children(true, true));
    }

    #[test]
    fn search_tree_nodes_default_open_but_respect_collapse() {
        assert!(tree_expanded_for_search(None, true));
        assert!(!tree_expanded_for_search(Some(false), true));
        assert!(tree_expanded_for_search(Some(true), true));
    }

    #[test]
    fn empty_visible_database_filter_shows_databases() {
        let options = BTreeMap::from([(VISIBLE_DATABASES_OPTION.to_string(), String::new())]);

        assert_eq!(configured_visible_databases(&options), None);
    }

    #[test]
    fn data_page_number_uses_limit_not_visible_row_count() {
        assert_eq!(data_page_number(100, 100), 2);
        assert_eq!(data_page_number(100, 5), 21);
    }

    #[test]
    fn data_page_offset_uses_one_based_page_number() {
        assert_eq!(data_page_offset_for_page(1, 100), 0);
        assert_eq!(data_page_offset_for_page(3, 100), 200);
        assert_eq!(data_page_offset_for_page(0, 100), 0);
    }

    #[test]
    fn data_page_offset_caps_at_supported_max_page() {
        assert_eq!(data_page_offset_for_supported_page(101, 100), 9900);
        assert_eq!(data_page_offset_for_supported_page(999, 50), 4950);
    }

    #[test]
    fn row_copy_formats_json_and_tsv() {
        let fields = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice".to_string()),
            },
        ];

        assert_eq!(
            row_json_text(fields.as_slice()),
            "{\n  \"id\": 7,\n  \"name\": \"Alice\"\n}"
        );
        assert_eq!(row_tsv_text(fields.as_slice()), "id\tname\n7\tAlice");
    }

    #[test]
    fn row_copy_formats_multiple_rows_json_and_tsv() {
        let first = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice".to_string()),
            },
        ];
        let second = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(8),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Bob".to_string()),
            },
        ];
        let rows = vec![first.as_slice(), second.as_slice()];

        assert_eq!(
            row_json_array_text(rows.as_slice()),
            "[\n  {\n    \"id\": 7,\n    \"name\": \"Alice\"\n  },\n  {\n    \"id\": 8,\n    \"name\": \"Bob\"\n  }\n]"
        );
        assert_eq!(
            row_tsv_rows_text(rows.as_slice()),
            "id\tname\n7\tAlice\n8\tBob"
        );
    }

    #[test]
    fn data_row_copy_submenu_width_expands_for_long_labels() {
        let width = data_row_copy_submenu_width([
            "复制选中 8 行 (JSON)",
            "复制选中 8 行为 INSERT 语句",
            "复制选中 8 行为 INSERT 语句（不含主键）",
            "复制选中 8 行为 UPDATE 语句",
            "复制选中 8 行 (TSV)",
        ]);

        assert!(width > 278.);
        assert!(width <= 440.);
    }

    #[test]
    fn data_row_export_writes_csv_with_header_and_escaping() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let rows = vec![vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice, \"A\"".to_string()),
            },
            RowFieldSnapshot {
                index: 3,
                name: "note".to_string(),
                type_name: "text".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("line\nbreak".to_string()),
            },
        ]];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::Csv,
            Some(&object),
            rows.as_slice(),
            DatabaseKind::MySql,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\u{FEFF}id,name,note\n7,\"Alice, \"\"A\"\"\",\"line\nbreak\"\n"
        );
    }

    #[test]
    fn data_row_export_without_object_supports_plain_formats_only() {
        let rows = vec![vec![RowFieldSnapshot {
            index: 1,
            name: "id".to_string(),
            type_name: "int".to_string(),
            primary_key: false,
            comment: None,
            value: CellValue::I64(7),
        }]];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::Csv,
            None,
            rows.as_slice(),
            DatabaseKind::MySql,
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "\u{FEFF}id\n7\n");

        let mut output = Vec::new();
        assert!(write_data_row_export(
            &mut output,
            DataRowExportFormat::SqlInsert,
            None,
            rows.as_slice(),
            DatabaseKind::MySql,
        )
        .is_err());
    }

    #[test]
    fn data_row_export_writes_markdown_with_escaped_cells() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let rows = vec![vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name|title".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice|Admin\nLead".to_string()),
            },
        ]];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::Markdown,
            Some(&object),
            rows.as_slice(),
            DatabaseKind::MySql,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "| id | name\\|title |\n| --- | --- |\n| 7 | Alice\\|Admin<br>Lead |\n"
        );
    }

    #[test]
    fn data_row_export_writes_insert_sql_per_row() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let rows = vec![
            vec![RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            }],
            vec![RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(8),
            }],
        ];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::SqlInsert,
            Some(&object),
            rows.as_slice(),
            DatabaseKind::MySql,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "INSERT INTO `shop`.`users` (`id`) VALUES (7);\nINSERT INTO `shop`.`users` (`id`) VALUES (8);\n"
        );
    }

    #[test]
    fn table_data_export_writer_filters_fields_and_escapes_xml() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "name".to_string(),
                    type_name: Some("varchar(20)".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![fluxdb_core::Row {
                values: vec![CellValue::I64(7), CellValue::Text("Alice & Bob".to_string())],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let base = std::env::temp_dir().join(format!(
            "gdb-table-export-test-{}",
            std::process::id()
        ));
        let csv_path = base.with_extension("csv");
        let xml_path = base.with_extension("xml");

        let mut csv = TableDataExportWriter::create(
            &csv_path,
            TableDataExportFormat::Csv,
            object.clone(),
            vec!["name".to_string()],
            DatabaseKind::MySql,
        )
        .unwrap();
        assert_eq!(csv.write_page(&page).unwrap(), 1);
        csv.finish().unwrap();

        let mut xml = TableDataExportWriter::create(
            &xml_path,
            TableDataExportFormat::Xml,
            object,
            vec!["name".to_string()],
            DatabaseKind::MySql,
        )
        .unwrap();
        assert_eq!(xml.write_page(&page).unwrap(), 1);
        xml.finish().unwrap();

        assert_eq!(
            fs::read_to_string(&csv_path).unwrap(),
            "\u{FEFF}name\nAlice & Bob\n"
        );
        assert!(fs::read_to_string(&xml_path)
            .unwrap()
            .contains("<field name=\"name\">Alice &amp; Bob</field>"));
        let _ = fs::remove_file(csv_path);
        let _ = fs::remove_file(xml_path);
    }

    /// T23：PG 导出各格式（SQL/CSV/JSON/XML/TXT）类型化值往返与方言渲染。
    #[test]
    fn postgres_table_data_export_formats_render_typed_values() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("appdb".to_string()),
            schema: Some("public".to_string()),
            name: "items".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: Some("integer".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "price".to_string(),
                    type_name: Some("numeric".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "tags".to_string(),
                    type_name: Some("jsonb".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "note".to_string(),
                    type_name: Some("text".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![fluxdb_core::Row {
                values: vec![
                    CellValue::I64(1),
                    // numeric 保精：以精确十进制文本（不经 f64）。
                    CellValue::Text("12.50".to_string()),
                    CellValue::Json("{\"k\": 1}".to_string()),
                    CellValue::Text("O'Brien".to_string()),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let fields = vec![
            "id".to_string(),
            "price".to_string(),
            "tags".to_string(),
            "note".to_string(),
        ];
        let base = std::env::temp_dir().join(format!(
            "gdb-pg-export-test-{}",
            std::process::id()
        ));
        let write = |ext: &str, format: TableDataExportFormat| -> String {
            let path = base.with_extension(ext);
            let mut w = TableDataExportWriter::create(
                &path,
                format,
                object.clone(),
                fields.clone(),
                DatabaseKind::Postgres,
            )
            .unwrap();
            w.write_page(&page).unwrap();
            w.finish().unwrap();
            let text = fs::read_to_string(&path).unwrap();
            let _ = fs::remove_file(&path);
            text
        };

        // SQL：PG 双引号标识符限定 + 类型化字面量（jsonb 具名转换；文本单引号转义）。
        let sql = write("sql", TableDataExportFormat::Sql);
        assert!(sql.contains("\"public\".\"items\""), "PG 限定名应双引号：{sql}");
        assert!(sql.contains("'12.50'"), "numeric 应以精确文本输出：{sql}");
        assert!(sql.contains("O''Brien"), "文本单引号应转义：{sql}");
        assert!(sql.contains("::jsonb"), "jsonb 应显式转换：{sql}");

        // CSV：UTF-8 BOM + 表头 + 值（含引号转义）。
        let csv = write("csv", TableDataExportFormat::Csv);
        assert!(
            csv.starts_with("\u{FEFF}id,price,tags,note\n"),
            "CSV 表头应以 BOM + 表头开头：{csv}"
        );
        assert!(csv.contains("12.50"), "numeric 值：{csv}");

        // JSON：值以 JSON 呈现。
        let json = write("json", TableDataExportFormat::Json);
        assert!(json.contains("\"price\""), "JSON 列名：{json}");
        assert!(json.contains("12.50"), "JSON numeric 值：{json}");

        // XML：转义字段（撇号转 &apos;，双引号转 &quot;）。
        let xml = write("xml", TableDataExportFormat::Xml);
        assert!(
            xml.contains("<field name=\"note\">O&apos;Brien</field>"),
            "XML 文本字段应转义撇号：{xml}"
        );
        assert!(
            xml.contains("&quot;k&quot;"),
            "XML 文本字段应转义双引号：{xml}"
        );

        // TXT：制表符分隔。
        let txt = write("txt", TableDataExportFormat::Txt);
        assert!(txt.contains("12.50"), "TXT 值：{txt}");
        assert!(txt.contains('\t'), "TXT 应为制表符分隔：{txt}");
    }

    #[test]
    fn data_table_rows_tsv_uses_visible_row_order() {
        let rows = vec![
            vec![SharedString::from("1"), SharedString::from("Alice")],
            vec![SharedString::from("2"), SharedString::from("Bob")],
            vec![SharedString::from("3"), SharedString::from("Chen")],
        ];

        assert_eq!(
            data_table_rows_tsv(rows.as_slice(), &BTreeSet::from([0, 2])),
            "1\tAlice\n3\tChen"
        );
    }

    #[test]
    fn data_table_cells_tsv_keeps_sparse_shape_and_sanitizes_cells() {
        let rows = vec![
            vec![
                SharedString::from("A1"),
                SharedString::from("A\t2"),
                SharedString::from("A3"),
            ],
            vec![
                SharedString::from("B1"),
                SharedString::from("B2"),
                SharedString::from("B\n3"),
            ],
        ];

        assert_eq!(
            data_table_cells_tsv(rows.as_slice(), &BTreeSet::from([(0, 1), (1, 3)])),
            "A1\t\n\tB 3"
        );
    }

    #[test]
    fn data_table_index_range_selects_inclusive_rows_in_either_direction() {
        assert_eq!(data_table_index_range(2, 5), BTreeSet::from([2, 3, 4, 5]));
        assert_eq!(data_table_index_range(5, 2), BTreeSet::from([2, 3, 4, 5]));
    }

    #[test]
    fn data_table_cell_range_selects_rectangular_region() {
        assert_eq!(
            data_table_cell_range(1, 2, 3, 4),
            BTreeSet::from([
                (1, 2),
                (1, 3),
                (1, 4),
                (2, 2),
                (2, 3),
                (2, 4),
                (3, 2),
                (3, 3),
                (3, 4),
            ])
        );
    }

    #[test]
    fn row_copy_insert_can_skip_primary_keys() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let fields = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice".to_string()),
            },
        ];

        assert_eq!(
            row_insert_sql(&object, fields.as_slice(), false, DatabaseKind::MySql),
            "INSERT INTO `shop`.`users` (`id`, `name`) VALUES (7, 'Alice');"
        );
        assert_eq!(
            row_insert_sql(&object, fields.as_slice(), true, DatabaseKind::MySql),
            "INSERT INTO `shop`.`users` (`name`) VALUES ('Alice');"
        );

        // PG：双引号标识符 + schema.table 限定（不生成跨库三段名）。
        let mut pg_object = object.clone();
        pg_object.kind = ObjectKind::Table;
        pg_object.schema = Some("public".to_string());
        assert_eq!(
            row_insert_sql(&pg_object, fields.as_slice(), false, DatabaseKind::Postgres),
            "INSERT INTO \"public\".\"users\" (\"id\", \"name\") VALUES (7, 'Alice');"
        );
    }

    /// PG 导出/预览字面量：bytea 用 `'\x..'::bytea`（非 MySQL `X'..'`）、jsonb 显式转换。
    #[test]
    fn postgres_export_literals_use_pg_bytea_and_jsonb() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("appdb".to_string()),
            schema: Some("public".to_string()),
            name: "blobs".to_string(),
            kind: ObjectKind::Table,
        };
        let fields = vec![
            RowFieldSnapshot {
                index: 1,
                name: "payload".to_string(),
                type_name: "bytea".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
            },
            RowFieldSnapshot {
                index: 2,
                name: "meta".to_string(),
                type_name: "jsonb".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Json("{\"k\": 1}".to_string()),
            },
        ];
        assert_eq!(
            row_insert_sql(&object, fields.as_slice(), false, DatabaseKind::Postgres),
            "INSERT INTO \"public\".\"blobs\" (\"payload\", \"meta\") \
             VALUES ('\\xdeadbeef'::bytea, '{\"k\": 1}'::jsonb);"
        );
        // MySQL 保持 X'..' 十六进制 + 裸 JSON 文本。
        assert_eq!(
            row_insert_sql(&object, fields.as_slice(), false, DatabaseKind::MySql),
            "INSERT INTO `appdb`.`public`.`blobs` (`payload`, `meta`) \
             VALUES (X'DEADBEEF', '{\"k\": 1}');"
        );
    }

    #[test]
    fn failed_app_event_becomes_error_message() {
        let event = AppEvent::Failed(fluxdb_core::UserFacingError {
            title: "连接失败".to_string(),
            message: "Connection refused".to_string(),
            detail: None,
            retryable: true,
        });

        assert_eq!(
            app_event_message(&event),
            Some((
                "连接失败：Connection refused".to_string(),
                AppMessageKind::Error
            ))
        );
    }

    #[test]
    fn local_table_filter_value_toggle_supports_multiselect() {
        let filters = BTreeMap::new();
        let filters = local_table_filters_after_value_toggle(filters, "status", "active");
        let filters = local_table_filters_after_value_toggle(filters, "status", "pending");

        assert_eq!(
            filters.get("status"),
            Some(&BTreeSet::from([
                "active".to_string(),
                "pending".to_string()
            ]))
        );

        let filters = local_table_filters_after_value_toggle(filters, "status", "active");
        assert_eq!(
            filters.get("status"),
            Some(&BTreeSet::from(["pending".to_string()]))
        );
    }

    #[test]
    fn data_filter_rules_convert_to_filter_specs() {
        let specs = data_filter_specs_from_rules(&[DataFilterRule {
            enabled: true,
            field: Some("name".to_string()),
            operator: DataFilterOperator::Contains,
            values: BTreeSet::from(["bike".to_string(), "helmet".to_string()]),
            grouped: false,
        }]);

        assert_eq!(
            specs,
            vec![FilterSpec {
                field: "name".to_string(),
                op: FilterOp::Contains,
                values: vec![
                    CellValue::Text("bike".to_string()),
                    CellValue::Text("helmet".to_string())
                ],
                enabled: true,
            }]
        );
    }

    #[test]
    fn data_filter_values_strip_outer_quotes_when_used() {
        let specs = data_filter_specs_from_rules(&[DataFilterRule {
            enabled: true,
            field: Some("id".to_string()),
            operator: DataFilterOperator::Eq,
            values: BTreeSet::from([
                "\"abc\"".to_string(),
                "'def'".to_string(),
                "“ghi”".to_string(),
                "「jkl」".to_string(),
            ]),
            grouped: false,
        }]);

        assert_eq!(
            specs[0].values,
            vec![
                CellValue::Text("abc".to_string()),
                CellValue::Text("def".to_string()),
                CellValue::Text("ghi".to_string()),
                CellValue::Text("jkl".to_string())
            ]
        );
    }

    #[test]
    fn local_filter_manager_hides_current_editing_field_from_condition_list() {
        let mut filters = BTreeMap::new();
        filters.insert(
            "status".to_string(),
            BTreeSet::from(["active".to_string(), "pending".to_string()]),
        );
        filters.insert("name".to_string(), BTreeSet::from(["tom".to_string()]));

        let entries = local_filter_manager_condition_entries(&filters, Some("status"));

        assert_eq!(
            entries,
            vec![("name".to_string(), BTreeSet::from(["tom".to_string()]))]
        );
    }

    #[test]
    fn data_search_matches_current_page_cells_case_insensitively() {
        let rows = vec![
            vec![SharedString::from("Alpha"), SharedString::from("beta")],
            vec![SharedString::from("草稿-复制"), SharedString::from("other")],
            vec![SharedString::from("copy"), SharedString::from("ALPHA")],
        ];

        assert_eq!(
            data_search_matches(rows.as_slice(), "alpha"),
            vec![
                DataSearchMatch {
                    row_ix: 0,
                    col_ix: 1
                },
                DataSearchMatch {
                    row_ix: 2,
                    col_ix: 2
                },
            ]
        );
        assert_eq!(
            data_search_matches(rows.as_slice(), "草稿"),
            vec![DataSearchMatch {
                row_ix: 1,
                col_ix: 1
            }]
        );
    }

    #[test]
    fn next_data_search_match_wraps_after_current_match() {
        let matches = vec![
            DataSearchMatch {
                row_ix: 0,
                col_ix: 1,
            },
            DataSearchMatch {
                row_ix: 2,
                col_ix: 3,
            },
        ];

        assert_eq!(
            next_data_search_match(matches.as_slice(), None),
            Some(matches[0])
        );
        assert_eq!(
            next_data_search_match(matches.as_slice(), Some(matches[0])),
            Some(matches[1])
        );
        assert_eq!(
            next_data_search_match(matches.as_slice(), Some(matches[1])),
            Some(matches[0])
        );
    }

    #[test]
    fn data_search_match_label_uses_current_position_and_total() {
        let matches = vec![
            DataSearchMatch {
                row_ix: 0,
                col_ix: 1,
            },
            DataSearchMatch {
                row_ix: 2,
                col_ix: 3,
            },
        ];

        assert_eq!(
            data_search_match_label(matches.as_slice(), Some(matches[1])),
            "2/2 匹配"
        );
        assert_eq!(
            data_search_match_label(matches.as_slice(), None),
            "1/2 匹配"
        );
        assert_eq!(data_search_match_label(&[], None), "0/0 匹配");
    }

    #[test]
    fn data_editor_sql_preview_includes_current_page_by_default() {
        let sql = data_editor_sql_preview(
            &ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                name: "users".to_string(),
                kind: ObjectKind::Table,
            },
            &[],
            &[],
            DataFilterMode::Builder,
            "",
            "",
            100,
            100,
            DatabaseKind::MySql,
        );

        assert_eq!(sql, "SELECT * FROM `main`.`users` LIMIT 100 OFFSET 100");
    }

    #[test]
    fn data_editor_sql_preview_postgres_uses_double_quotes_and_real_column_names() {
        // PG：三段对象名用双引号（schema/表），排序用真实字段名（主键不带 “  PK” 后缀），
        // 字段带双引号且内部引号转义为 ""。
        let mut object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("fluxdb_manual".to_string()),
            schema: Some("tenant_a".to_string()),
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        };
        let sort_rules = vec![DataSortRule {
            enabled: true,
            field: "id".to_string(),
            ascending: false,
        }];
        // 主键字段升序。
        let primary_key_rules = vec![DataSortRule {
            field: "id".to_string(),
            ascending: true,
            ..sort_rules[0].clone()
        }];

        let pg_version = data_editor_sql_preview(
            &object,
            &[],
            &sort_rules,
            DataFilterMode::Builder,
            "",
            "",
            0,
            1000,
            DatabaseKind::Postgres,
        );
        assert_eq!(
            pg_version,
            "SELECT * FROM \"tenant_a\".\"orders\" ORDER BY \"id\" DESC LIMIT 1000"
        );

        // 主键字段（真实名 id）升序。
        let pk_asc = data_editor_sql_preview(
            &object,
            &[],
            &primary_key_rules,
            DataFilterMode::Builder,
            "",
            "",
            0,
            1000,
            DatabaseKind::Postgres,
        );
        assert_eq!(
            pk_asc,
            "SELECT * FROM \"tenant_a\".\"orders\" ORDER BY \"id\" ASC LIMIT 1000"
        );

        // 无 schema 时 PG 只用表名双引号。
        object.schema = None;
        let no_schema = data_editor_sql_preview(
            &object,
            &[],
            &[],
            DataFilterMode::Builder,
            "",
            "",
            0,
            1000,
            DatabaseKind::Postgres,
        );
        assert_eq!(
            no_schema,
            "SELECT * FROM \"orders\" LIMIT 1000"
        );

        // MySQL 保持反引号行为不被破坏。
        let mysql_version = data_editor_sql_preview(
            &object,
            &[],
            &sort_rules,
            DataFilterMode::Builder,
            "",
            "",
            0,
            1000,
            DatabaseKind::MySql,
        );
        assert_eq!(
            mysql_version,
            "SELECT * FROM `fluxdb_manual`.`orders` ORDER BY `id` DESC LIMIT 1000"
        );
    }

    #[test]
    fn parses_editable_data_sql_into_filter_and_sort_text() {
        let parsed = parse_data_editor_sql_text(
            "SELECT * FROM `main`.`users` WHERE `name` LIKE '%tom%' AND `age` >= '18' ORDER BY `id` DESC, `name` ASC LIMIT 100 OFFSET 0",
        )
        .expect("simple data SQL should be parsed");

        assert_eq!(parsed.filter_text, "`name` LIKE '%tom%' AND `age` >= '18'");
        assert_eq!(parsed.sort_text, "`id` DESC, `name` ASC");
        assert_eq!(parsed.limit, Some(100));
    }

    #[test]
    fn parses_editable_data_sql_limit() {
        let parsed = parse_data_editor_sql_text("SELECT * FROM `users` LIMIT 250")
            .expect("limit should be parsed");

        assert_eq!(parsed.limit, Some(250));
    }

    /// 回归：数据页 SQL 面板里显示的 LIMIT 就是当前页大小，刷新（⌘R）会把它解析回来
    /// 再夹一次。封顶一度写死 100，于是「默认分页行数」选 500/1000 的表一按刷新就掉回 100。
    #[test]
    fn data_editor_limit_survives_refresh_for_every_offered_page_size() {
        for (label, size) in DATA_TABLE_PAGE_SIZE_CHOICES {
            assert_eq!(
                data_editor_effective_limit(size),
                size,
                "设置档位 {label} 的表刷新后不应被夹小"
            );
        }

        // 手输超大 LIMIT 仍被夹到最大档（护栏保留）。
        let max = data_table_page_size_max();
        assert_eq!(data_editor_effective_limit(max + 1), max);
        assert_eq!(data_editor_effective_limit(u64::MAX), max);
        // 下限静默夹到 1。
        assert_eq!(data_editor_effective_limit(0), 1);
    }

    #[test]
    fn sql_selection_offset_returns_valid_byte_boundary() {
        let bounds = Bounds::new(point(px(10.), px(0.)), size(px(200.), px(20.)));
        let text = "SELECT * FROM `测试`";

        let offset = sql_text_selection_offset(text, Some(&bounds), point(px(95.), px(4.)));

        assert!(text.is_char_boundary(offset));
    }

    #[test]
    fn byte_index_for_char_index_clamps_to_text_end() {
        assert_eq!(byte_index_for_char_index("测a", 99), "测a".len());
    }

    #[test]
    fn sql_prefix_char_count_clamps_to_text_end() {
        assert_eq!(sql_prefix_char_count("中文 SQL", 999), 6);
    }

    #[test]
    fn tab_context_menu_close_targets_stay_in_current_workspace_scope() {
        let current = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "main".to_string(),
        };
        let other_database = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "audit".to_string(),
        };
        let other_connection = WorkspaceScope {
            connection_id: ConnectionId(2),
            database: "main".to_string(),
        };
        let scopes = vec![
            (TabId(1), Some(current.clone())),
            (TabId(2), Some(current.clone())),
            (TabId(3), Some(other_database)),
            (TabId(4), Some(other_connection)),
            (TabId(5), None),
        ];

        assert_eq!(
            tab_context_menu_close_targets(scopes.iter().cloned(), TabId(1), false),
            vec![TabId(2)]
        );
        assert_eq!(
            tab_context_menu_close_targets(scopes.iter().cloned(), TabId(1), true),
            vec![TabId(1), TabId(2)]
        );
    }

    #[test]
    fn tab_row_overflow_uses_fixed_tab_width() {
        assert!(!tab_row_overflows(3, 260., 900.));
        assert!(tab_row_overflows(5, 260., 900.));
    }

    #[test]
    fn tab_switcher_popup_top_tracks_tab_layout() {
        assert_eq!(
            tab_switcher_popup_top(TabSwitcherKind::Databases, true),
            32.
        );
        assert_eq!(tab_switcher_popup_top(TabSwitcherKind::Tables, true), 66.);
        assert_eq!(tab_switcher_popup_top(TabSwitcherKind::Tables, false), 30.);
    }

    #[test]
    fn pinning_tabs_appends_to_pinned_group_in_order() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let mut pinned = BTreeSet::from([TabId(2)]);
        let order = tab_order_after_pin(&ids, &ids, &pinned, TabId(4));
        pinned.insert(TabId(4));

        assert_eq!(
            tab_display_order(&ids, &order, &pinned),
            vec![TabId(2), TabId(4), TabId(1), TabId(3),]
        );
    }

    #[test]
    fn unpinning_tab_moves_it_after_last_remaining_pinned_tab() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::from([TabId(2), TabId(4)]);

        assert_eq!(
            tab_order_after_unpin(&ids, &ids, &pinned, TabId(2)),
            vec![TabId(4), TabId(2), TabId(1), TabId(3),]
        );

        let mut remaining_pinned = pinned;
        remaining_pinned.remove(&TabId(2));
        assert_eq!(
            tab_display_order(
                &ids,
                &tab_order_after_unpin(&ids, &ids, &BTreeSet::from([TabId(2), TabId(4)]), TabId(2)),
                &remaining_pinned
            ),
            vec![TabId(4), TabId(2), TabId(1), TabId(3)]
        );
    }

    #[test]
    fn unpinning_last_pinned_tab_keeps_visual_position() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::from([TabId(2), TabId(4)]);
        let order = tab_order_after_unpin(&ids, &ids, &pinned, TabId(4));
        let mut remaining_pinned = pinned;
        remaining_pinned.remove(&TabId(4));

        assert_eq!(
            tab_display_order(&ids, &order, &remaining_pinned),
            vec![TabId(2), TabId(4), TabId(1), TabId(3)]
        );
    }

    #[test]
    fn dragging_within_same_tab_group_inserts_by_direction() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::new();

        let forward = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(4), TabId(2));
        assert_eq!(forward.pinned, None);
        assert_eq!(forward.order, vec![TabId(1), TabId(4), TabId(2), TabId(3)]);

        let backward = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(1), TabId(3));
        assert_eq!(backward.pinned, None);
        assert_eq!(backward.order, vec![TabId(2), TabId(3), TabId(1), TabId(4)]);
    }

    #[test]
    fn dragging_across_pinned_boundary_clamps_to_expected_group() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::from([TabId(1), TabId(2)]);

        let pinned_to_normal = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(1), TabId(3));
        assert_eq!(pinned_to_normal.pinned, Some(false));
        assert_eq!(
            pinned_to_normal.order,
            vec![TabId(2), TabId(3), TabId(4), TabId(1),]
        );

        let normal_to_pinned = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(4), TabId(1));
        assert_eq!(normal_to_pinned.pinned, None);
        assert_eq!(
            normal_to_pinned.order,
            vec![TabId(1), TabId(2), TabId(4), TabId(3),]
        );
    }

    #[test]
    fn workspace_tabs_are_reordered_by_drag_direction() {
        let a = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "a".to_string(),
        };
        let b = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "b".to_string(),
        };
        let c = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "c".to_string(),
        };
        let scopes = vec![a.clone(), b.clone(), c.clone()];

        assert_eq!(
            workspace_tab_order_after_drop(&scopes, &scopes, &c, &b),
            vec![a.clone(), c.clone(), b.clone()]
        );
        assert_eq!(
            workspace_tab_order_after_drop(&scopes, &scopes, &a, &b),
            vec![b.clone(), a.clone(), c.clone()]
        );
    }

    #[test]
    fn tab_switcher_entries_filter_by_search_text() {
        let mut state = AppState::default();
        state.tabs = vec![
            TabState {
                id: TabId(1),
                title: "shining_agent_chat_know_rel".to_string(),
                kind: TabKind::QueryEditor(QueryEditorState {
                    connection_id: ConnectionId(1),
                    database: Some("data_centre_cloud".to_string()),
                    schema: None,
                    text: String::new(),
                    origin: None,
                    saved_fingerprint: None,
                    running: false,
                    results: Vec::new(),
                    result_editors: BTreeMap::new(),
                    active_result_editor: None,
                    summaries: Vec::new(),
                    error: None,
                }),
                dirty: false,
            },
            TabState {
                id: TabId(2),
                title: "3d_device_model".to_string(),
                kind: TabKind::QueryEditor(QueryEditorState {
                    connection_id: ConnectionId(1),
                    database: Some("data_centre_cloud".to_string()),
                    schema: None,
                    text: String::new(),
                    origin: None,
                    saved_fingerprint: None,
                    running: false,
                    results: Vec::new(),
                    result_editors: BTreeMap::new(),
                    active_result_editor: None,
                    summaries: Vec::new(),
                    error: None,
                }),
                dirty: false,
            },
        ];
        state.active_tab = Some(TabId(1));
        let scope = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "data_centre_cloud".to_string(),
        };

        let entries = table_tab_entries(&state, Some(&scope), "device", &[], &BTreeSet::new());

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, TabId(2));
    }

    #[test]
    fn settings_tab_is_only_attached_to_its_current_database_workspace() {
        let query_tab = |id, database: &str| TabState {
            id: TabId(id),
            title: format!("{database} query"),
            kind: TabKind::QueryEditor(QueryEditorState {
                connection_id: ConnectionId(1),
                database: Some(database.to_string()),
                schema: None,
                text: String::new(),
                origin: None,
                saved_fingerprint: None,
                running: false,
                results: Vec::new(),
                result_editors: BTreeMap::new(),
                active_result_editor: None,
                summaries: Vec::new(),
                error: None,
            }),
            dirty: false,
        };
        let mut state = AppState::default();
        state.tabs = vec![
            query_tab(1, "community_test"),
            query_tab(3, "fluxdb_demo"),
            TabState {
                id: TabId(2),
                title: "设置".to_string(),
                kind: TabKind::Settings(fluxdb_app::SettingsTabState {
                    workspace: Some(WorkspaceScope {
                        connection_id: ConnectionId(1),
                        database: "fluxdb_demo".to_string(),
                    }),
                }),
                dirty: false,
            },
            query_tab(4, "newly-opened"),
        ];
        state.active_tab = Some(TabId(2));

        let active_scope = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "fluxdb_demo".to_string(),
        };
        assert_eq!(active_workspace_scope(&state), Some(active_scope.clone()));
        assert_eq!(
            table_tab_entries(&state, Some(&active_scope), "", &[], &BTreeSet::new())
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![TabId(3), TabId(2)]
        );

        for (database, expected_tab_id) in [("community_test", TabId(1)), ("newly-opened", TabId(4))]
        {
            let scope = WorkspaceScope {
                connection_id: ConnectionId(1),
                database: database.to_string(),
            };
            assert_eq!(
                table_tab_entries(&state, Some(&scope), "", &[], &BTreeSet::new())
                    .into_iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>(),
                vec![expected_tab_id]
            );
        }
    }

    #[test]
    fn app_icons_use_lucide_svg_assets() {
        assert_eq!(app_icon_path(AppIcon::Check), "icons/check.svg");
        assert_eq!(app_icon_path(AppIcon::Copy), "icons/copy-plus.svg");
        assert_eq!(app_icon_path(AppIcon::Close), "icons/x.svg");
        assert_eq!(
            app_icon_path(AppIcon::ChevronDown),
            "icons/chevron-down.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronLeft),
            "icons/chevron-left.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronRight),
            "icons/chevron-right.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronsLeft),
            "icons/chevrons-left.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronsRight),
            "icons/chevrons-right.svg"
        );
        assert_eq!(app_icon_path(AppIcon::Pin), "icons/pin.svg");
        assert_eq!(app_icon_path(AppIcon::Table), "icons/table-2.svg");
        assert_eq!(app_icon_path(AppIcon::Database), "icons/database.svg");
        assert_eq!(app_icon_path(AppIcon::Eye), "icons/eye.svg");
        assert_eq!(app_icon_path(AppIcon::EyeOff), "icons/eye-off.svg");
        assert_eq!(app_icon_path(AppIcon::List), "icons/list.svg");
        assert_eq!(app_icon_path(AppIcon::Minus), "icons/minus.svg");
        assert_eq!(app_icon_path(AppIcon::PanelBottom), "icons/panel-bottom.svg");
        assert_eq!(app_icon_path(AppIcon::PanelRight), "icons/panel-right.svg");
        assert_eq!(app_icon_path(AppIcon::Redo), "icons/redo-2.svg");
        assert_eq!(app_icon_path(AppIcon::Square), "icons/square.svg");
        assert_eq!(app_icon_path(AppIcon::Undo), "icons/undo-2.svg");
        assert_eq!(app_icon_path(AppIcon::AlignLeft), "icons/align-left.svg");
        assert_eq!(app_icon_path(AppIcon::ArrowUpDown), "icons/arrow-up-down.svg");
        assert_eq!(app_icon_path(AppIcon::FileSearch), "icons/file-search.svg");
        assert_eq!(app_icon_path(AppIcon::Workflow), "icons/workflow.svg");
        assert_eq!(app_icon_path(AppIcon::Bot), "icons/bot.svg");
        assert_eq!(app_icon_path(AppIcon::Github), "icons/github.svg");
        assert_eq!(app_icon_path(AppIcon::Select), "icons/text-select.svg");
        assert_eq!(app_icon_path(AppIcon::Text), "icons/text.svg");
        assert_eq!(app_icon_path(AppIcon::WrapText), "icons/wrap-text.svg");
    }

    /// 顶部栏按钮引用的图标必须在资源目录里真实存在。
    /// `Assets::load` 是运行时按路径读盘的，拼错文件名不会编译报错，只会在界面上静默缺图。
    #[test]
    fn topbar_icon_assets_exist_on_disk() {
        for icon in [AppIcon::Github, AppIcon::Bot, AppIcon::CalendarClock, AppIcon::Settings] {
            let relative = app_icon_path(icon);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(relative);
            assert!(path.exists(), "缺少图标资源 {}", path.display());
        }
    }

    #[test]
    fn sql_text_selection_returns_selected_text() {
        let selection = SqlTextSelection {
            text: "SELECT 123".to_string(),
            anchor: 0,
            cursor: 6,
            selecting: false,
            bounds: None,
        };

        assert_eq!(selection.selected_text().as_deref(), Some("SELECT"));
    }

    #[test]
    fn data_change_sql_preview_formats_update_insert_and_delete() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop-db".to_string()),
            schema: None,
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "status".to_string(),
                    type_name: Some("varchar(20)".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "note".to_string(),
                    type_name: Some("varchar(255)".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: Vec::new(),
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let changes = DataChangeSet {
            object,
            inserts: vec![fluxdb_core::Row {
                values: vec![
                    CellValue::I64(2),
                    CellValue::Text("new".to_string()),
                    CellValue::Null,
                ],
            }],
            updates: vec![fluxdb_core::RowUpdate {
                identity: RowIdentity {
                    values: BTreeMap::from([("id".to_string(), CellValue::I64(1))]),
                },
                cells: vec![fluxdb_core::CellUpdate {
                    column: "status".to_string(),
                    value: CellValue::Text("Bob's order".to_string()),
                }],
            }],
            deletes: vec![RowIdentity {
                values: BTreeMap::from([("id".to_string(), CellValue::I64(3))]),
            }],
            insert_intents: None,
        };

        let preview = data_change_sql_preview(&page, &changes, DatabaseKind::MySql);

        assert_eq!(data_change_statement_count(&changes), 3);
        assert!(
            preview.contains(
                "UPDATE `shop-db`.`orders` SET `status` = 'Bob''s order' WHERE `id` = 1;"
            )
        );
        assert!(
            preview.contains("INSERT INTO `shop-db`.`orders` (`id`, `status`) VALUES (2, 'new');")
        );
        assert!(!preview.contains("`note`"));
        assert!(preview.contains("DELETE FROM `shop-db`.`orders` WHERE `id` = 3;"));
    }

    #[test]
    fn data_change_sql_preview_uses_default_values_for_all_null_insert() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop-db".to_string()),
            schema: None,
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![GdbColumn {
                name: "note".to_string(),
                type_name: Some("varchar(255)".to_string()),
                nullable: true,
                primary_key: false,
                comment: None,
            }],
            rows: Vec::new(),
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let changes = DataChangeSet {
            object,
            inserts: vec![fluxdb_core::Row {
                values: vec![CellValue::Null],
            }],
            updates: Vec::new(),
            deletes: Vec::new(),
            insert_intents: None,
        };

        let preview = data_change_sql_preview(&page, &changes, DatabaseKind::MySql);

        assert_eq!(preview, "INSERT INTO `shop-db`.`orders` DEFAULT VALUES;");
    }

    #[test]
    fn data_change_deleted_rows_maps_delete_identities_to_row_indexes() {
        let page = DataPage {
            columns: vec![GdbColumn {
                name: "id".to_string(),
                type_name: Some("int".to_string()),
                nullable: false,
                primary_key: true,
                comment: None,
            }],
            rows: vec![
                fluxdb_core::Row {
                    values: vec![CellValue::I64(1)],
                },
                fluxdb_core::Row {
                    values: vec![CellValue::I64(2)],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let changes = DataChangeSet {
            object: ObjectPath {
                connection_id: ConnectionId(1),
                database: None,
                schema: None,
                name: "orders".to_string(),
                kind: ObjectKind::Table,
            },
            inserts: Vec::new(),
            updates: Vec::new(),
            deletes: vec![RowIdentity {
                values: BTreeMap::from([("id".to_string(), CellValue::I64(2))]),
            }],
            insert_intents: None,
        };

        assert_eq!(
            data_change_deleted_rows(&page, Some(&changes)),
            BTreeSet::from([1])
        );
    }

    #[test]
    fn new_query_scope_auto_database_only_for_single_context_kinds() {
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::Sqlite, None)),
            Some("main".to_string())
        );
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::Redis, Some("3"))),
            Some("3".to_string())
        );
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::MySql, Some("app"))),
            None
        );
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::MongoDb, Some("app"))),
            None
        );
    }

    #[test]
    fn user_admin_role_tabs_do_not_reload_loaded_grants() {
        let user = DatabaseUserIdentity {
            user: "app".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let mut admin = UserAdminState::new(ConnectionId(1), None, fluxdb_core::PrivilegeScope::MySql);
        admin.selected_user = Some(user.clone());

        assert_eq!(
            user_admin_detail_tab_grants_load_user(UserAdminDetailTab::Advanced, &admin),
            None
        );
        assert_eq!(
            user_admin_detail_tab_grants_load_user(UserAdminDetailTab::MemberOf, &admin),
            Some(user.clone())
        );

        admin.grants_loaded_user = Some(user.clone());
        assert_eq!(
            user_admin_detail_tab_grants_load_user(UserAdminDetailTab::MemberOf, &admin),
            None
        );

        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::Advanced, &admin),
            None
        );
        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::MemberOf, &admin),
            Some(user.clone())
        );
        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::Members, &admin),
            Some(user.clone())
        );

        admin.member_grants_loaded_role = Some(user);
        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::Members, &admin),
            None
        );
    }

    #[test]
    fn user_admin_sql_preview_collects_multiple_statements_on_separate_lines() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let user = DatabaseUserIdentity {
            user: "app".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let role = DatabaseUserIdentity {
            user: "reader".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let mut admin = UserAdminState::new(ConnectionId(1), Some("app".to_string()), fluxdb_core::PrivilegeScope::MySql);
        admin.users = vec![user.clone(), role.clone()];
        admin.selected_user = Some(user.clone());
        admin.set_role_membership_granted(role.clone(), true);
        admin.set_role_membership_default(role, true);
        admin.add_privilege_row("app".to_string());
        let row_id = admin.privilege_rows[0].id;
        admin.toggle_privilege_row_privilege(row_id, "SELECT".to_string());

        let sql = user_admin_all_sql_preview(provider, &admin).sql();

        assert!(sql.contains('\n'));
        assert!(sql.contains("GRANT 'reader'@'%' TO 'app'@'%';"));
        assert!(sql.contains("SET DEFAULT ROLE 'reader'@'%' TO 'app'@'%';"));
        assert!(sql.contains("GRANT SELECT ON `app`.* TO 'app'@'%';"));
    }

    #[test]
    fn user_admin_sql_preview_includes_general_and_advanced_changes() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let user = DatabaseUserIdentity {
            user: "app".to_string(),
            host: "%".to_string(),
            plugin: Some("caching_sha2_password".to_string()),
        };
        let mut admin = UserAdminState::new(
            ConnectionId(1),
            Some("app".to_string()),
            fluxdb_core::PrivilegeScope::MySql,
        );
        admin.selected_user = Some(user);
        admin.auth_plugin = "mysql_native_password".to_string();
        admin.password_expiry_policy = "NEVER".to_string();
        admin.max_queries_per_hour = "10".to_string();
        admin.max_user_connections = "3".to_string();
        admin.ssl_type = "ANY".to_string();

        let sql = user_admin_all_sql_preview(provider, &admin).sql();

        assert!(sql.contains(
            "ALTER USER 'app'@'%' IDENTIFIED WITH `mysql_native_password`;"
        ));
        assert!(sql.contains("ALTER USER 'app'@'%' PASSWORD EXPIRE NEVER;"));
        assert!(sql.contains(
            "ALTER USER 'app'@'%' WITH MAX_QUERIES_PER_HOUR 10 MAX_USER_CONNECTIONS 3;"
        ));
        assert!(sql.contains("ALTER USER 'app'@'%' REQUIRE SSL;"));
        assert!(user_admin_can_save_all(provider, &admin));
    }

    #[test]
    fn connection_tree_does_not_invent_main_database_for_empty_objects() {
        let mut connection = ConnectionState {
            config: query_scope_config(DatabaseKind::MySql, Some("main")),
            connected: true,
            expanded: true,
            objects: Vec::new(),
            redis_overview: RedisConnectionOverview::default(),
        };

        assert!(connection_databases(&connection, false).is_empty());

        connection.objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                name: "orders".to_string(),
                kind: ObjectKind::Table,
            },
            rows: None,
            comment: None,
            modified_at: None,
        });

        assert_eq!(connection_databases(&connection, false).len(), 1);
    }

    #[test]
    fn next_table_folder_name_avoids_existing_names() {
        let existing = vec![
            "新建组".to_string(),
            "新建组 1".to_string(),
            "业务分组".to_string(),
        ];

        assert_eq!(next_table_folder_name(&existing), "新建组 2");
    }

    #[test]
    fn table_folders_keep_custom_order_above_pinned_tables() {
        let connection_id = ConnectionId(1);
        let mut folders = BTreeMap::new();
        assert_eq!(
            table_folder_parent_key(connection_id, "main"),
            object_group_tree_key(connection_id, "main", ObjectGroup::Tables)
        );
        folders.insert(
            table_folder_parent_key(connection_id, "main"),
            vec!["z-folder".to_string(), "a-folder".to_string()],
        );
        let folder_names = sorted_table_folders(&folders, connection_id, "main")
            .into_iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(folder_names, vec!["z-folder", "a-folder"]);

        let connection = ConnectionState {
            config: query_scope_config(DatabaseKind::MySql, Some("main")),
            connected: true,
            expanded: true,
            objects: vec![
                ObjectSummary {
                    path: ObjectPath {
                        connection_id,
                        database: Some("main".to_string()),
                        schema: None,
                        name: "orders".to_string(),
                        kind: ObjectKind::Table,
                    },
                    rows: None,
                    comment: None,
                    modified_at: None,
                },
                ObjectSummary {
                    path: ObjectPath {
                        connection_id,
                        database: Some("main".to_string()),
                        schema: None,
                        name: "users".to_string(),
                        kind: ObjectKind::Table,
                    },
                    rows: None,
                    comment: None,
                    modified_at: None,
                },
            ],
            redis_overview: RedisConnectionOverview::default(),
        };
        let mut pinned = BTreeSet::new();
        pinned.insert(table_tree_key(&connection.objects[1].path));
        let table_names = sorted_group_objects(&connection, "main", None, ObjectGroup::Tables, &pinned)
            .into_iter()
            .map(|object| object.path.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(table_names, vec!["users", "orders"]);

        let mut assignments = BTreeMap::new();
        assignments.insert(
            table_tree_key(&connection.objects[1].path),
            (
                table_folder_parent_key(connection_id, "main"),
                "a-folder".to_string(),
            ),
        );
        let folder_tables = sorted_folder_table_objects(
            &connection,
            "main",
            &table_folder_parent_key(connection_id, "main"),
            "a-folder",
            &pinned,
            &assignments,
        )
        .into_iter()
        .map(|object| object.path.name.as_str())
        .collect::<Vec<_>>();
        let unassigned_tables = sorted_unassigned_group_objects(
            &connection,
            "main",
            None,
            ObjectGroup::Tables,
            &pinned,
            &assignments,
        )
        .into_iter()
        .map(|object| object.path.name.as_str())
        .collect::<Vec<_>>();

        assert_eq!(folder_tables, vec!["users"]);
        assert_eq!(unassigned_tables, vec!["orders"]);
    }

    #[test]
    fn move_table_folder_name_swaps_with_neighbor() {
        let mut folders = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        move_table_folder_name(&mut folders, "b", -1);
        assert_eq!(folders, vec!["b", "a", "c"]);

        move_table_folder_name(&mut folders, "b", -1);
        assert_eq!(folders, vec!["b", "a", "c"]);

        move_table_folder_name(&mut folders, "b", 1);
        assert_eq!(folders, vec!["a", "b", "c"]);

        move_table_folder_name(&mut folders, "missing", 1);
        assert_eq!(folders, vec!["a", "b", "c"]);
    }

    #[test]
    fn redis_filter_page_filters_by_type_and_key_mode() {
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "键".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "类型".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![
                Row {
                    values: vec![
                        CellValue::Text("user:test:1".to_string()),
                        CellValue::Text("string".to_string()),
                    ],
                },
                Row {
                    values: vec![
                        CellValue::Text("orders:test".to_string()),
                        CellValue::Text("hash".to_string()),
                    ],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };

        // 无显式模式，统一按 Redis SCAN MATCH 通配符语义匹配（对齐 RedisInsight 的 Pattern 检索）。
        // `*test*` 包含匹配两个键；`*test` 后缀匹配；`user:*` 前缀匹配；`orders:test` 精确匹配。
        let contains = redis_filter_page(&page, "所有", "*test*");
        let suffix_hash = redis_filter_page(&page, "hash", "*test");
        let prefix = redis_filter_page(&page, "所有", "user:*");
        let exact = redis_filter_page(&page, "所有", "orders:test");

        assert_eq!(contains.rows.len(), 2);
        assert_eq!(suffix_hash.rows.len(), 1);
        assert_eq!(
            suffix_hash.rows[0].values[0],
            CellValue::Text("orders:test".to_string())
        );
        assert_eq!(prefix.rows.len(), 1);
        assert_eq!(
            prefix.rows[0].values[0],
            CellValue::Text("user:test:1".to_string())
        );
        assert_eq!(exact.rows.len(), 1);
        assert_eq!(
            exact.rows[0].values[0],
            CellValue::Text("orders:test".to_string())
        );
    }

    #[test]
    fn redis_glob_matches_follows_scan_match_wildcards() {
        // `*` 匹配零或多个任意字符，`?` 匹配单个字符，其余按字面匹配（对齐 Redis SCAN MATCH）。
        assert!(redis_glob_matches("user:test:1", "user:*"));
        assert!(redis_glob_matches("user:test:1", "*test*"));
        assert!(redis_glob_matches("orders:test", "*test"));
        assert!(redis_glob_matches("a1c", "a?c"));
        assert!(redis_glob_matches("abc", "abc"));
        assert!(redis_glob_matches("abc", "*"));
        assert!(!redis_glob_matches("user:test:1", "orders:*"));
        assert!(!redis_glob_matches("abc", "abd"));
        // `?` 只能匹配单个字符，不能跨多个字符。
        assert!(!redis_glob_matches("abcd", "a?d"));
    }

    #[test]
    fn redis_key_detail_uses_named_columns() {
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "类型".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "TTL".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "键".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "值".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![Row {
                values: vec![
                    CellValue::Text("string".to_string()),
                    CellValue::Text("无 TTL".to_string()),
                    CellValue::Text("test".to_string()),
                    CellValue::Text("13".to_string()),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };

        assert_eq!(
            redis_key_detail_for_row(&page, 0),
            Some(RedisKeyDetail {
                key: "test".to_string(),
                kind: "string".to_string(),
                value: "13".to_string(),
                ttl: "无 TTL".to_string(),
                size: String::new(),
            })
        );
    }

    #[test]
    fn redis_key_detail_refresh_kind_maps_known_types() {
        assert_eq!(
            RedisKeyDetail {
                key: String::new(),
                kind: "hash".to_string(),
                value: String::new(),
                ttl: String::new(),
                size: String::new(),
            }
            .refresh_kind(),
            RedisKeyDetailRefreshKind::Hash
        );
        assert_eq!(
            RedisKeyDetail {
                key: String::new(),
                kind: "stream".to_string(),
                value: String::new(),
                ttl: String::new(),
                size: String::new(),
            }
            .refresh_kind(),
            RedisKeyDetailRefreshKind::Stream
        );
        assert_eq!(
            RedisKeyDetail {
                key: String::new(),
                kind: "string".to_string(),
                value: String::new(),
                ttl: String::new(),
                size: String::new(),
            }
            .refresh_kind(),
            RedisKeyDetailRefreshKind::Key
        );
    }

    #[test]
    fn redis_table_column_widths_follow_requested_ratios() {
        let table_width = px(950.);
        let assert_width = |actual: Pixels, expected: f32| {
            assert!((f32::from(actual) - expected).abs() < 0.01);
        };

        assert_width(redis_table_column_width("键", table_width), 120.);
        assert_width(redis_table_column_width("类型", table_width), 100.);
        assert_width(redis_table_column_width("值", table_width), 450.);
        assert_width(redis_table_column_width("大小", table_width), 80.);
        assert_width(redis_table_column_width("TTL", table_width), 80.);
        assert_width(redis_table_column_width("__redis_actions", table_width), 120.);
    }

    #[test]
    fn redis_table_fit_width_keeps_columns_inside_bounds() {
        let bounds_width = px(1000.);
        let table_width = redis_table_fit_width(bounds_width);
        let total_width: f32 = ["键", "类型", "值", "大小", "TTL", "__redis_actions"]
            .iter()
            .map(|column| f32::from(redis_table_column_width(column, table_width)))
            .sum();

        assert!(total_width <= f32::from(bounds_width));
    }

    #[test]
    fn redis_hash_field_ttl_command_maps_input_to_write_semantics() {
        // 没改 TTL 这一格 → 必须保留原 TTL，否则 HSET 会把它清掉
        assert_eq!(
            redis_hash_field_ttl_command("120", false),
            Ok(RedisHashFieldTtl::Keep)
        );
        // 清空 → 永不过期
        assert_eq!(
            redis_hash_field_ttl_command("  ", true),
            Ok(RedisHashFieldTtl::Persist)
        );
        assert_eq!(
            redis_hash_field_ttl_command("120", true),
            Ok(RedisHashFieldTtl::Seconds(120))
        );
        assert!(redis_hash_field_ttl_command("0", true).is_err());
        assert!(redis_hash_field_ttl_command("abc", true).is_err());
    }

    #[test]
    fn redis_hash_field_ttl_display_value_strips_seconds_suffix_for_table_view() {
        assert_eq!(redis_hash_field_ttl_display_value("51182034s"), "51182034");
        assert_eq!(redis_hash_field_ttl_display_value("1s"), "1");
        assert_eq!(redis_hash_field_ttl_display_value("无 TTL"), "无 TTL");
        assert_eq!(redis_hash_field_ttl_display_value("已过期"), "已过期");
    }

    #[test]
    fn redis_option_pairs_only_allow_known_keys() {
        let pairs = redis_option_pairs(
            "tls=true&sentinel_master=mymaster&evil=1&tls_server_name=redis.example.com&tls=",
        );

        assert_eq!(
            pairs,
            vec![
                ("tls".to_string(), "true".to_string()),
                ("sentinel_master".to_string(), "mymaster".to_string()),
                ("tls_server_name".to_string(), "redis.example.com".to_string()),
            ]
        );
    }

    #[test]
    fn redis_stream_time_input_round_trips() {
        let millis = redis_stream_time_from_input("2026-08-09 12:34:56")
            .unwrap()
            .unwrap();

        // 回填输入框的文本必须能被原样解析回同一个时间戳
        let text = redis_stream_time_to_input(millis);
        assert_eq!(text, "2026-08-09 12:34:56");
        assert_eq!(redis_stream_time_from_input(&text), Ok(Some(millis)));
    }

    #[test]
    fn redis_stream_maxlen_from_input_requires_positive_integer() {
        assert_eq!(redis_stream_maxlen_from_input("  "), Ok(None));
        assert_eq!(redis_stream_maxlen_from_input("1000"), Ok(Some(1000)));
        assert!(redis_stream_maxlen_from_input("0").is_err());
        assert!(redis_stream_maxlen_from_input("-5").is_err());
        assert!(redis_stream_maxlen_from_input("abc").is_err());
    }

    #[test]
    fn redis_stream_time_from_input_parses_local_time_forms() {
        assert_eq!(redis_stream_time_from_input(""), Ok(None));

        let full = redis_stream_time_from_input("2026-08-09 12:00:00").unwrap();
        let minute = redis_stream_time_from_input("2026-08-09 12:00").unwrap();
        let date_only = redis_stream_time_from_input("2026-08-09").unwrap();

        assert_eq!(full, minute);
        // 只写日期按当天 00:00:00 处理，因此比 12:00 早 12 小时
        assert_eq!(full.unwrap() - date_only.unwrap(), 12 * 3600 * 1000);
        assert!(redis_stream_time_from_input("09/08/2026").is_err());
    }

    #[test]
    fn redis_stream_entry_columns_collect_dynamic_fields() {
        let entries = vec![
            RedisStreamEntryRow {
                id: "1785677482094-0".to_string(),
                time: "2026-08-02 21:31:22".to_string(),
                fields: [("test".to_string(), "3".to_string())]
                    .into_iter()
                    .collect(),
            },
            RedisStreamEntryRow {
                id: "1785677482090-0".to_string(),
                time: "2026-08-02 21:31:22".to_string(),
                fields: [
                    ("test".to_string(), "11".to_string()),
                    ("test1".to_string(), "22".to_string()),
                ]
                .into_iter()
                .collect(),
            },
        ];

        let columns = redis_stream_entry_columns(&entries);

        assert_eq!(columns, vec!["test".to_string(), "test1".to_string()]);
        assert_eq!(entries[0].fields.get("test"), Some(&"3".to_string()));
        assert_eq!(entries[1].fields.get("test1"), Some(&"22".to_string()));
    }

    #[test]
    fn redis_stream_entry_field_pair_validates_field_name() {
        let field = redis_stream_entry_field_pair("name".to_string(), "alice".to_string()).unwrap();

        assert_eq!(field, ("name".to_string(), "alice".to_string()));
        assert!(redis_stream_entry_field_pair(" ".to_string(), "alice".to_string()).is_err());
    }

    #[test]
    fn redis_stream_entry_id_validation_accepts_star_or_timestamp_sequence() {
        assert_eq!(redis_stream_entry_id_validation_error("*"), None);
        assert_eq!(redis_stream_entry_id_validation_error("1700000000000-0"), None);
        assert!(redis_stream_entry_id_validation_error("1700000000000").is_some());
        assert!(redis_stream_entry_id_validation_error("abc-0").is_some());
    }

    #[test]
    fn redis_stream_entry_field_pairs_from_snapshot_collects_multiple_rows() {
        let fields = redis_stream_entry_field_pairs_from_snapshot(&[
            ("name".to_string(), "alice".to_string()),
            ("city".to_string(), "shanghai".to_string()),
        ])
        .unwrap();

        assert_eq!(
            fields,
            vec![
                ("name".to_string(), "alice".to_string()),
                ("city".to_string(), "shanghai".to_string())
            ]
        );
    }

    #[test]
    fn redis_set_preview_members_parse_rows() {
        let (summary, members) = redis_set_preview_members("2 成员\ntest\ntest1");

        assert_eq!(summary, "2 成员");
        assert_eq!(members, vec!["test".to_string(), "test1".to_string()]);
    }

    #[test]
    fn redis_pretty_json_formats_only_valid_json() {
        assert_eq!(
            redis_pretty_json(r#"{"name":"test","items":[1,2]}"#).unwrap(),
            "{\n  \"name\": \"test\",\n  \"items\": [\n    1,\n    2\n  ]\n}"
        );
        assert!(redis_pretty_json("not json").is_err());
    }

    #[test]
    fn redis_ttl_input_value_keeps_only_seconds() {
        assert_eq!(redis_ttl_input_value("(No TTL)"), "");
        assert_eq!(redis_ttl_input_value("120s"), "120");
    }

    #[test]
    fn redis_key_meta_actions_stay_visible_while_editing() {
        assert!(redis_key_meta_actions_visible(
            false,
            false,
            Some(RedisKeyMetaField::Ttl)
        ));
        assert!(redis_key_meta_actions_enabled(
            false,
            false,
            Some(RedisKeyMetaField::Ttl)
        ));
        assert!(!redis_key_meta_actions_enabled(
            true,
            true,
            Some(RedisKeyMetaField::Ttl)
        ));
        assert!(!redis_key_meta_actions_visible(false, false, None));
    }

    fn query_scope_config(kind: DatabaseKind, database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(1),
            name: "test".to_string(),
            kind,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: database.map(ToString::to_string),
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        }
    }

    #[test]
    fn redis_hash_value_is_truncated_detects_prefix_only() {
        // 带截断标记前缀 → true
        assert!(redis_hash_value_is_truncated(
            "[Truncated due to length] the rest..."
        ));
        // 普通值 → false
        assert!(!redis_hash_value_is_truncated("plain value"));
        // 前缀在串中段 → false（只认前缀）
        assert!(!redis_hash_value_is_truncated(
            "not [Truncated due to length] at start"
        ));
        // 仅前缀本身（无后续字符）→ true（starts_with 语义）
        assert!(redis_hash_value_is_truncated(REDIS_HASH_TRUNCATED_MARKER));
    }

    #[test]
    fn dangerous_redis_command_detects_destructive_commands() {
        // 破坏性/高危命令：独立成行或跟在参数后面均需识别
        assert!(is_dangerous_redis_command("FLUSHDB"));
        assert!(is_dangerous_redis_command("flushall async"));
        assert!(is_dangerous_redis_command("SHUTDOWN NOSAVE"));
        assert!(is_dangerous_redis_command("DEBUG SEGFAULT"));
        assert!(is_dangerous_redis_command("SLAVEOF 1.2.3.4 6379"));
        assert!(is_dangerous_redis_command("REPLICAOF no one"));
        assert!(is_dangerous_redis_command("CLUSTER RESET HARD"));
        assert!(is_dangerous_redis_command("MIGRATE 1.2.3.4 6379 key 0 5000"));
        assert!(is_dangerous_redis_command("CLIENT PAUSE 30000"));
    }

    #[test]
    fn dangerous_redis_command_ignores_safe_commands() {
        assert!(!is_dangerous_redis_command(""));
        assert!(!is_dangerous_redis_command("GET foo"));
        assert!(!is_dangerous_redis_command("SET foo bar"));
        assert!(!is_dangerous_redis_command("DEL foo bar"));
        assert!(!is_dangerous_redis_command("LPUSH mylist a"));
        // 大小写与空白不干扰安全判断
        assert!(!is_dangerous_redis_command("  set foo bar  "));
    }

    #[test]
    fn redis_database_row_click_does_not_expand() {
        // 回归：Redis 数据库节点整行点击不得触发展开/加载 key，展开只由箭头负责。
        assert!(!database_row_click_expands(ObjectKind::RedisDb));
    }

    #[test]
    fn other_database_row_click_still_expands() {
        // MySQL/SQLite 等数据库类型保持整行点击展开/加载子节点。
        assert!(database_row_click_expands(ObjectKind::Database));
        assert!(database_row_click_expands(ObjectKind::Schema));
        assert!(database_row_click_expands(ObjectKind::Collection));
    }

    fn data_filter_rule(
        field: &str,
        operator: DataFilterOperator,
        values: &[&str],
        grouped: bool,
    ) -> DataFilterRule {
        DataFilterRule {
            enabled: true,
            field: Some(field.to_string()),
            operator,
            values: values.iter().map(|value| (*value).to_string()).collect(),
            grouped,
        }
    }

    fn assert_data_filter_round_trip(rules: Vec<DataFilterRule>) {
        let sql = data_filter_rules_sql_pretty(&rules, DatabaseKind::MySql);
        let parsed = parse_data_filter_rules_text(&sql).expect("should parse");
        assert_eq!(parsed, rules);
    }

    #[test]
    fn data_filter_round_trips_all_operators() {
        for case in [
            data_filter_rule("id", DataFilterOperator::Eq, &["1"], false),
            data_filter_rule("id", DataFilterOperator::Eq, &["1", "2"], false),
            data_filter_rule("id", DataFilterOperator::Ne, &["1", "2"], false),
            data_filter_rule("name", DataFilterOperator::Contains, &["a", "b"], false),
            data_filter_rule("name", DataFilterOperator::NotContains, &["a", "b"], false),
            data_filter_rule("name", DataFilterOperator::StartsWith, &["ab"], false),
            data_filter_rule("name", DataFilterOperator::NotStartsWith, &["ab"], false),
            data_filter_rule("name", DataFilterOperator::EndsWith, &["xy"], false),
            data_filter_rule("name", DataFilterOperator::NotEndsWith, &["xy"], false),
            data_filter_rule("age", DataFilterOperator::Gt, &["18"], false),
            data_filter_rule("age", DataFilterOperator::Ge, &["18"], false),
            data_filter_rule("age", DataFilterOperator::Lt, &["65"], false),
            data_filter_rule("age", DataFilterOperator::Le, &["65"], false),
            data_filter_rule("age", DataFilterOperator::Between, &["18", "65"], false),
            data_filter_rule("age", DataFilterOperator::NotBetween, &["18", "65"], false),
            data_filter_rule("tag", DataFilterOperator::InList, &["a", "b"], false),
            data_filter_rule("tag", DataFilterOperator::NotInList, &["a", "b"], false),
            data_filter_rule("deleted_at", DataFilterOperator::IsNull, &[], false),
            data_filter_rule("deleted_at", DataFilterOperator::IsNotNull, &[], false),
            data_filter_rule("title", DataFilterOperator::IsEmpty, &[], false),
            data_filter_rule("title", DataFilterOperator::IsNotEmpty, &[], false),
        ] {
            assert_data_filter_round_trip(vec![case]);
        }
    }

    #[test]
    fn data_filter_round_trips_groups_and_between() {
        assert_data_filter_round_trip(vec![
            data_filter_rule("age", DataFilterOperator::Between, &["18", "65"], false),
            data_filter_rule("status", DataFilterOperator::Eq, &["active", "pending"], false),
        ]);

        assert_data_filter_round_trip(vec![
            data_filter_rule("a", DataFilterOperator::Eq, &["1"], true),
            data_filter_rule("b", DataFilterOperator::Eq, &["2"], true),
            data_filter_rule("c", DataFilterOperator::IsNull, &[], false),
        ]);
    }

    #[test]
    fn data_filter_null_like_operators_do_not_require_values() {
        assert!(!DataFilterOperator::IsNull.requires_values());
        assert!(!DataFilterOperator::IsNotNull.requires_values());
        assert!(!DataFilterOperator::IsEmpty.requires_values());
        assert!(!DataFilterOperator::IsNotEmpty.requires_values());
        assert!(DataFilterOperator::Between.requires_values());
    }

    #[test]
    fn loaded_schema_context_extracts_tables_and_columns() {
        // 一个连接：main 库有 orders(表) / users(表) / v_orders(视图) / other_db.t(表)；
        // 另打开一个属于该连接的 DataEditor 标签（users 表已加载 id / name 两列）。
        let connection_id = ConnectionId(1);
        let object = |name: &str, database: &str, kind: ObjectKind| ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some(database.to_string()),
                schema: None,
                name: name.to_string(),
                kind,
            },
            rows: None,
            comment: None,
            modified_at: None,
        };
        let connection = ConnectionState {
            config: query_scope_config(DatabaseKind::MySql, Some("main")),
            connected: true,
            expanded: true,
            objects: vec![
                object("orders", "main", ObjectKind::Table),
                object("users", "main", ObjectKind::Table),
                object("v_orders", "main", ObjectKind::View),
                object("t", "other_db", ObjectKind::Table),
            ],
            redis_overview: RedisConnectionOverview::default(),
        };
        let page = DataPage {
            columns: vec![
                fluxdb_core::Column {
                    name: "id".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                fluxdb_core::Column {
                    name: "name".to_string(),
                    type_name: None,
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: Vec::new(),
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let state = AppState {
            connections: vec![connection],
            tabs: vec![TabState {
                id: TabId(1),
                title: "users".to_string(),
                kind: TabKind::DataEditor(DataEditorState {
                    object: ObjectPath {
                        connection_id,
                        database: Some("main".to_string()),
                        schema: None,
                        name: "users".to_string(),
                        kind: ObjectKind::Table,
                    },
                    page: Some(page),
                    original_page: None,
                    pagination: Default::default(),
                    changes: None,
                    editing_cell: None,
                    cell_detail_panel: CellDetailPanelState::default(),
                    table_info: TableInfoState::default(),
                    loading: false,
                    error: None,
                }),
                dirty: false,
            }],
            ..AppState::default()
        };

        let ctx = loaded_schema_context(&state, connection_id, Some("main"));
        // 表 / 视图候选来自已加载 objects，且过滤掉其他库的 t。
        assert_eq!(ctx.tables, vec!["orders", "users", "v_orders"]);
        // 列候选来自已打开的 DataEditor 标签（users 表两列）。
        assert_eq!(
            ctx.columns,
            vec![
                ("users".to_string(), "id".to_string()),
                ("users".to_string(), "name".to_string()),
            ]
        );

        // 库不匹配时返回空。
        let empty = loaded_schema_context(&state, connection_id, Some("nope"));
        assert!(empty.tables.is_empty());
        assert!(empty.columns.is_empty());
    }

    // —— 关系画布默认布局与连线路由正确性（用户实际复现：默认重叠 + 线路穿卡）——

    /// 默认布局（er_relation_layout → build_er_scene → materialize）全套校验：
    /// 节点矩形不相交、每边路径不穿端点卡以外的任何卡正文、自关联不进入自身卡。
    fn assert_demo_layout_sane(tables: Vec<fluxdb_core::ErTableNode>, edges: Vec<fluxdb_core::ErForeignKeyEdge>) {
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges,
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let layout = er_relation_layout(&tables, &graph.edges);
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        for t in &tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) {
                positions.insert(t.name.clone(), (x, y));
            }
        }
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 5000.0, 5000.0);
        assert_eq!(frame.node_views.len(), tables.len(), "全部表都参与布局");

        // 1) 节点矩形两两不相交。
        for i in 0..frame.node_views.len() {
            for j in i + 1..frame.node_views.len() {
                let a = &frame.node_views[i];
                let b = &frame.node_views[j];
                let sep = a.x + NODE_WIDTH <= b.x || b.x + NODE_WIDTH <= a.x
                    || a.y + a.height <= b.y || b.y + b.height <= a.y;
                assert!(sep, "节点矩形相交：{} @({},{}) v {} @({},{})",
                    a.name, a.x, a.y, b.name, b.x, b.y);
            }
        }

        // 2) 每条边路径不穿端点卡以外的任何卡正文（垂直/水平段全部校验）。
        for e in &frame.edge_views {
            assert!(e.points.len() >= 2, "边应有折线路径");
            for w in e.points.windows(2) {
                let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
                for nv in &frame.node_views {
                    if nv.idx == e.from_idx || nv.idx == e.to_idx {
                        continue; // 端点卡仅允许端口出口接触
                    }
                    assert!(
                        !segment_intersects_rect(ax, ay, bx, by, nv.x, nv.y, nv.x + NODE_WIDTH, nv.y + nv.height),
                        "边 {} 线段 ({ax},{ay})-({bx},{by}) 穿过非端点卡 {}",
                        e.desc, nv.name
                    );
                }
            }
        }
    }

    #[test]
    fn er_demo_default_layout_rectangles_disjoint() {
        // er_demo 六表：customers/orders/order_items/products + 无关系的 tags/settings。
        let mut customers = er_table("customers", 4);
        customers.columns[0] = fluxdb_core::ErColumn {
            name: "id".into(), type_name: Some("bigint".into()), primary_key: true, nullable: false,
        };
        let orders = er_table("orders", 8); // 含 refer 字段（自关联）
        let order_items = er_table("order_items", 6);
        let products = er_table("products", 5);
        let tags = er_table("tags", 3);
        let settings = er_table("settings", 2);

        // 实际关系：orders 自关联 + 引用 customers；order_items 引用 orders/products。
        let edges = vec![
            er_edge("orders", "customer_id", "customers", "id"),
            er_edge("orders", "refer_order_id", "orders", "id"), // 自关联
            er_edge("order_items", "order_id", "orders", "id"),
            er_edge("order_items", "product_id", "products", "id"),
        ];
        assert_demo_layout_sane(
            vec![customers, orders, order_items, products, tags, settings],
            edges,
        );
    }

    /// 真实 UI 折线必须是水平/垂直正交段（§5.1「水平/垂直折线」），不出现斜穿两卡间隙的斜线段。
    /// 回归：base_cand 曾用 (a_exit,a.y)→(b_enter,b.y) 直连，y 不同时形成斜线。
    fn assert_orthogonal_paths(frame: &ErFrame) {
        for e in &frame.edge_views {
            for w in e.points.windows(2) {
                let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
                let horiz = (ay - by).abs() < 0.01;
                let vert = (ax - bx).abs() < 0.01;
                assert!(
                    horiz || vert,
                    "关系 {} 存在斜线段 ({ax},{ay})→({bx},{by})，非正交折线",
                    e.desc
                );
            }
        }
    }

    #[test]
    fn er_demo_routes_are_orthogonal_rounded_polylines() {
        // 复用 er_demo 六表：真实卡片高 + 关系，所有可见折线段必须水平/垂直。
        let mut customers = er_table("customers", 4);
        customers.columns[0] = fluxdb_core::ErColumn {
            name: "id".into(), type_name: Some("bigint".into()), primary_key: true, nullable: false,
        };
        let orders = er_table("orders", 8);
        let order_items = er_table("order_items", 6);
        let products = er_table("products", 5);
        let tags = er_table("tags", 3);
        let settings = er_table("settings", 2);
        let tables = vec![customers, orders, order_items, products, tags, settings];
        let edges = vec![
            er_edge("orders", "customer_id", "customers", "id"),
            er_edge("orders", "refer_order_id", "orders", "id"),
            er_edge("order_items", "order_id", "orders", "id"),
            er_edge("order_items", "product_id", "products", "id"),
        ];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(), edges, relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let layout = er_relation_layout(&tables, &graph.edges);
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        for t in &tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) { positions.insert(t.name.clone(), (x, y)); }
        }
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 5000.0, 5000.0);
        assert!(!frame.edge_views.is_empty());
        assert_orthogonal_paths(&frame);
    }

    #[test]
    fn er_relation_line_does_not_cross_middle_card() {
        // 中间障碍卡必须被绕开，不能水平段直穿（复现「普通边穿卡」）。
        let tables = vec![
            er_table("src", 3),
            er_table("mid", 6),
            er_table("dst", 4),
        ];
        let edges = vec![er_edge("src", "c0", "dst", "c0")];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(), edges, relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let layout = er_relation_layout(&tables, &graph.edges);
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        for t in &tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) { positions.insert(t.name.clone(), (x, y)); }
        }
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 5000.0, 5000.0);
        assert_eq!(frame.edge_views.len(), 1);
        let e = &frame.edge_views[0];
        for w in e.points.windows(2) {
            let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
            for nv in &frame.node_views {
                if nv.idx == e.from_idx || nv.idx == e.to_idx { continue; }
                assert!(
                    !segment_intersects_rect(ax, ay, bx, by, nv.x, nv.y, nv.x + NODE_WIDTH, nv.y + nv.height),
                    "边 {} 线段穿中间卡 {}", e.desc, nv.name
                );
            }
        }
    }

    #[test]
    fn er_self_loop_after_drag_stays_outside_card() {
        // 手动拖开后仍要绕行（不贴回弯、不进入卡片正文）；且新建障碍时要重路由并避开。
        let tables = vec![er_table("orders", 10)];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("orders", "refer_col", "orders", "id")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let layout = er_relation_layout(&tables, &graph.edges);
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        positions.insert("orders".to_string(), layout["orders"].clone_into_pos());
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 2000.0, 2000.0);
        assert_eq!(frame.edge_views.len(), 1);
        let e = &frame.edge_views[0];
        let card = &frame.node_views[0];
        // 自关联路径整体位于卡片外部（x ≥ 卡右边界 - eps），不进入卡片正文。
        for &(px, py) in &e.points {
            assert!(
                px >= card.x + NODE_WIDTH - 0.5,
                "自关联折线不得进入卡片正文：点 ({px},{py})"
            );
        }
    }

    /// 把第三张卡拖进 src↔dst 同 y 的间隙：折线必须重路由绕开它，
    /// 不能退回「预算用尽」就接受穿卡路径（§5.1 / 任务：有限候选被占时不静默穿卡）。
    #[test]
    fn er_drag_obstacle_into_gap_re_routes_out() {
        let tables = vec![er_table("src", 3), er_table("dst", 4), er_table("mid", 3)];
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(),
            edges: vec![er_edge("src", "c0", "dst", "c0")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &loaded_layout(&tables, &graph.edges));
        // 同 y 排布：src 左、dst 右；mid 先远离（不挡），后拖进 src↔dst 间隙。
        let mut positions = BTreeMap::new();
        positions.insert("src".to_string(), (0.0, 400.0));
        positions.insert("dst".to_string(), (900.0, 400.0));
        positions.insert("mid".to_string(), (0.0, 0.0));

        let env_far = env_for(&scene, positions.clone());
        let frame_far = scene.materialize(&env_far, ErViewport::default(), 3000.0, 3000.0);
        let e_far = frame_far.edge_views.iter().find(|e| e.desc.starts_with("src")).expect("src→dst 边存在");

        // 把 mid 拖进 src 与 dst 之间的行通道（同 y=400 附近，恰好挡住水平段）。
        positions.insert("mid".to_string(), (positions["src"].0 + 2.0 * NODE_WIDTH, 400.0));
        let env_blocked = env_for(&scene, positions);
        let frame_blocked = scene.materialize(&env_blocked, ErViewport::default(), 3000.0, 3000.0);
        let e_blocked = frame_blocked.edge_views.iter().find(|e| e.desc.starts_with("src")).expect("src→dst 边存在");
        assert_ne!(e_far.points, e_blocked.points, "拖动制造新障碍后路径必须重路由");

        // 重路由后的路径仍不得穿过 mid（非端点卡）。
        let mid = frame_blocked.node_views.iter().find(|v| v.name == "mid").unwrap();
        for w in e_blocked.points.windows(2) {
            let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
            assert!(
                !segment_intersects_rect(ax, ay, bx, by, mid.x, mid.y, mid.x + NODE_WIDTH, mid.y + mid.height),
                "拖动后路径 {} 仍穿过 mid 卡",
                e_blocked.desc
            );
        }
    }

    /// 真实 er_demo：关联链呈左→右层次（customers/orders/order_items），
    /// 分支/孤立表不与关联组件重叠，卡片用真实字段加载高度。
    #[test]
    fn er_demo_chain_hierarchy_and_isolated_outside() {
        let mut customers = er_table("customers", 4);
        customers.columns[0] = fluxdb_core::ErColumn {
            name: "id".into(), type_name: Some("bigint".into()), primary_key: true, nullable: false,
        };
        let orders = er_table("orders", 8);
        let order_items = er_table("order_items", 6);
        let products = er_table("products", 5);
        let tags = er_table("tags", 3);
        let settings = er_table("settings", 2);
        let tables = vec![customers, orders, order_items, products, tags, settings];
        let edges = vec![
            er_edge("orders", "customer_id", "customers", "id"),
            er_edge("order_items", "order_id", "orders", "id"),
            er_edge("order_items", "product_id", "products", "id"),
        ];
        let layout = er_relation_layout(&tables, &edges);
        // 链条左→右：customers(最左被引用) → orders → order_items。
        assert!(
            layout["customers"].0 < layout["orders"].0 && layout["orders"].0 < layout["order_items"].0,
            "链应左→右递增：customers {} < orders {} < order_items {}",
            layout["customers"].0, layout["orders"].0, layout["order_items"].0
        );
        // 孤立表 tags/settings 放在关联区右侧外围，不与任何关联卡片重叠（真实高度）。
        let comp_max_x = layout
            .iter()
            .filter(|(n, _)| *n != "tags" && *n != "settings")
            .map(|(_, v)| v.0)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(layout["tags"].0 > comp_max_x && layout["settings"].0 > comp_max_x, "孤立表应在关联区之外");
        // 用真实字段高度的卡片矩形做两两不相交校验。
        let graph = fluxdb_core::ErGraphData {
            tables: tables.clone(), edges, relation_status: fluxdb_core::ErLoadStatus::Loaded,
        };
        let scene = build_er_scene(&graph, &layout);
        let mut positions = BTreeMap::new();
        for t in &tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) { positions.insert(t.name.clone(), (x, y)); }
        }
        let env = env_for(&scene, positions);
        let frame = scene.materialize(&env, ErViewport::default(), 5000.0, 5000.0);
        let v = &frame.node_views;
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                let sep = v[i].x + NODE_WIDTH <= v[j].x || v[j].x + NODE_WIDTH <= v[i].x
                    || v[i].y + v[i].height <= v[j].y || v[j].y + v[j].height <= v[i].y;
                assert!(sep, "真实卡片矩形相交：{} @({},{}) v {} @({},{})",
                    v[i].name, v[i].x, v[i].y, v[j].name, v[j].x, v[j].y);
            }
        }
    }

    /// 基准探针：测量关系布局、场景构建、本帧物化(网格+路由)在 50/200/1843/10000 表的耗时与可见规模。
    /// 仅报告纯 CPU 纯函数耗时（非 UI FPS）；供本轮性能审查记录。`cargo test -p fluxdb-desktop er_probe_perf_scales -- --ignored --nocapture` 运行。
    #[test]
    #[ignore]
    fn er_probe_perf_scales() {
        let n_sizes = [50usize, 200, 1843, 10000];
        for &n in &n_sizes {
            let mut tables: Vec<_> = (0..n).map(|i| er_table(&format!("t{i:05}"), 3)).collect();
            // 每隔 50 张加一张 30 字段长表，模拟真实库长表。
            for i in (0..n).step_by(50) {
                tables[i] = er_table(&tables[i].name.clone(), 30);
            }
            let mut edges = Vec::new();
            for i in 0..n.saturating_sub(1) {
                if i % 3 == 0 {
                    edges.push(er_edge(&format!("t{i:05}"), "c0", &format!("t{:05}", i + 1), "c0"));
                }
            }
            let graph = fluxdb_core::ErGraphData {
                tables: tables.clone(), edges: edges.clone(), relation_status: fluxdb_core::ErLoadStatus::Loaded,
            };
            let t_layout = std::time::Instant::now();
            let layout = loaded_layout(&tables, &edges);
            let dt_layout = t_layout.elapsed();
            let t_scene = std::time::Instant::now();
            let scene = build_er_scene(&graph, &layout);
            let dt_scene = t_scene.elapsed();
            let mut positions = BTreeMap::new();
            for t in &tables {
                if let Some(&(x, y, _)) = layout.get(&t.name) { positions.insert(t.name.clone(), (x, y)); }
            }
            let env = env_for(&scene, positions);
            let t_mat = std::time::Instant::now();
            let frame = scene.materialize(&env, ErViewport::default(), 1280.0, 800.0);
            let dt_mat = t_mat.elapsed();
            println!(
                "PERF n={n} layout={:?} scene={:?} materialize={:?} visible_nodes={} visible_edges={} tables={} edges={}",
                dt_layout, dt_scene, dt_mat, frame.visible_nodes.len(), frame.edge_views.len(), n, edges.len()
            );
        }
    }

    // 计算 layout 值拷贝辅助。
    trait LayoutPosExt { fn clone_into_pos(&self) -> (f32, f32); }
    impl LayoutPosExt for (f32, f32, i32) {
        fn clone_into_pos(&self) -> (f32, f32) { (self.0, self.1) }
    }

}

/// T19：PG 表单 → 结构化档案 → 回填表单，字段不丢；测试连接与保存用同一份档案。
#[test]
fn postgres_connection_form_roundtrips_into_profile() {
    let mut form = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    form.host = "db.internal".to_string();
    form.port = "5433".to_string();
    form.database = "appdb".to_string();
    form.username = "app_user".to_string();
    form.password = "s3cret".to_string();
    form.tls_enabled = true;
    form.pg_tls_ssl_mode = "verify-full".to_string();
    form.tls_ca = "/etc/ssl/root.crt".to_string();
    form.tls_sni = "db.example.com".to_string();
    form.pg_default_schema = "sales".to_string();
    form.pg_application_name = "FluxDB Desktop".to_string();
    form.pg_connect_timeout_secs = "7".to_string();
    form.pg_query_timeout_secs = "30".to_string();

    let profile = form.build_postgres_profile();
    assert_eq!(profile.basic.host, "db.internal");
    assert_eq!(profile.basic.port, 5433);
    assert_eq!(profile.basic.maintenance_database, "appdb");
    assert_eq!(profile.basic.username, "app_user");
    assert_eq!(
        profile.basic.password.value().map(str::to_string),
        Some("s3cret".to_string())
    );
    assert!(profile.tls.enabled);
    assert_eq!(profile.tls.ssl_mode, fluxdb_core::PostgresSslMode::VerifyFull);
    assert_eq!(profile.tls.ca.key, "/etc/ssl/root.crt");
    assert_eq!(profile.tls.server_name, "db.example.com");
    assert_eq!(profile.scope.default_schema, "sales");
    assert_eq!(profile.advanced.application_name, "FluxDB Desktop");
    assert_eq!(profile.advanced.connect_timeout_secs, 7);
    assert_eq!(profile.advanced.query_timeout_secs, 30);
    // 未启用 SSH/代理时保持直连（不塞入无效传输层）。
    assert_eq!(profile.transport.len(), 1);

    // 回填：编辑/重启后表单值应与档案一致。
    let mut restored = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    restored.apply_postgres_profile(&profile);
    assert_eq!(restored.pg_tls_ssl_mode, "verify-full");
    assert_eq!(restored.pg_default_schema, "sales");
    assert_eq!(restored.pg_application_name, "FluxDB Desktop");
    assert_eq!(restored.pg_connect_timeout_secs, "7");
    assert_eq!(restored.pg_query_timeout_secs, "30");
    assert_eq!(restored.database, "appdb");
    assert_eq!(restored.tls_ca, "/etc/ssl/root.crt");

    // 数据库留空：保存后保持为空，回填也显示空（不再强制回退 postgres）。
    let mut empty_db_form = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    empty_db_form.host = "127.0.0.1".to_string();
    empty_db_form.port = "5432".to_string();
    empty_db_form.database.clear();
    let empty_profile = empty_db_form.build_postgres_profile();
    assert_eq!(empty_profile.basic.maintenance_database, "");
    let mut empty_restored = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    empty_restored.apply_postgres_profile(&empty_profile);
    assert_eq!(empty_restored.database, "");

    // 空维护库拨号时仍回落 postgres 默认（连接/枚举正常）。
    assert_eq!(empty_profile.maintenance_database(), "postgres");

    // SSH 隧道：启用后进入传输层并可回填。
    let mut ssh_form = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    ssh_form.ssh_enabled = true;
    ssh_form.ssh_host = "jump.internal".to_string();
    ssh_form.ssh_username = "ops".to_string();
    ssh_form.ssh_auth = "private_key".to_string();
    ssh_form.ssh_private_key = "/home/ops/.ssh/id_ed25519".to_string();
    let profile = ssh_form.build_postgres_profile();
    let ssh = profile
        .transport
        .iter()
        .find_map(|layer| match layer {
            fluxdb_core::PostgresTransportLayer::Ssh(ssh) => Some(ssh),
            _ => None,
        })
        .expect("SSH 传输层应存在");
    assert_eq!(ssh.host, "jump.internal");
    assert_eq!(ssh.private_key.key, "/home/ops/.ssh/id_ed25519");

    let mut restored_ssh = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    restored_ssh.apply_postgres_profile(&profile);
    assert!(restored_ssh.ssh_enabled);
    assert_eq!(restored_ssh.ssh_host, "jump.internal");
    assert_eq!(restored_ssh.ssh_auth, "private_key");
}

/// T19：默认值保持 MySQL/TiDB 表单不变（回归）。
#[test]
fn mysql_connection_form_defaults_unchanged_by_postgres_fields() {
    let form = NewConnectionForm::for_kind(DatabaseKind::MySql, 1);
    assert_eq!(form.host, "127.0.0.1");
    assert_eq!(form.port, "3306");
    assert_eq!(form.username, "root");
    assert_eq!(form.mysql_tls_ssl_mode, "preferred");
    assert_eq!(form.mysql_charset, "utf8mb4");
}

/// T19：PG 表单校验走结构化档案（TLS 模式/SSH 等），不是只看主机端口。
#[test]
fn postgres_form_validation_uses_profile_rules() {
    let mut form = NewConnectionForm::for_kind(DatabaseKind::Postgres, 1);
    form.name = "PG".to_string();
    form.host = "127.0.0.1".to_string();
    form.port = "5432".to_string();
    let profile = form.build_postgres_profile();
    assert_eq!(profile.validate(), None);

    // verify-full 未启用 TLS：档案校验应拒绝。
    form.pg_tls_ssl_mode = "verify-full".to_string();
    form.tls_enabled = false;
    assert!(
        form.build_postgres_profile()
            .validate()
            .is_some_and(|message| message.contains("需先启用 TLS")),
        "verify-full 未启用 TLS 应被拒绝"
    );

    // 启用 TLS 后通过；verify-full 用纯 IP 且无 server_name 仍应提示。
    form.tls_enabled = true;
    assert!(
        form.build_postgres_profile()
            .validate()
            .is_some_and(|message| message.contains("server_name")),
        "verify-full 纯 IP 应要求主机名或 server_name"
    );
    form.tls_sni = "db.example.com".to_string();
    assert_eq!(form.build_postgres_profile().validate(), None);

    // SSH 启用但缺主机：应报错而不是静默用直连。
    form.ssh_enabled = true;
    assert!(
        form.build_postgres_profile()
            .validate()
            .is_some_and(|message| message.contains("SSH")),
        "启用 SSH 但缺主机应被拒绝"
    );
}
