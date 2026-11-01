// 终端右键菜单 / 键位的端到端用例（真实生产视图 + 真实 redis-cli PTY，无头驱动）。
//
// 复制与清屏是两条独立路径：键盘 `cmd-c` / `cmd-l` 与右键菜单项。共用同一个 helper 不代表
// 接线正确——菜单点击走的是 deferred 菜单实体与它自己的焦点/键盘导航，这条路径必须单独测。
//
// 连接目标取 `FLUXDB_REDIS_HOST` / `FLUXDB_REDIS_PORT`（默认 127.0.0.1:6379）；
// 探测不可达时整组跳过，不把“本机没起 redis”伪装成通过。
mod terminal_menu_tests {
    use fluxdb_app::{AppCommand, AppController, TabKind};
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, Settings};
    use fluxdb_storage::{FileStorage, Storage as _};
    use gpui::{
        AppContext as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
        TestAppContext, VisualTestContext,
    };
    use gpui_component::Root;

    use crate::{
        NavicatMain, app_assets_base_path, extend_component_i18n, init_i18n, navicat_main_new,
    };

    fn selector(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    /// 探测本机 redis 是否可达；不可达时用例跳过（不伪造通过）。
    fn redis_available() -> Option<(String, u16)> {
        let host = std::env::var("FLUXDB_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port: u16 = std::env::var("FLUXDB_REDIS_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(6379);
        let address = format!("{host}:{port}");
        std::net::TcpStream::connect_timeout(
            &address.parse().ok()?,
            std::time::Duration::from_millis(300),
        )
        .ok()?;
        Some((host, port))
    }

    fn redis_config(host: &str, port: u16) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(9_901),
            name: "redis-live".into(),
            kind: DatabaseKind::Redis,
            endpoint: Endpoint::Tcp {
                host: host.to_string(),
                port,
                database: Some("0".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    fn launch(
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

    fn redraw(
        cx: &mut VisualTestContext,
        selector: &'static str,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.debug_bounds(selector)
    }

    fn wait_until<R>(
        cx: &mut VisualTestContext,
        label: &str,
        mut probe: impl FnMut(&mut VisualTestContext) -> Option<R>,
    ) -> R {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            if let Some(value) = probe(cx) {
                return value;
            }
            assert!(std::time::Instant::now() < deadline, "等待超时：{label}");
        }
    }

    fn screen(cx: &mut VisualTestContext, view: &gpui::Entity<NavicatMain>) -> String {
        cx.update(|_, cx| {
            view.read(cx)
                .terminal_sessions
                .values()
                .next()
                .map(|session| session.read(cx).grid.visible_lines().join("\n"))
                .unwrap_or_default()
        })
    }

    /// 打开 Redis CLI 标签页，等到终端表面渲染、提示符上屏并取得焦点。
    fn open_redis_cli(
        cx: &mut VisualTestContext,
        view: &gpui::Entity<NavicatMain>,
    ) -> gpui::Bounds<gpui::Pixels> {
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::OpenRedisCli {
                        connection_id: ConnectionId(9_901),
                        database: 0,
                    },
                    cx,
                );
            })
        });
        wait_until(cx, "redis-cli 标签页", |cx| {
            cx.update(|_, cx| {
                view.read(cx)
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| matches!(tab.kind, TabKind::RedisCli(_)))
                    .map(|tab| tab.id)
            })
        });
        let surface = wait_until(cx, "终端表面", |cx| {
            redraw(cx, selector("terminal-surface".to_string()))
        });
        wait_until(cx, "redis 提示符", |cx| {
            screen(cx, view).contains("6379> ").then_some(())
        });
        cx.simulate_click(surface.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        surface
    }

    /// 真实拖选：从提示符行拖过若干列，必须产生选区。
    fn drag_select(cx: &mut VisualTestContext, surface: gpui::Bounds<gpui::Pixels>) {
        let start = gpui::point(surface.left() + gpui::px(2.), surface.top() + gpui::px(6.));
        let end = gpui::point(surface.left() + gpui::px(160.), surface.top() + gpui::px(6.));
        cx.simulate_event(MouseDownEvent {
            position: start,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(MouseMoveEvent {
            position: end,
            modifiers: Modifiers::default(),
            pressed_button: Some(MouseButton::Left),
        });
        cx.simulate_event(MouseUpEvent {
            position: end,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
        });
        cx.run_until_parked();
    }

    /// cmd-c 必须把选区写进剪贴板。
    #[gpui::test]
    fn cmd_c_copies_selection_to_clipboard(cx: &mut TestAppContext) {
        let Some((host, port)) = redis_available() else {
            eprintln!("跳过：本机 redis 不可达（FLUXDB_REDIS_HOST/FLUXDB_REDIS_PORT）");
            return;
        };
        let (view, cx) = launch(cx, redis_config(&host, port));
        let surface = open_redis_cli(cx, &view);
        drag_select(cx, surface);
        cx.simulate_keystrokes("cmd-c");
        cx.run_until_parked();
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert!(
            copied.as_deref().is_some_and(|text| !text.trim().is_empty()),
            "cmd-c 应把选区写入剪贴板，实际 {copied:?}；屏幕 = {:?}",
            screen(cx, &view)
        );
    }

    /// cmd-l 必须清掉屏幕上的历史内容（真实 PTY 的 Ctrl-L 回显）。
    #[gpui::test]
    fn cmd_l_clears_screen(cx: &mut TestAppContext) {
        let Some((host, port)) = redis_available() else {
            eprintln!("跳过：本机 redis 不可达（FLUXDB_REDIS_HOST/FLUXDB_REDIS_PORT）");
            return;
        };
        let (view, cx) = launch(cx, redis_config(&host, port));
        let _surface = open_redis_cli(cx, &view);
        cx.simulate_input("PING");
        cx.simulate_keystrokes("enter");
        wait_until(cx, "PING 结果上屏", |cx| {
            screen(cx, &view).contains("PONG").then_some(())
        });
        cx.simulate_keystrokes("cmd-l");
        wait_until(cx, "cmd-l 清屏", |cx| {
            let after = screen(cx, &view);
            (!after.contains("PONG")).then_some(())
        });
    }

    /// 右键菜单路径：右键 → 键盘下移到「清屏」→ 回车。
    ///
    /// 菜单项由 gpui-component 渲染、没有 debug selector，所以这里走菜单自己的键盘导航，
    /// 但触发的仍是真实菜单实体与真实点击处理器——不是直接调 `clear_screen`。
    #[gpui::test]
    fn context_menu_clear_screen_clears_real_terminal(cx: &mut TestAppContext) {
        let Some((host, port)) = redis_available() else {
            eprintln!("跳过：本机 redis 不可达（FLUXDB_REDIS_HOST/FLUXDB_REDIS_PORT）");
            return;
        };
        let (view, cx) = launch(cx, redis_config(&host, port));
        let surface = open_redis_cli(cx, &view);
        cx.simulate_input("PING");
        cx.simulate_keystrokes("enter");
        wait_until(cx, "PING 结果上屏", |cx| {
            screen(cx, &view).contains("PONG").then_some(())
        });

        cx.simulate_event(MouseDownEvent {
            position: surface.center(),
            modifiers: Modifiers::default(),
            button: MouseButton::Right,
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        // 菜单是 deferred 建的：多画几帧让它拿到焦点。
        cx.update(|window, cx| {
            for _ in 0..3 {
                window.draw(cx).clear(cx);
            }
        });
        // 菜单打开时 selected_index=None：第 1 次 down 选 0（复制）、第 2 次选 1（粘贴）、
        // 第 3 次跳过分隔符选 3（清屏）。
        cx.simulate_keystrokes("down down down enter");
        wait_until(cx, "右键菜单清屏", |cx| {
            (!screen(cx, &view).contains("PONG")).then_some(())
        });
    }

    /// 右键菜单「复制」：无选区时右键先选中指针下的词，再点复制，剪贴板必须有内容。
    #[gpui::test]
    fn context_menu_copy_copies_word_under_cursor(cx: &mut TestAppContext) {
        let Some((host, port)) = redis_available() else {
            eprintln!("跳过：本机 redis 不可达（FLUXDB_REDIS_HOST/FLUXDB_REDIS_PORT）");
            return;
        };
        let (view, cx) = launch(cx, redis_config(&host, port));
        let surface = open_redis_cli(cx, &view);
        cx.update(|_, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(String::new()))
        });
        cx.run_until_parked();

        // 右键点在提示符行左侧的 `6379` 上（提示符形如 `127.0.0.1:6379> `）。
        let point = gpui::point(surface.left() + gpui::px(70.), surface.top() + gpui::px(8.));
        cx.simulate_event(MouseDownEvent {
            position: point,
            modifiers: Modifiers::default(),
            button: MouseButton::Right,
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            for _ in 0..3 {
                window.draw(cx).clear(cx);
            }
        });
        // 第 1 次 down 选中 0 = 复制。
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert!(
            copied.as_deref().is_some_and(|text| !text.trim().is_empty()),
            "右键复制应把指针下的词写入剪贴板，实际 {copied:?}；屏幕 = {:?}",
            screen(cx, &view)
        );
    }

    /// MongoDB 命令行的 Ctrl+C 必须放弃当前输入并**另起一行**（mongosh / node REPL 的语义）。
    ///
    /// 这条曾经是死的：adapter 的 `ctrl_c_bytes` 返回空切片，组件层收到空切片就不写后端，
    /// REPL 的 `0x03` 分支永远收不到——用户看到的就是「按 Ctrl+C 没反应」。
    #[gpui::test]
    fn mongo_ctrl_c_clears_the_input_line_and_moves_to_a_new_line(cx: &mut TestAppContext) {
        let config = redis_config("127.0.0.1", 6379);
        let (view, cx) = launch(cx, config);
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.dispatch(
                    AppCommand::OpenMongoCli {
                        connection_id: ConnectionId(9_901),
                        database: "demo".into(),
                    },
                    cx,
                );
            })
        });
        let surface = wait_until(cx, "Mongo 终端表面", |cx| {
            redraw(cx, selector("terminal-surface".to_string()))
        });
        wait_until(cx, "Mongo 提示符", |cx| {
            screen(cx, &view).contains("demo> ").then_some(())
        });
        cx.simulate_click(surface.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        cx.simulate_input("db.events.find(");
        wait_until(cx, "输入回显", |cx| {
            screen(cx, &view).contains("db.events.find(").then_some(())
        });
        // 旧行保留输入后仍含 `demo> `，所以必须取**最后**一个提示符行的行号。
        let last_prompt_row = |screen: &str| {
            screen
                .lines()
                .collect::<Vec<_>>()
                .iter()
                .rposition(|line| line.contains("demo> "))
        };
        let prompt_row_before = last_prompt_row(&screen(cx, &view)).expect("提示符所在行");

        cx.simulate_keystrokes("ctrl-c");
        wait_until(cx, "Ctrl+C 另起一行", |cx| {
            last_prompt_row(&screen(cx, &view))
                .filter(|&row| row > prompt_row_before)
                .map(|_| ())
        });
        // 放弃的输入必须留在旧行（node/mongosh REPL 实测行为）：擦成空白等于把用户输入弄丢了。
        let after = screen(cx, &view);
        assert!(
            after.contains("db.events.find("),
            "Ctrl+C 应把放弃的输入留在上一行，而不是擦成空白：{after:?}"
        );
        assert!(after.contains("demo> "), "Ctrl+C 不能结束会话：{after:?}");
    }
}
