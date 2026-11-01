// MongoDB 侧边栏 → 集合工作台的端到端用例（真实生产视图，无头驱动）。
//
// 只走 UI 层可见路径：双击连接 → 展开库 → 展开「集合」分组 → 点击集合行。
// 连接器是真实 `MongoConnector`，因此需要 `FLUXDB_MONGO_URI`；未设置时跳过。
// 凭据只从环境变量进内存，不落文件、不打印。

mod mongo_sidebar_tests {
    use std::collections::BTreeMap;

    use fluxdb_app::{AppCommand, AppController, TabKind};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoConnectionProfile, ObjectKind,
        Settings,
    };
    use fluxdb_storage::{FileStorage, Storage as _};
    use gpui::{
        AppContext as _, Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext,
        px,
    };
    use gpui_component::Root;

    use crate::{
        MongoDocumentEditorMode, NavicatMain, app_assets_base_path, extend_component_i18n,
        init_i18n, navicat_main_new,
    };

    /// `debug_bounds` 只接受 `&'static str`；测试里按需把格式化后的选择器泄漏成静态串。
    fn selector(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    /// 从 `FLUXDB_MONGO_URI` 构造真实连接配置；未设置时返回 `None`（用例跳过）。
    fn live_connection() -> Option<ConnectionConfig> {
        let uri = std::env::var("FLUXDB_MONGO_URI").ok()?;
        let uri = uri.trim();
        if uri.is_empty() {
            return None;
        }
        let profile = MongoConnectionProfile::from_uri(uri)
            .unwrap_or_else(|error| panic!("FLUXDB_MONGO_URI 解析失败: {error}"));
        Some(ConnectionConfig {
            id: ConnectionId(7_701),
            name: "MongoDB live".into(),
            kind: DatabaseKind::MongoDb,
            endpoint: Endpoint::Tcp {
                host: profile
                    .basic
                    .hosts
                    .first()
                    .map(|host| host.host.clone())
                    .unwrap_or_default(),
                port: profile
                    .basic
                    .hosts
                    .first()
                    .map(|host| host.port)
                    .unwrap_or(27_017),
                database: Some(profile.basic.default_database.clone()),
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: Some(profile),
            oracle_profile: None,
        })
    }

    /// 目标库：URI 默认库 → `FLUXDB_MONGO_DB` → `fluxdb_demo`。
    fn live_database(config: &ConnectionConfig) -> String {
        config
            .mongo_profile
            .as_ref()
            .map(|profile| profile.basic.default_database.clone())
            .filter(|database| !database.is_empty())
            .or_else(|| std::env::var("FLUXDB_MONGO_DB").ok())
            .unwrap_or_else(|| "fluxdb_demo".to_string())
    }

    /// 推进测试时钟直到 `probe` 返回 `Some`；用于等后台连接/查询任务落地。
    fn wait_until<R>(
        cx: &mut VisualTestContext,
        label: &str,
        mut probe: impl FnMut(&mut VisualTestContext) -> Option<R>,
    ) -> R {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            if let Some(value) = probe(cx) {
                return value;
            }
            assert!(std::time::Instant::now() < deadline, "等待超时：{label}");
            std::thread::yield_now();
        }
    }

    /// 把文档虚拟列表滚到目标卡片进入视口。
    ///
    /// 列表只渲染视口附近的卡片，视口外的卡片**根本不构建**，所以「找到第 N 张卡片的按钮」
    /// 必须先像用户一样滚动过去，不能假设整页 DOM 都在。
    ///
    /// 「在窗口内」还不够：列表是 `overflow_hidden` 的裁剪区，贴着视口下沿的卡片
    /// 边界仍在窗口内、但绘制与命中都被裁掉，点击会被静默丢弃。所以判定用视口缩小
    /// 一圈后的矩形；滚不动时（已到列表末尾）才退回窗口判定，避免误报。
    fn scroll_card_into_view(cx: &mut VisualTestContext, target: &'static str) {
        let viewport = redraw_and_bounds(cx, selector("mongo-card-list".to_string()))
            .expect("文档区视口必须渲染");
        let inside = |cx: &mut VisualTestContext, rect: gpui::Bounds<gpui::Pixels>| {
            redraw_and_bounds(cx, target).is_some_and(|bounds| rect.contains(&bounds.center()))
        };
        // 留 8px 余量：滚动是一格一格推进的，正好压线仍可能被裁剪。
        let inset = gpui::px(8.);
        let safe_viewport = gpui::Bounds {
            origin: viewport.origin + gpui::point(inset, inset),
            size: gpui::size(
                viewport.size.width - inset * 2.,
                viewport.size.height - inset * 2.,
            ),
        };
        let window_bounds = cx.update(|window, _| window.bounds());
        let wheel = |cx: &mut VisualTestContext, lines: f32| {
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: viewport.center(),
                delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
                ..Default::default()
            });
            cx.run_until_parked();
        };
        if inside(cx, safe_viewport) {
            return;
        }
        // 两个方向各扫一遍：先把列表推到一端，再逐格推向另一端，目标在上方或下方都能找到。
        for (reset, step) in [(10., -3.), (-10., 3.)] {
            for _ in 0..60 {
                wheel(cx, reset);
            }
            for _ in 0..150 {
                if inside(cx, safe_viewport) {
                    return;
                }
                wheel(cx, step);
            }
        }
        // 卡片比视口还高时，贴边是滚动的极限，按钮可能永远进不了安全区：退回窗口判定。
        if inside(cx, window_bounds) {
            return;
        }
        panic!("滚动到底仍未渲染：{target}");
    }

    /// 第 0 张 JSON 卡片当前渲染出的 gutter 行数（含可折叠行与普通行）。
    fn json_card_visible_rows(cx: &mut VisualTestContext) -> usize {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let mut rows = 0;
        for line in 0..200 {
            // 选择器由 `json_fold_view` 按 `{前缀}:{行}` / `{前缀}-plain:{行}` 生成，
            // 前缀是 `mongo-json-fold:0`，所以普通行是 `mongo-json-fold:0-plain:N`。
            let fold = selector(format!("mongo-json-fold:0:{line}"));
            let plain = selector(format!("mongo-json-fold:0-plain:{line}"));
            if cx.debug_bounds(fold).is_some() || cx.debug_bounds(plain).is_some() {
                rows += 1;
            }
        }
        rows
    }

    /// 重绘并返回指定选择器的边界（渲染后的命中区域）。
    fn redraw_and_bounds(
        cx: &mut VisualTestContext,
        selector: &'static str,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.debug_bounds(selector)
    }

    /// 元素是否**真的可见**：有非零尺寸，且中心点落在窗口内。
    ///
    /// `debug_bounds` 对零尺寸或窗口外元素同样返回 `Some`，只用它判「存在」会把
    /// 「浮层被裁掉 / 缩成一条线」当成通过——实测就是这样漏掉过一次：候选算出来了，
    /// 浮层却落在窗口下方（面板比窗口高，绝对定位以画布底边为基准）。
    fn visible_bounds(
        cx: &mut VisualTestContext,
        selector: &'static str,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        let bounds = redraw_and_bounds(cx, selector)?;
        if f32::from(bounds.size.width) < 1.0 || f32::from(bounds.size.height) < 1.0 {
            return None;
        }
        let window_bounds = cx.update(|window, _| window.bounds());
        window_bounds.contains(&bounds.center()).then_some(bounds)
    }

    /// 起一个隔离 profile 的生产视图，返回 `NavicatMain` 与可视化测试上下文。
    ///
    /// 每个用例独立临时目录：共享 profile 会让后台存储任务跨用例互相干扰。
    fn launch_app(
        cx: &mut TestAppContext,
        config: ConnectionConfig,
    ) -> (gpui::Entity<NavicatMain>, &mut VisualTestContext) {
        cx.background_executor.allow_parking();
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let settings = Settings {
            locale: "en".into(),
            show_sidebar: true,
            ..Default::default()
        };
        storage.save_settings(&settings).expect("settings");
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        let mut controller = AppController::new();
        controller.set_ai_storage(storage.clone());
        controller.dispatch(AppCommand::SaveSettings(settings));
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
            // 终端键位平时由 app boot 的 register_shortcuts 注册；用例直接建视图，
            // 少了这一步回车不会变成 `TermEnter`，命令敲不进去。
            crate::register_terminal_shortcuts(cx);
        });
        let profile = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut app = navicat_main_new(
                    window,
                    cx,
                    controller,
                    profile,
                    gpui_component::ThemeMode::Light,
                );
                // 跳过首帧会话恢复，避免后台存储任务越过用例边界。
                app.ai_chat.recovery_pending = false;
                app
            });
            Root::new(view, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(950.)));
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("root")
                .expect("typed root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production view")
        });
        (view, cx)
    }

    /// 双击连接行并等库行出现（单击只展开/收起，不建连）。
    fn connect_and_wait_database(
        cx: &mut VisualTestContext,
        connection_id: ConnectionId,
        database: &str,
    ) -> &'static str {
        let connection_row =
            redraw_and_bounds(cx, selector(format!("connection-row:{}", connection_id.0)))
                .expect("连接行必须渲染");
        cx.simulate_event(MouseDownEvent {
            position: connection_row.center(),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        let database_selector = selector(format!("database-row:{database}"));
        wait_until(cx, "连接打开", |cx| {
            redraw_and_bounds(cx, database_selector)
        });
        database_selector
    }

    /// 当前终端 grid 的可视文本（渲染唯一事实来源，等于用户看到的内容）。
    fn shell_screen(cx: &mut VisualTestContext, view: &gpui::Entity<NavicatMain>) -> String {
        cx.update(|_, cx| {
            view.read(cx)
                .terminal_sessions
                .values()
                .next()
                .map(|session| session.read(cx).grid.visible_lines().join("\n"))
                .unwrap_or_default()
        })
    }

    /// 在终端里敲一行并回车（走真实键盘事件与输入 handler）。
    fn type_in_terminal(cx: &mut VisualTestContext, line: &str) {
        cx.simulate_input(line);
        cx.simulate_keystrokes("enter");
    }

    /// 敲一行并等回显上屏。
    ///
    /// 补全读的是 grid 里的回显，而 REPL 输出要等 30ms 泵才进 grid。敲完立刻按 Tab
    /// 会按空输入行算候选（全量），测试必须像人一样先看到自己敲的字。
    fn type_in_terminal_echo(
        cx: &mut VisualTestContext,
        view: &gpui::Entity<NavicatMain>,
        line: &str,
    ) {
        cx.simulate_input(line);
        wait_until(cx, "输入回显", |cx| {
            shell_screen(cx, view).contains(line).then_some(())
        });
    }

    /// 去掉所有空白后的终端内容。
    ///
    /// grid 里每个宽字符后面跟一个填充空格（占 2 单元），直接按原文匹配中文会永远不中；
    /// 断言中文提示时统一用这一版。
    fn shell_screen_compact(cx: &mut VisualTestContext, view: &gpui::Entity<NavicatMain>) -> String {
        shell_screen(cx, view)
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect()
    }

    #[gpui::test]
    fn mongo_database_expands_and_collection_opens_workspace(cx: &mut TestAppContext) {
        let Some(config) = live_connection() else {
            eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
            return;
        };
        let connection_id = config.id;
        let database = live_database(&config);
        let (view, cx) = launch_app(cx, config);

        // 1. 双击连接行 = 建立连接（单击只展开/收起）。
        let database_selector = connect_and_wait_database(cx, connection_id, &database);

        // 2. 点击库行展开集合分组。
        let database_row = redraw_and_bounds(cx, database_selector).expect("库行必须渲染");
        cx.simulate_click(database_row.center(), Modifiers::default());
        let group_selector = selector(format!("object-group-row:collections:{database}"));
        wait_until(cx, "库展开", |cx| redraw_and_bounds(cx, group_selector));

        // 3. 展开「集合」分组，等集合行出现。
        let group_row = redraw_and_bounds(cx, group_selector).expect("集合分组行必须渲染");
        cx.simulate_click(group_row.center(), Modifiers::default());
        let collection = wait_until(cx, "集合列表加载", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == connection_id)
                    .and_then(|connection| {
                        connection
                            .objects
                            .iter()
                            .find(|object| {
                                object.path.kind == ObjectKind::Collection
                                    && object.path.database.as_deref() == Some(database.as_str())
                            })
                            .map(|object| object.path.name.clone())
                    })
            })
        });

        // 4. 点击集合行，工作台必须真的打开并渲染出来。
        let collection_row =
            redraw_and_bounds(cx, selector(format!("table-tree-row:{collection}")))
                .expect("集合行必须渲染");
        cx.simulate_click(collection_row.center(), Modifiers::default());
        let page_selector = selector(format!("mongo-collection-page:{database}/{collection}"));
        wait_until(cx, "集合页渲染", |cx| {
            redraw_and_bounds(cx, page_selector)
        });

        // 5. 首屏查询结果必须落到标签页状态（值由真实服务端返回）。
        wait_until(cx, "集合首屏查询", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.clone(),
                        _ => None,
                    })
                    .filter(|page| page.returned > 0)
            })
        });

        // 6. 文档卡片默认展开成字段列表；点折叠箭头后正文消失（原型 / Compass 行为）。
        let fields_selector = selector("mongo-document-fields:0".to_string());
        wait_until(cx, "文档字段列表渲染", |cx| {
            redraw_and_bounds(cx, fields_selector)
        });
        let toggle_selector = selector("mongo-document-toggle:0".to_string());
        let toggle = redraw_and_bounds(cx, toggle_selector).expect("折叠箭头必须渲染");
        cx.simulate_click(toggle.center(), Modifiers::default());
        wait_until(cx, "文档折叠", |cx| {
            (redraw_and_bounds(cx, fields_selector).is_none()).then_some(())
        });
        let toggle = redraw_and_bounds(cx, toggle_selector).expect("折叠后箭头仍在");
        cx.simulate_click(toggle.center(), Modifiers::default());
        wait_until(cx, "文档重新展开", |cx| {
            redraw_and_bounds(cx, fields_selector)
        });

        // 6.1 整条卡头都能折叠：点 `_id` 文字（不是箭头）也要生效。
        let head = redraw_and_bounds(cx, selector("mongo-document-head:0".to_string()))
            .expect("卡片头必须渲染");
        cx.simulate_click(head.center(), Modifiers::default());
        wait_until(cx, "点卡头折叠", |cx| {
            (redraw_and_bounds(cx, fields_selector).is_none()).then_some(())
        });
        let head = redraw_and_bounds(cx, selector("mongo-document-head:0".to_string()))
            .expect("折叠后卡片头仍在");
        cx.simulate_click(head.center(), Modifiers::default());
        wait_until(cx, "点卡头重新展开", |cx| {
            redraw_and_bounds(cx, fields_selector)
        });

        // 7. 切到 JSON 展示：仍是逐条文档卡片，正文换成带行号的 JSON（不是整页一个数组）。
        let json_tab = selector("mongo-display-tab:Json".to_string());
        let json_tab_bounds = redraw_and_bounds(cx, json_tab).expect("JSON 展示页签必须渲染");
        cx.simulate_click(json_tab_bounds.center(), Modifiers::default());
        let json_selector = selector("mongo-document-json:0".to_string());
        wait_until(cx, "JSON 卡片正文渲染", |cx| {
            redraw_and_bounds(cx, json_selector)
        });
        assert!(
            redraw_and_bounds(cx, selector("mongo-document-toggle:0".to_string())).is_some(),
            "JSON 展示复用同一张卡片头"
        );
        assert!(
            redraw_and_bounds(cx, fields_selector).is_none(),
            "JSON 展示不再渲染字段列表"
        );

        // 7.1 默认态与 Compass 一致：根节点展开、嵌套节点全部收起。
        // 折叠后行号会重建，所以直接读标签页状态里的折叠集合判断默认态。
        let default_folds = cx.update(|_, cx| {
            let main = view.read(cx);
            main.controller
                .state()
                .tabs
                .iter()
                .find(|tab| matches!(tab.kind, TabKind::MongoCollection(_)))
                .map(|tab| {
                    main.mongo_json_fold_toggles
                        .get(&tab.id)
                        .cloned()
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        });
        assert!(default_folds.is_empty(), "默认态不应有任何被点开的节点");
        let collapsed_rows = json_card_visible_rows(cx);
        // 默认态下嵌套的 object / array 都收成单行，可见行数一定少于展开态。
        let window_bounds = cx.update(|window, _| window.bounds());
        // 跳过第 0 行：那是根对象，默认就是展开的，点它只会收起。
        let fold_selector = (1..200)
            .map(|line| selector(format!("mongo-json-fold:0:{line}")))
            .find(|candidate| {
                redraw_and_bounds(cx, candidate)
                    .is_some_and(|bounds| window_bounds.contains(&bounds.center()))
            })
            .expect("JSON 视图至少有一个嵌套可折叠节点在窗口内");

        // 7.2 点开一个嵌套节点后，可见行数必须增加（子节点展开）。
        let fold_bounds = redraw_and_bounds(cx, fold_selector).expect("折叠行必须渲染");
        cx.simulate_click(fold_bounds.center(), Modifiers::default());
        let expanded_rows = wait_until(cx, "JSON 节点展开", |cx| {
            redraw_and_bounds(cx, fold_selector);
            let rows = json_card_visible_rows(cx);

            (rows > collapsed_rows).then_some(rows)
        });
        assert!(
            expanded_rows > collapsed_rows,
            "展开后行数必须增加：{collapsed_rows} → {expanded_rows}"
        );

        // 还原成文档展示，避免后续步骤依赖当前展示方式。
        let documents_tab =
            redraw_and_bounds(cx, selector("mongo-display-tab:Documents".to_string()))
                .expect("文档展示页签必须渲染");
        cx.simulate_click(documents_tab.center(), Modifiers::default());
        wait_until(cx, "切回文档展示", |cx| {
            redraw_and_bounds(cx, fields_selector)
        });

        // 8. 表格展示：整表带 `_id` 列，单元格按「行:列」渲染（Compass 表格语义）。
        let table_tab = redraw_and_bounds(cx, selector("mongo-display-tab:Table".to_string()))
            .expect("表格展示页签必须渲染");
        cx.simulate_click(table_tab.center(), Modifiers::default());
        wait_until(cx, "表格视图渲染", |cx| {
            redraw_and_bounds(cx, selector("mongo-document-table".to_string()))
        });
        assert!(
            redraw_and_bounds(cx, selector("mongo-table-cell:0:id".to_string())).is_some(),
            "表格首行必须渲染 _id 单元格"
        );
        // 操作列固定在表格右侧：表格视图同样有四个操作按钮，且首行删除入口在窗口内。
        assert!(
            redraw_and_bounds(cx, selector("mongo-document-delete:0".to_string()))
                .is_some_and(|bounds| window_bounds.contains(&bounds.center())),
            "表格首行必须渲染可见的删除按钮（固定操作列）"
        );
        let copy_button = redraw_and_bounds(cx, selector("mongo-document-copy:0".to_string()))
            .expect("表格首行必须渲染复制按钮");
        assert!(
            window_bounds.contains(&copy_button.center()),
            "复制按钮中心点必须在窗口内，否则点击会被丢弃"
        );
        // 写剪贴板是无声操作：必须给出可观察反馈，否则用户不知道复制成功没有。
        cx.simulate_click(copy_button.center(), Modifiers::default());
        wait_until(cx, "复制后出现提示", |cx| {
            cx.debug_bounds("app-message-overlay")
        });
        assert!(
            redraw_and_bounds(cx, fields_selector).is_none(),
            "表格展示不再渲染文档卡片"
        );
        let documents_tab =
            redraw_and_bounds(cx, selector("mongo-display-tab:Documents".to_string()))
                .expect("文档展示页签必须渲染");
        cx.simulate_click(documents_tab.center(), Modifiers::default());
        wait_until(cx, "切回文档展示", |cx| {
            redraw_and_bounds(cx, fields_selector)
        });

        // 9. 嵌套字段默认收起成 `{ N fields }` / `[ N items ]`；点开后才出现子行。
        // 候选取自真实返回的文档（不假设夹具结构），并要求摘要行落在窗口内才点击——
        // 越界点击会被静默丢弃。
        let window = cx.update(|window, _| window.bounds());
        let candidates = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find_map(|tab| match &tab.kind {
                    TabKind::MongoCollection(state) => state.page.as_ref(),
                    _ => None,
                })
                .map(|page| {
                    page.documents
                        .iter()
                        .flat_map(|document| {
                            let fields = serde_json::from_str::<serde_json::Value>(&document.json)
                                .ok()
                                .and_then(|json| json.as_object().cloned())
                                .unwrap_or_default();
                            fields
                                .into_iter()
                                .filter(|(_, value)| {
                                    matches!(value, serde_json::Value::Array(_))
                                        || matches!(
                                            value,
                                            serde_json::Value::Object(map)
                                                if !map.keys().next().is_some_and(|k| k.starts_with('$'))
                                        )
                                })
                                .map(|(key, _)| (document.id.clone(), key))
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        let Some((document_id, nested_key)) = candidates.into_iter().find(|(id, key)| {
            let selector = selector(format!("mongo-field-toggle:{id}/{key}"));
            redraw_and_bounds(cx, selector).is_some_and(|bounds| window.contains(&bounds.center()))
        }) else {
            eprintln!("跳过嵌套字段断言：本页没有落在窗口内的可展开字段");
            return;
        };
        let nested_selector = selector(format!("mongo-field-toggle:{document_id}/{nested_key}"));
        let nested = redraw_and_bounds(cx, nested_selector).expect("嵌套摘要行必须渲染");
        let children_selector =
            selector(format!("mongo-field-children:{document_id}/{nested_key}"));
        assert!(
            redraw_and_bounds(cx, children_selector).is_none(),
            "嵌套字段默认收起"
        );
        cx.simulate_click(nested.center(), Modifiers::default());
        wait_until(cx, "嵌套字段展开", |cx| {
            redraw_and_bounds(cx, children_selector)
        });

        // 9.1 未保存切换需要两张卡片：演示库的集合可能只剩一篇文档，
        // 这里临时插一条固定 `_id` 的探针文档（先删后插，重复运行也幂等），结尾删除。
        let switch_document_id = "fluxdb-e2e-switch";
        let mongo_tab = cx
            .update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| matches!(tab.kind, TabKind::MongoCollection(_)))
                    .map(|tab| tab.id)
            })
            .expect("集合标签页仍在");
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(
                            r#"{{ "delete": "{collection}", "deletes": [ {{ "q": {{ "_id": "{switch_document_id}" }}, "limit": 1 }} ] }}"#
                        ),
                    },
                    cx,
                );
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(
                            r#"{{ "insert": "{collection}", "documents": [ {{ "_id": "{switch_document_id}", "name": "fluxdb e2e switch", "count": 1 }} ] }}"#
                        ),
                    },
                    cx,
                );
                this.run_mongo_query(mongo_tab, cx);
            });
        });
        wait_until(cx, "探针文档落库", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .is_some_and(|page| {
                        page.documents
                            .iter()
                            .any(|document| document.id == switch_document_id)
                            && page.documents.len() >= 2
                    })
            })
            .then_some(())
        });

        // 10. 删除：走真实 UI 按钮 → 确认框 → 服务端删除；删完文档从本页消失。
        // 用自己的探针文档，绝不删演示库里的真实数据。
        let delete_document_id = "fluxdb-e2e-delete";
        let delete_tab = cx
            .update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| matches!(tab.kind, TabKind::MongoCollection(_)))
                    .map(|tab| tab.id)
            })
            .expect("集合标签页仍在");
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(
                            r#"{{ "delete": "{collection}", "deletes": [ {{ "q": {{ "_id": "{delete_document_id}" }}, "limit": 1 }} ] }}"#
                        ),
                    },
                    cx,
                );
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(
                            r#"{{ "insert": "{collection}", "documents": [ {{ "_id": "{delete_document_id}", "name": "fluxdb e2e delete" }} ] }}"#
                        ),
                    },
                    cx,
                );
                this.run_mongo_query(delete_tab, cx);
            });
        });
        let delete_index = wait_until(cx, "待删探针文档落库", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .and_then(|page| {
                        page.documents
                            .iter()
                            .position(|document| document.id == delete_document_id)
                    })
            })
        });
        // 卡片很高，先把展开的卡片收起，让目标卡片的删除按钮落进窗口。
        for _ in 0..8 {
            let mut collapsed_one = false;
            for index in 0..8 {
                if redraw_and_bounds(cx, selector(format!("mongo-document-fields:{index}")))
                    .is_none()
                {
                    continue;
                }
                let Some(toggle) =
                    redraw_and_bounds(cx, selector(format!("mongo-document-toggle:{index}")))
                        .filter(|bounds| window_bounds.contains(&bounds.center()))
                else {
                    continue;
                };
                cx.simulate_click(toggle.center(), Modifiers::default());
                collapsed_one = true;
                break;
            }
            if !collapsed_one {
                break;
            }
        }
        // 列表只渲染视口内的卡片：目标不在首屏时要先滚动过去。
        let delete_selector = selector(format!("mongo-document-delete:{delete_index}"));
        scroll_card_into_view(cx, delete_selector);
        let delete_button =
            redraw_and_bounds(cx, delete_selector).expect("待删文档的删除按钮必须渲染");
        cx.simulate_click(delete_button.center(), Modifiers::default());
        // 删除必须先确认：确认框出现前点删除不能发出写请求。
        wait_until(cx, "删除确认框出现", |cx| {
            redraw_and_bounds(cx, selector("mongo-document-delete-confirm".to_string()))
        });
        let page_has_delete_probe = |cx: &mut VisualTestContext| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .is_some_and(|page| {
                        page.documents
                            .iter()
                            .any(|document| document.id == delete_document_id)
                    })
            })
        };
        assert!(
            page_has_delete_probe(cx),
            "确认框出现时文档必须还在（没点确认不能删）"
        );
        // 先走一次「取消」：确认框必须能真的取消删除。
        let cancel_button =
            redraw_and_bounds(cx, selector("mongo-document-delete-cancel".to_string()))
                .expect("删除确认框的取消按钮必须渲染");
        cx.simulate_click(cancel_button.center(), Modifiers::default());
        wait_until(cx, "取消后确认框消失", |cx| {
            (redraw_and_bounds(cx, selector("mongo-document-delete-modal".to_string())).is_none())
                .then_some(())
        });
        assert!(page_has_delete_probe(cx), "取消后文档必须还在");

        // 再来一次并确认：这次才真的删。
        scroll_card_into_view(cx, delete_selector);
        let delete_button =
            redraw_and_bounds(cx, delete_selector).expect("取消后删除按钮仍可点");
        cx.simulate_click(delete_button.center(), Modifiers::default());
        let confirm_button = wait_until(cx, "删除确认框再次出现", |cx| {
            redraw_and_bounds(cx, selector("mongo-document-delete-confirm".to_string()))
        });
        cx.simulate_click(confirm_button.center(), Modifiers::default());
        wait_until(cx, "文档已从本页删除", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .is_some_and(|page| {
                        page.documents
                            .iter()
                            .all(|document| document.id != delete_document_id)
                    })
            })
            .then_some(())
        });

        // 10.1 复制文档：点按钮开抽屉（内容不含 `_id`）→ 确认 → 服务端生成新 `_id` 插一条。
        // 仍然只碰自己的探针文档。
        let duplicate_document_id = "fluxdb-e2e-duplicate";
        let duplicate_name = "fluxdb e2e duplicate";
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(
                            r#"{{ "delete": "{collection}", "deletes": [ {{ "q": {{ "name": "{duplicate_name}" }}, "limit": 0 }} ] }}"#
                        ),
                    },
                    cx,
                );
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(
                            r#"{{ "insert": "{collection}", "documents": [ {{ "_id": "{duplicate_document_id}", "name": "{duplicate_name}", "n": 7 }} ] }}"#
                        ),
                    },
                    cx,
                );
                this.run_mongo_query(delete_tab, cx);
            });
        });
        let duplicate_index = wait_until(cx, "复制用探针文档落库", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .and_then(|page| {
                        page.documents
                            .iter()
                            .position(|document| document.id == duplicate_document_id)
                    })
            })
        });
        let duplicate_selector = selector(format!("mongo-document-duplicate:{duplicate_index}"));
        scroll_card_into_view(cx, duplicate_selector);
        let duplicate_button =
            redraw_and_bounds(cx, duplicate_selector).expect("复制文档按钮必须渲染");
        cx.simulate_click(duplicate_button.center(), Modifiers::default());
        // 复制与编辑共用右侧抽屉（不再弹居中弹框），选择器区分模式。
        let duplicate_drawer = selector("mongo-document-duplicate-drawer".to_string());
        wait_until(cx, "复制抽屉出现", |cx| {
            redraw_and_bounds(cx, duplicate_drawer)
        });
        // 抽屉里的初值必须已经剥掉 `_id`：带着源 `_id` 插入必然撞唯一键。
        let draft_input = cx
            .update(|_, cx| {
                view.read(cx)
                    .mongo_document_editor
                    .as_ref()
                    .filter(|editor| editor.mode == MongoDocumentEditorMode::Duplicate)
                    .and_then(|editor| editor.component.input().cloned())
            })
            .expect("复制抽屉必须持有编辑框");
        let draft = cx.read(|cx| draft_input.read(cx).value().to_string());
        assert!(!draft.contains("_id"), "复制抽屉的初值不能含 _id: {draft}");

        // 先点一次取消：抽屉关闭且不写入。
        let duplicate_cancel =
            redraw_and_bounds(cx, selector("mongo-document-editor-cancel".to_string()))
                .expect("复制抽屉的取消按钮必须渲染");
        cx.simulate_click(duplicate_cancel.center(), Modifiers::default());
        wait_until(cx, "取消后复制抽屉消失", |cx| {
            (redraw_and_bounds(cx, duplicate_drawer).is_none()).then_some(())
        });

        // 再打开并确认：这次真的插入。新文档由服务端生成 `_id`，所以按名字数条数。
        scroll_card_into_view(cx, duplicate_selector);
        let duplicate_button =
            redraw_and_bounds(cx, duplicate_selector).expect("取消后复制按钮仍可点");
        cx.simulate_click(duplicate_button.center(), Modifiers::default());
        let duplicate_confirm = wait_until(cx, "复制抽屉再次出现", |cx| {
            redraw_and_bounds(cx, selector("mongo-document-duplicate-confirm".to_string()))
        });
        cx.simulate_click(duplicate_confirm.center(), Modifiers::default());
        wait_until(cx, "复制出的新文档落库", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .is_some_and(|page| {
                        let copies: Vec<&fluxdb_core::MongoDocument> = page
                            .documents
                            .iter()
                            .filter(|document| document.json.contains(duplicate_name))
                            .collect();
                        // 原文档 + 副本：两条，且副本的 `_id` 不是源文档的 `_id`。
                        copies.len() == 2
                            && copies
                                .iter()
                                .any(|document| document.id != duplicate_document_id)
                    })
            })
            .then_some(())
        });

        // 10.2 结果条的「+」：打开同一副抽屉，初值只有一个本机生成的 `_id`；
        // 补一个字段后插入，服务端必须按这个 `_id` 落库（否则界面预填的 id 是假的）。
        let insert_name = "fluxdb e2e insert";
        let insert_button = redraw_and_bounds(cx, selector("mongo-results-insert".to_string()))
            .expect("结果条的新增按钮必须渲染");
        assert!(
            window_bounds.contains(&insert_button.center()),
            "新增按钮必须落在窗口内，否则点击会被丢弃"
        );
        cx.simulate_click(insert_button.center(), Modifiers::default());
        let insert_drawer = selector("mongo-document-insert-drawer".to_string());
        wait_until(cx, "新增抽屉出现", |cx| {
            redraw_and_bounds(cx, insert_drawer)
        });
        let insert_input = cx
            .update(|_, cx| {
                view.read(cx)
                    .mongo_document_editor
                    .as_ref()
                    .filter(|editor| editor.mode == MongoDocumentEditorMode::Insert)
                    .and_then(|editor| editor.component.input().cloned())
            })
            .expect("新增抽屉必须持有编辑框");
        let insert_draft = cx.read(|cx| insert_input.read(cx).value().to_string());
        let insert_draft_json: serde_json::Value =
            serde_json::from_str(&insert_draft).expect("新增初值必须是合法 JSON");
        let insert_id = insert_draft_json["_id"]["$oid"]
            .as_str()
            .expect("新增初值的 _id 必须是 $oid 包装")
            .to_string();
        assert_eq!(
            insert_draft_json.as_object().map(|fields| fields.len()),
            Some(1),
            "新增初值只应有 _id: {insert_draft}"
        );

        // 补一个字段后插入：这一篇必须真的按预填的 `_id` 落库。
        let insert_ready = format!(
            "{{\n  \"_id\": {{\n    \"$oid\": \"{insert_id}\"\n  }},\n  \"name\": \"{insert_name}\"\n}}"
        );
        cx.update(|window, cx| {
            insert_input.update(cx, |input, cx| {
                input.set_value(insert_ready.clone(), window, cx);
                input.focus(window, cx);
            });
            window.draw(cx).clear(cx);
        });
        let insert_confirm = wait_until(cx, "新增按钮可用", |cx| {
            redraw_and_bounds(cx, selector("mongo-document-insert-confirm".to_string()))
        });
        cx.simulate_click(insert_confirm.center(), Modifiers::default());
        wait_until(cx, "新增的文档落库", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .is_some_and(|page| {
                        page.documents
                            .iter()
                            .any(|document| document.id == insert_id)
                    })
            })
            .then_some(())
        });
        wait_until(cx, "新增后抽屉关闭", |cx| {
            (redraw_and_bounds(cx, insert_drawer).is_none()).then_some(())
        });

        // 10.3 结果条的导入按钮：真实点击打开弹窗；未选文件时「开始导入」必须禁用，
        // 关闭后弹窗消失。选文件走系统原生对话框，无头环境无法驱动，因此解析与写入
        // 由连接器 live 用例覆盖（`mongo_live_import_documents_from_csv_and_json`）。
        let import_button = redraw_and_bounds(cx, selector("mongo-results-import".to_string()))
            .expect("结果条的导入按钮必须渲染");
        assert!(
            window_bounds.contains(&import_button.center()),
            "导入按钮必须落在窗口内，否则点击会被丢弃"
        );
        cx.simulate_click(import_button.center(), Modifiers::default());
        let import_modal = selector("mongo-import-modal".to_string());
        wait_until(cx, "导入弹窗出现", |cx| {
            redraw_and_bounds(cx, import_modal)
        });
        // 未选文件：主按钮禁用。断言它存在但不可点，避免「点了没反应」被当成通过。
        assert!(
            redraw_and_bounds(cx, selector("mongo-import-start".to_string())).is_some(),
            "导入弹窗必须有开始按钮"
        );
        assert!(
            cx.update(|_, cx| {
                view.read(cx)
                    .pending_mongo_import
                    .as_ref()
                    .map(|form| form.source.is_none() && form.preview.is_none())
            })
            .unwrap_or(false),
            "未选文件时弹窗不应有预览"
        );
        // 分隔符是「标签 + 控件」两行、开关只有一行，两者必须底部对齐；
        // 居中对齐会把开关推到标签和控件之间，看起来像浮在空处。
        let delimiter_field = redraw_and_bounds(
            cx,
            selector("mongo-import-delimiter-field".to_string()),
        )
        .expect("分隔符字段必须渲染");
        let empty_null_toggle = redraw_and_bounds(
            cx,
            selector("mongo-import-empty-null".to_string()),
        )
        .expect("空值开关必须渲染");
        let bottom_gap = (delimiter_field.bottom() - empty_null_toggle.bottom()).abs();
        assert!(
            bottom_gap <= gpui::px(2.),
            "空值开关必须与分隔符下拉框底部对齐，实际相差 {bottom_gap:?}"
        );
        // 与表数据导入一致要有「常规 / 消息日志」两个页签：日志页只读，
        // 写入动作留在常规页，因此用按钮集合而不是纯状态断言它真的换了页。
        let log_tab = wait_until(cx, "消息日志页签", |cx| {
            redraw_and_bounds(cx, selector("import-dialog-tab:Log".to_string()))
        });
        assert!(
            redraw_and_bounds(cx, selector("import-dialog-tab:General".to_string())).is_some(),
            "导入弹窗必须有常规页签"
        );
        cx.simulate_click(log_tab.center(), Modifiers::default());
        wait_until(cx, "切到消息日志页", |cx| {
            (redraw_and_bounds(cx, selector("mongo-import-copy-log".to_string())).is_some()
                && redraw_and_bounds(cx, selector("mongo-import-start".to_string())).is_none())
            .then_some(())
        });
        assert!(
            redraw_and_bounds(cx, selector("mongo-import-start".to_string())).is_none(),
            "日志页只应保留复制日志与关闭，不该出现开始导入"
        );
        let general_tab = redraw_and_bounds(cx, selector("import-dialog-tab:General".to_string()))
            .expect("常规页签仍在");
        cx.simulate_click(general_tab.center(), Modifiers::default());
        wait_until(cx, "切回常规页", |cx| {
            redraw_and_bounds(cx, selector("mongo-import-start".to_string()))
        });
        let import_close = wait_until(cx, "导入弹窗关闭按钮", |cx| {
            redraw_and_bounds(cx, selector("mongo-import-close".to_string()))
        });
        cx.simulate_click(import_close.center(), Modifiers::default());
        wait_until(cx, "导入弹窗关闭", |cx| {
            (redraw_and_bounds(cx, import_modal).is_none()).then_some(())
        });

        // 10.4 创建集合：库行右键 → 菜单 → 填名 → 确认，新集合必须**自动**出现在树里，
        // 不需要用户手动刷新。这条正是「创建集合后没刷新该库」的回归点：控制器只替换
        // 第一层（库列表），UI 必须自己重取该库子节点。
        let created_collection = format!("fluxdb_ui_probe_{}", std::process::id());
        let database_row = redraw_and_bounds(cx, database_selector).expect("库行仍在");
        cx.simulate_event(MouseDownEvent {
            position: database_row.center(),
            modifiers: Modifiers::default(),
            button: MouseButton::Right,
            click_count: 1,
            first_mouse: false,
        });
        let new_collection_item = wait_until(cx, "库右键菜单出现", |cx| {
            redraw_and_bounds(
                cx,
                selector("database-menu-item:Create collection".to_string()),
            )
        });
        cx.simulate_click(new_collection_item.center(), Modifiers::default());
        wait_until(cx, "创建集合弹窗", |cx| {
            redraw_and_bounds(cx, selector("mongo-create-collection-modal".to_string()))
        });
        let _ = redraw_and_bounds(cx, selector("mongo-create-collection-input".to_string()))
            .expect("集合名输入框");
        // 弹窗打开时已把焦点给输入框；这里不能再点外层容器——那层包着标签，
        // 点中心很容易落在标签/间隙上，反而把焦点从输入框移走，模拟输入就丢了。
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.simulate_input(&created_collection);
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert_eq!(
            cx.update(|_, cx| {
                view.read(cx)
                    .create_collection_input
                    .read(cx)
                    .value()
                    .to_string()
            }),
            created_collection,
            "集合名没写进输入框"
        );
        let confirm =
            redraw_and_bounds(cx, selector("mongo-create-collection-confirm".to_string()))
                .expect("确认按钮");
        cx.simulate_click(confirm.center(), Modifiers::default());
        // 不点刷新、不重开连接：对象树里的集合清单必须靠创建成功后的自动重取更新。
        // 先断言状态层真的拿到了新集合（这才是「刷新」的实质），再看界面行。
        wait_until(cx, "对象树状态出现新集合", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == connection_id)
                    .filter(|connection| {
                        connection.objects.iter().any(|object| {
                            object.path.kind == ObjectKind::Collection
                                && object.path.name == created_collection
                                && object.path.database.as_deref() == Some(database.as_str())
                        })
                    })
                    .map(|connection| connection.objects.len())
            })
        });
        wait_until(cx, "新集合行渲染", |cx| {
            redraw_and_bounds(cx, selector(format!("table-tree-row:{created_collection}")))
        });
        // 清理探针集合（与第 12 步一致，直接发服务端命令），避免污染真实库。
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::MongoRunCommand {
                        connection_id,
                        database: database.clone(),
                        command: format!(r#"{{ "drop": "{created_collection}" }}"#),
                    },
                    cx,
                );
            });
        });

        // 11. 编辑入口：卡头编辑按钮打开抽屉即编辑器；改一处值 → 保存 → 抽屉关闭、
        // 重新查询后服务端返回新值。用真实服务端确认写入真的落地，而不只是界面状态变化。
        let editor_selector = selector("mongo-document-editor".to_string());
        assert!(
            redraw_and_bounds(cx, editor_selector).is_none(),
            "初始不应有编辑抽屉"
        );
        let (document_id, before_json) = first_document(cx, &view);
        let Some(edited_json) = edit_first_scalar(&before_json) else {
            eprintln!("跳过编辑断言：首篇文档没有可改写的顶层标量字段");
            return;
        };
        // 卡片正文很高，先把展开着的卡片逐个收起，让每张卡片的卡头（含编辑按钮）都落进窗口——
        // 越界点击会被静默丢弃。每收起一张后面的卡片会上移，所以每轮重新扫。
        for _ in 0..8 {
            let mut collapsed_one = false;
            for index in 0..8 {
                let fields_open =
                    redraw_and_bounds(cx, selector(format!("mongo-document-fields:{index}")))
                        .is_some();
                if !fields_open {
                    continue;
                }
                let Some(toggle) =
                    redraw_and_bounds(cx, selector(format!("mongo-document-toggle:{index}")))
                        .filter(|bounds| window_bounds.contains(&bounds.center()))
                else {
                    continue;
                };
                cx.simulate_click(toggle.center(), Modifiers::default());
                collapsed_one = true;
                break;
            }
            if !collapsed_one {
                break;
            }
        }
        let probe_index = cx
            .update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .and_then(|page| {
                        page.documents
                            .iter()
                            .position(|document| document.id == switch_document_id)
                    })
            })
            .expect("探针文档必须在本页");
        // 虚拟列表只构建视口附近的卡片：先滚回首篇，再从**同时可见**的卡片里另选一张，
        // 这样两张卡片的编辑按钮都点得到（切文档的确认框需要点第二张）。
        let target_index = 0usize;
        scroll_card_into_view(
            cx,
            selector(format!("mongo-document-detail:{target_index}")),
        );
        let visible_edit: Vec<usize> = (0..=probe_index.max(8))
            .filter(|index| {
                redraw_and_bounds(cx, selector(format!("mongo-document-detail:{index}")))
                    .is_some_and(|bounds| window_bounds.contains(&bounds.center()))
            })
            .collect();
        let other_index = visible_edit.iter().copied().find(|index| *index != target_index);
        let other_index = other_index.expect("首篇旁边必须还有一张卡片的编辑按钮落在窗口内");
        let edit_button = redraw_and_bounds(
            cx,
            selector(format!("mongo-document-detail:{target_index}")),
        )
        .expect("编辑按钮必须渲染");
        cx.simulate_click(edit_button.center(), Modifiers::default());
        wait_until(cx, "编辑抽屉打开", |cx| {
            redraw_and_bounds(cx, editor_selector)
        });

        // 11.1 修改草稿后点另一张卡片的编辑按钮：必须弹确认框，选「继续编辑」草稿仍在。
        let editor_input = cx
            .update(|_, cx| {
                view.read(cx)
                    .mongo_document_editor
                    .as_ref()
                    .and_then(|editor| editor.component.input().cloned())
            })
            .expect("编辑框必须存在");
        let base_json = cx
            .update(|_, cx| {
                view.read(cx)
                    .mongo_document_editor
                    .as_ref()
                    .map(|editor| editor.base_json.clone())
            })
            .expect("编辑会话");
        assert_eq!(
            cx.update(|_, cx| editor_input.read(cx).value().to_string()),
            base_json,
            "打开时编辑框内容应与基线一致"
        );
        cx.update(|window, cx| {
            editor_input.update(cx, |input, cx| {
                input.set_value(edited_json.clone(), window, cx);
                input.focus(window, cx);
            });
            window.draw(cx).clear(cx);
        });
        let discard_selector = selector("mongo-document-discard".to_string());
        // 抽屉占去 380px 后卡片会重新测量高度，滚动位置也可能微调；点之前再确认一次可见。
        scroll_card_into_view(
            cx,
            selector(format!("mongo-document-detail:{other_index}")),
        );
        let other_edit =
            redraw_and_bounds(cx, selector(format!("mongo-document-detail:{other_index}")))
                .expect("另一张卡片的编辑按钮必须渲染");
        cx.simulate_click(other_edit.center(), Modifiers::default());
        wait_until(cx, "切换文档先确认", |cx| {
            redraw_and_bounds(cx, discard_selector)
        });
        let stay = redraw_and_bounds(cx, selector("mongo-document-discard-cancel".to_string()))
            .expect("继续编辑按钮必须渲染");
        cx.simulate_click(stay.center(), Modifiers::default());
        wait_until(cx, "关闭确认框", |cx| {
            (redraw_and_bounds(cx, discard_selector).is_none()).then_some(())
        });
        assert_eq!(
            cx.update(|_, cx| editor_input.read(cx).value().to_string()),
            edited_json,
            "选「继续编辑」必须保留草稿"
        );

        // 11.1.1 选「放弃并切换」：草稿丢弃、编辑会话切到目标文档。
        scroll_card_into_view(
            cx,
            selector(format!("mongo-document-detail:{other_index}")),
        );
        let other_edit =
            redraw_and_bounds(cx, selector(format!("mongo-document-detail:{other_index}")))
                .expect("另一张卡片的编辑按钮必须渲染");
        cx.simulate_click(other_edit.center(), Modifiers::default());
        wait_until(cx, "再次确认", |cx| {
            redraw_and_bounds(cx, discard_selector)
        });
        let discard = redraw_and_bounds(cx, selector("mongo-document-discard-confirm".to_string()))
            .expect("放弃并切换按钮必须渲染");
        cx.simulate_click(discard.center(), Modifiers::default());
        wait_until(cx, "切换到目标文档", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .mongo_document_editor
                    .as_ref()
                    .filter(|editor| editor.index == Some(other_index))
                    .and_then(|editor| editor.component.input().cloned())
            })
        });
        // 回到首篇继续验证保存：重新打开它并写入同一份草稿。
        scroll_card_into_view(
            cx,
            selector(format!("mongo-document-detail:{target_index}")),
        );
        let reopen = redraw_and_bounds(
            cx,
            selector(format!("mongo-document-detail:{target_index}")),
        )
        .expect("首篇卡片的编辑按钮必须渲染");
        cx.simulate_click(reopen.center(), Modifiers::default());
        let editor_input = wait_until(cx, "重新打开首篇", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .mongo_document_editor
                    .as_ref()
                    .filter(|editor| editor.index == Some(target_index))
                    .and_then(|editor| editor.component.input().cloned())
            })
        });
        cx.update(|window, cx| {
            editor_input.update(cx, |input, cx| {
                input.set_value(edited_json.clone(), window, cx);
                input.focus(window, cx);
            });
            window.draw(cx).clear(cx);
        });

        // 11.2 保存：真实按钮触发，成功后抽屉关闭并重查本页带回新值。
        let save_selector = selector("mongo-document-editor-save".to_string());
        let save_bounds = wait_until(cx, "保存按钮可用", |cx| {
            redraw_and_bounds(cx, save_selector)
        });
        cx.simulate_click(save_bounds.center(), Modifiers::default());
        wait_until(cx, "保存后抽屉关闭", |cx| {
            (redraw_and_bounds(cx, editor_selector).is_none()).then_some(())
        });
        wait_for_mongo_document(cx, &view, &document_id, &edited_json, "重新查询带回新值");

        // 11.3 还原本次写入：用例只借真实库验证写回，不留下被改的演示数据。
        // 还原走同一条写命令（定位条件相同），随后重新查询确认已回到原值。
        let restore_tab = cx
            .update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| matches!(tab.kind, TabKind::MongoCollection(_)))
                    .map(|tab| tab.id)
            })
            .expect("集合标签页仍在");
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::SaveMongoDocument {
                        tab_id: restore_tab,
                        before_json: edited_json.clone(),
                        after_json: before_json.clone(),
                    },
                    cx,
                );
                this.run_mongo_query(restore_tab, cx);
            });
        });
        wait_for_mongo_document(cx, &view, &document_id, &before_json, "还原原始文档");

        // 12. 清理探针文档：本用例只在真实库验证交互，不留下额外数据。
        // 复制出来的副本 `_id` 由服务端生成，只能按 `name` 定位；新增的那篇按预填的 `_id`。
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                for query in [
                    format!(
                        r#"{{ "delete": "{collection}", "deletes": [ {{ "q": {{ "_id": "{switch_document_id}" }}, "limit": 1 }} ] }}"#
                    ),
                    format!(
                        r#"{{ "delete": "{collection}", "deletes": [ {{ "q": {{ "name": "{duplicate_name}" }}, "limit": 0 }} ] }}"#
                    ),
                    format!(
                        r#"{{ "delete": "{collection}", "deletes": [ {{ "q": {{ "name": "{insert_name}" }}, "limit": 0 }} ] }}"#
                    ),
                ] {
                    this.dispatch(
                        AppCommand::MongoRunCommand {
                            connection_id,
                            database: database.clone(),
                            command: query,
                        },
                        cx,
                    );
                }
                this.run_mongo_query(mongo_tab, cx);
            });
        });
        wait_until(cx, "探针文档清理", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .is_some_and(|page| {
                        page.documents
                            .iter()
                            .all(|document| document.id != switch_document_id)
                            && page
                                .documents
                                .iter()
                                .all(|document| !document.json.contains(duplicate_name))
                            && page
                                .documents
                                .iter()
                                .all(|document| !document.json.contains(insert_name))
                    })
            })
            .then_some(())
        });
    }

    /// MongoDB 命令行：库行右键 → 菜单打开终端 → 键盘输入真实语句。
    ///
    /// 断言分两层：终端 grid 里用户真正看到的文本（banner / prompt / 结果 / 只读拒绝），
    /// 以及标签页状态里确实存在 `MongoCli`。不直接调 REPL 或处理函数。
    #[gpui::test]
    fn mongo_cli_runs_shell_statements_from_keyboard(cx: &mut TestAppContext) {
        let Some(config) = live_connection() else {
            eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
            return;
        };
        let connection_id = config.id;
        let database = live_database(&config);
        let (view, cx) = launch_app(cx, config);
        let database_selector = connect_and_wait_database(cx, connection_id, &database);

        // 1. 库行右键 → 「Command line」（界面文案走语言包，按真实文案定位）。
        let database_row = redraw_and_bounds(cx, database_selector).expect("库行必须渲染");
        cx.simulate_event(MouseDownEvent {
            position: database_row.center(),
            modifiers: Modifiers::default(),
            button: MouseButton::Right,
            click_count: 1,
            first_mouse: false,
        });
        let menu_item = wait_until(cx, "库右键菜单出现", |cx| {
            redraw_and_bounds(
                cx,
                selector("database-menu-item:Command line".to_string()),
            )
        });
        cx.simulate_click(menu_item.center(), Modifiers::default());

        // 2. 标签页必须真的建起来，并且指向当前连接 / 当前库。
        wait_until(cx, "命令行标签页", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCli(cli)
                            if cli.connection_id == connection_id && cli.database == database =>
                        {
                            Some(())
                        }
                        _ => None,
                    })
            })
        });

        // 3. 点终端表面取焦点：键盘输入只发给聚焦的输入 handler。
        let surface = wait_until(cx, "终端表面渲染", |cx| {
            redraw_and_bounds(cx, selector("terminal-surface".to_string()))
        });
        cx.simulate_click(surface.center(), Modifiers::default());
        // 聚焦后要再画一帧，输入 handler 才会挂到窗口上（否则 `simulate_input` 无处可去）。
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        // 4. 开场只有提示符（不打 banner），格式是 `<库名>> `（Compass / mongosh 风格）。
        let prompt = format!("{database}> ");
        wait_until(cx, "提示符", |cx| {
            shell_screen(cx, &view).contains(&prompt).then_some(())
        });

        // 5. Tab 补全：敲 `sh` 按 Tab 必须真的弹出候选浮层（不是静默透传），
        //    回车把首个候选写成整行 `show dbs`，再回车执行。
        cx.simulate_input("sh");
        wait_until(cx, "输入回显", |cx| {
            shell_screen(cx, &view).contains("> sh").then_some(())
        });
        cx.simulate_keystrokes("tab");
        wait_until(cx, "补全浮层", |cx| {
            visible_bounds(cx, selector("terminal-completion-popup".to_string()))
        });
        cx.simulate_keystrokes("enter");
        wait_until(cx, "候选写回输入行", |cx| {
            (shell_screen(cx, &view).contains("> show dbs")
                && redraw_and_bounds(cx, selector("terminal-completion-popup".to_string()))
                    .is_none())
            .then_some(())
        });

        // 6. 回车执行补全出来的整行：结果来自真实服务端，`admin` 必然存在。
        cx.simulate_keystrokes("enter");
        wait_until(cx, "show dbs 结果", |cx| {
            shell_screen(cx, &view).contains("admin").then_some(())
        });

        // 6.5 敲到 `db.` 就要出集合名候选：这条走的是**服务端集合列表**，与方法白名单是
        //     两条完全不同的数据来源。只测方法名会让「集合名一直读不到」照样全绿。
        //     必须在这里做：第 7 步 `use admin` 之后当前库换成 admin，它本来就没有集合。
        type_in_terminal_echo(cx, &view, "db.");
        // 浮层尺寸必须跟着候选数走：只有一个集合（`events`）时就该是一行高的小卡片，
        // 不能像早先那样固定 `min_w(320) + max_h(260)` 撑成一块空黑板。
        wait_until(cx, "db. 出集合名候选", |cx| {
            (visible_bounds(cx, selector("terminal-completion-popup".to_string())).is_some()
                && shell_screen(cx, &view).contains("db."))
                .then_some(())
        });
        let bounds = visible_bounds(cx, selector("terminal-completion-popup".to_string()))
            .expect("浮层必须可见");
        // 上下都要卡：撑满 260 是一块空黑板，压到一行高以下则文字被裁掉——
        // 这两种都真实发生过，截图里表现为「一个大黑框」和「一个空白小框」。
        let height = f32::from(bounds.size.height);
        assert!(
            (34.0..60.0).contains(&height),
            "单条候选的高度应为一行 + 内边距（约 38px），实际 {height}px：{bounds:?}"
        );
        // 宽度同理：只有一个三字母候选时不该被拉成一条宽卡片（截图里是 365px 的白板）。
        let width = f32::from(bounds.size.width);
        assert!(
            width < 200.0,
            "单条 'use' 候选应贴合内容（约 60-120px），实际 {width}px：{bounds:?}"
        );
        // 盒子高度对了不代表字看得见：行内容比外壳高时，GPUI 只裁掉溢出部分，
        // 截图上就是「use」只剩上半截。必须断言候选行整行落在浮层内。
        let row = visible_bounds(cx, selector("terminal-completion-row:0".to_string()))
            .expect("候选行必须可见");
        assert!(
            row.top() >= bounds.top() && row.bottom() <= bounds.bottom(),
            "候选行必须整行落在浮层内（截图中被裁成半截字）：行 {row:?} 浮层 {bounds:?}"
        );
        // 文字本身也要落在行框里。浮层没有显式 `text_size` 时会继承根字号（16px），
        // 比终端 12px 大，行框只有 30px 时装不下，上下各切一刀——截图里就是「use 只剩半截」。
        // 这条在 `completion_popup` 丢掉 `text_size` 时失败。
        let label = visible_bounds(cx, selector("terminal-completion-label:0".to_string()))
            .expect("候选文字必须可见");
        assert!(
            label.top() >= row.top() && label.bottom() <= row.bottom(),
            "候选文字必须整块落在行框内（字号继承根字号会被裁掉）：字 {label:?} 行 {row:?}"
        );
        // 横向也必须跟着光标：锚点早先钉在面板左边缘，光标在行尾时浮层离光标半个屏幕。
        // 提示符 `fluxdb_demo> ` 就占 13 格，浮层左缘必然明显在面板左缘之内。
        let surface = visible_bounds(cx, selector("terminal-surface".to_string()))
            .expect("终端表面必须可见");
        assert!(
            bounds.left() > surface.left() + px(40.),
            "浮层应贴着光标列而不是面板左缘：浮层 {bounds:?} 面板 {surface:?}"
        );
        // 清掉这半行：终端没有绑 `ctrl-u`，用退格逐个删。删空后候选必须自己收起
        // （空行不提示），这本身也是「浮层跟着输入行走」的断言。
        for _ in 0..3 {
            cx.simulate_keystrokes("backspace");
        }
        wait_until(cx, "清空半行后浮层收起", |cx| {
            redraw_and_bounds(cx, selector("terminal-completion-popup".to_string()))
                .is_none()
                .then_some(())
        });

        // 7. 光标可左右移动：敲 `use admn`，← 一次把光标移到 n 前，补 i 成 `use admin`。
        //    这条同时验证「中途插入」——只靠行尾追加是拼不出正确语句的。
        cx.simulate_input("use admn");
        cx.simulate_keystrokes("left");
        cx.simulate_input("i");
        cx.simulate_keystrokes("enter");
        wait_until(cx, "use 切库", |cx| {
            let screen = shell_screen(cx, &view);
            (screen.contains("switched to db admin") && screen.contains("admin> ")).then_some(())
        });

        // 8. 只读边界：写命令必须被拒，且错误文本真的上屏。
        type_in_terminal(cx, "db.demo.insertOne({ name: \"fluxdb\" })");
        wait_until(cx, "写命令被拒", |cx| {
            shell_screen_compact(cx, &view)
                .contains("read-only")
                .then_some(())
        });

        // 9. 未知方法：单行英文报错，且真的上屏。
        //    `shell_screen_compact` 已去掉空白，匹配文本也不能带空格（长句会跨行折行）。
        type_in_terminal(cx, "db.demo.unknownMethod()");
        wait_until(cx, "unknown method message", |cx| {
            shell_screen_compact(cx, &view)
                .contains("Unknownmethod")
                .then_some(())
        });

        // 10. 边打字边出候选：敲到 `db.demo.co` 不按 Tab，浮层就该自己出来（Compass 行为）。
        //     集合名不必真实存在——候选来自方法白名单，不依赖服务端数据。
        type_in_terminal_echo(cx, &view, "db.demo.co");
        wait_until(cx, "打字即出候选", |cx| {
            visible_bounds(cx, selector("terminal-completion-popup".to_string()))
        });
        cx.simulate_keystrokes("tab");
        wait_until(cx, "方法补全浮层", |cx| {
            visible_bounds(cx, selector("terminal-completion-popup".to_string()))
        });
        // 10.1 浮层打开时 ↑/↓ 在候选间移动高亮，而不是把方向键透传给 PTY 去翻历史行。
        //      `db.demo.co` 的候选按方法白名单过滤后是 `count` / `countDocuments` 两条，
        //      ↓ 之后回车写回的是第二条——写回 `countDocuments(` 才算方向键真的生效。
        cx.simulate_keystrokes("down");
        // 方向键被浮层吃掉：输入行仍是刚敲的 `db.demo.co`。透传给 PTY 时 REPL 会去翻历史行，
        // 整行输入被换成上一条命令——这条断言就是它的探针。
        wait_until(cx, "方向键不翻历史", |cx| {
            shell_screen(cx, &view).contains("db.demo.co").then_some(())
        });
        cx.simulate_keystrokes("enter");
        wait_until(cx, "↓ 后回车写回第二条候选", |cx| {
            shell_screen_compact(cx, &view)
                .contains("db[\"demo\"].countDocuments")
                .then_some(())
        });

        // 10.2 鼠标：划过候选行即改高亮，按下即写回。清干净后重来一次。
        for _ in 0..40 {
            cx.simulate_keystrokes("backspace");
        }
        wait_until(cx, "清行", |cx| {
            shell_screen_compact(cx, &view).ends_with("admin>").then_some(())
        });
        type_in_terminal_echo(cx, &view, "db.demo.co");
        wait_until(cx, "再次出候选", |cx| {
            visible_bounds(cx, selector("terminal-completion-popup".to_string()))
        });
        let first_row = visible_bounds(cx, selector("terminal-completion-row:0".to_string()))
            .expect("候选行必须可见");
        // 行中心落在窗口内：越界点击会被 gpui 静默丢弃，先断言再点。
        assert!(
            first_row.size.width > px(0.) && first_row.size.height > px(0.),
            "候选行必须有尺寸：{first_row:?}"
        );
        cx.simulate_mouse_move(first_row.center(), None, Modifiers::default());
        cx.run_until_parked();
        cx.simulate_click(first_row.center(), Modifiers::default());
        wait_until(cx, "点候选写回输入行", |cx| {
            let screen = shell_screen_compact(cx, &view);
            (redraw_and_bounds(cx, selector("terminal-completion-popup".to_string())).is_none()
                && screen.contains("db[\"demo\"].count(")
                && !screen.contains("countDocuments"))
            .then_some(())
        });
        // 清掉写回的候选，回到干净状态，接着验证 Esc。
        for _ in 0..40 {
            cx.simulate_keystrokes("backspace");
        }
        wait_until(cx, "清行", |cx| {
            shell_screen_compact(cx, &view).ends_with("admin>").then_some(())
        });
        type_in_terminal_echo(cx, &view, "db.demo.co");
        wait_until(cx, "第三次出候选", |cx| {
            visible_bounds(cx, selector("terminal-completion-popup".to_string()))
        });
        cx.simulate_keystrokes("escape");
        wait_until(cx, "Esc 关闭补全", |cx| {
            (redraw_and_bounds(cx, selector("terminal-completion-popup".to_string())).is_none()
                && shell_screen(cx, &view).contains("db.demo.co"))
            .then_some(())
        });
    }

    /// 双击页签条空白区：MongoDB 库必须开出 Shell，而不是 SQL 查询编辑器。
    ///
    /// 空白区双击原本无条件 `open_new_query`，MongoDB 没有 SQL 编辑器，双击等于没反应。
    /// 这里走真实鼠标事件 + 真实页签条空白区命中框，断言的是标签页种类与提示符上屏。
    #[gpui::test]
    fn mongo_double_click_tab_blank_area_opens_shell(cx: &mut TestAppContext) {
        let Some(config) = live_connection() else {
            eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
            return;
        };
        let connection_id = config.id;
        let database = live_database(&config);
        let (view, cx) = launch_app(cx, config);
        let database_selector = connect_and_wait_database(cx, connection_id, &database);

        // 1. 展开库 → 集合分组 → 点一个集合，让「库工作区」页签条出现（第二级页签行）。
        let database_row = redraw_and_bounds(cx, database_selector).expect("库行必须渲染");
        cx.simulate_click(database_row.center(), Modifiers::default());
        let group_selector = selector(format!("object-group-row:collections:{database}"));
        wait_until(cx, "库展开", |cx| redraw_and_bounds(cx, group_selector));
        let group_row = redraw_and_bounds(cx, group_selector).expect("集合分组行必须渲染");
        cx.simulate_click(group_row.center(), Modifiers::default());
        let collection = wait_until(cx, "集合列表加载", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == connection_id)
                    .and_then(|connection| {
                        connection
                            .objects
                            .iter()
                            .find(|object| {
                                object.path.kind == ObjectKind::Collection
                                    && object.path.database.as_deref() == Some(database.as_str())
                            })
                            .map(|object| object.path.name.clone())
                    })
            })
        });
        let collection_row =
            redraw_and_bounds(cx, selector(format!("table-tree-row:{collection}")))
                .expect("集合行必须渲染");
        cx.simulate_click(collection_row.center(), Modifiers::default());
        wait_until(cx, "集合页渲染", |cx| {
            redraw_and_bounds(cx, selector(format!("mongo-collection-page:{database}/{collection}")))
        });

        // 2. 双击页签条空白区（集合页右侧那段空白）。
        let blank = wait_until(cx, "页签空白区", |cx| {
            visible_bounds(cx, selector("table-tab-blank-area".to_string()))
        });
        // 越界点击会被 gpui 静默丢弃：先断言点在窗口内且不在页签本身上。
        let click_at = gpui::point(blank.right() - px(12.), blank.center().y);
        assert!(
            blank.contains(&click_at),
            "双击点必须落在空白区内：点 {click_at:?} 区域 {blank:?}"
        );
        cx.simulate_event(MouseDownEvent {
            position: click_at,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.run_until_parked();

        // 3. 必须开出 MongoDB Shell 页签，且提示符上屏（不是 SQL 查询编辑器）。
        wait_until(cx, "MongoDB Shell 页签", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .any(|tab| matches!(&tab.kind, TabKind::MongoCli(cli) if cli.database == database))
                    .then_some(())
            })
        });
        assert!(
            !cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .any(|tab| matches!(tab.kind, TabKind::QueryEditor(_)))
            }),
            "MongoDB 双击空白区不应开出 SQL 查询编辑器"
        );
        wait_until(cx, "Shell 提示符", |cx| {
            shell_screen(cx, &view)
                .contains(&format!("{database}> "))
                .then_some(())
        });
    }

    /// 本页首篇文档的 `(可读 id, 完整 JSON)`。
    fn first_document(
        cx: &mut VisualTestContext,
        view: &gpui::Entity<NavicatMain>,
    ) -> (String, String) {
        cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find_map(|tab| match &tab.kind {
                    TabKind::MongoCollection(state) => state.page.as_ref(),
                    _ => None,
                })
                .and_then(|page| page.documents.first())
                .map(|document| (document.id.clone(), document.json.clone()))
        })
        .expect("首屏至少一篇文档")
    }

    /// 改写首篇文档里第一个**顶层标量**字段（`_id` 除外，它不可改），返回改后的整篇文本。
    ///
    /// 优先改字符串（只追加标记，不改变 BSON 类型），没有字符串才退回「数值 +1」。
    fn edit_first_scalar(json: &str) -> Option<String> {
        let mut value: serde_json::Value = serde_json::from_str(json).ok()?;
        let object = value.as_object_mut()?;
        let key = object
            .iter()
            .find(|(key, value)| *key != "_id" && value.is_string())
            .or_else(|| {
                object
                    .iter()
                    .find(|(key, value)| *key != "_id" && value.is_number())
            })
            .map(|(key, _)| key.clone())?;
        let next = match object.get(&key)? {
            serde_json::Value::String(text) => {
                serde_json::Value::String(format!("{text} [fluxdb-e2e]"))
            }
            value => serde_json::json!(value.as_f64()? + 1.0),
        };
        object.insert(key, next);
        serde_json::to_string_pretty(&value).ok()
    }

    /// 等某一页里 `document_id` 的 JSON 与 `expected` 语义相等（忽略键序与空白）。
    fn wait_for_mongo_document(
        cx: &mut VisualTestContext,
        view: &gpui::Entity<NavicatMain>,
        document_id: &str,
        expected: &str,
        label: &str,
    ) {
        let document_id = document_id.to_string();
        let want: serde_json::Value = serde_json::from_str(expected).expect("期望 JSON");
        wait_until(cx, label, |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find_map(|tab| match &tab.kind {
                        TabKind::MongoCollection(state) => state.page.as_ref(),
                        _ => None,
                    })
                    .and_then(|page| {
                        page.documents
                            .iter()
                            .find(|document| document.id == document_id)
                    })
                    .and_then(|document| {
                        serde_json::from_str::<serde_json::Value>(&document.json).ok()
                    })
                    .filter(|got| got == &want)
            })
        });
    }
}
