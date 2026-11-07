    /// 重绘并清空，和现有生产视图测试保持一致。
    fn redis_e2e_redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    fn redis_e2e_click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing control: {selector}"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        redis_e2e_redraw(cx);
    }

    /// 表单在滚动容器里：靠下字段初始在可视区外，先滚动再点击。
    fn redis_e2e_scroll(cx: &mut gpui::VisualTestContext, lines: f32) {
        let center = cx.update(|window, _| window.bounds()).center();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: center,
            delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
            ..Default::default()
        });
        redis_e2e_redraw(cx);
    }

    fn redis_e2e_fill(cx: &mut gpui::VisualTestContext, selector: &'static str, text: &str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing input: {selector}"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        redis_e2e_redraw(cx);
        // 端口等字段带默认值，先全选再输入，避免拼接成 “637926379”。
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input(text);
        redis_e2e_redraw(cx);
    }

    /// 打开「新建连接 → Redis → 高级」。
    fn redis_e2e_open_advanced(cx: &mut gpui::VisualTestContext) {
        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-kind-redis");
        redis_e2e_click(cx, "new-connection-tab-2");
    }

    /// 建好隔离 profile 的生产视图；临时目录必须随返回值存活到测试结束。
    fn redis_e2e_view(
        cx: &mut TestAppContext,
    ) -> (
        &mut gpui::VisualTestContext,
        gpui::Entity<NavicatMain>,
        tempfile::TempDir,
    ) {
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let bootstrap = init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        assert!(bootstrap.issues.is_empty(), "{:?}", bootstrap.issues);

        let controller = AppController::new();
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    storage,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        // 高级页签字段靠下：窗口给足高度，避免点击落在可视区外被静默丢弃。
        vcx.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(1600.)));
        let view = vcx.update(|window, cx| {
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
        redis_e2e_redraw(vcx);
        (vcx, view, temp)
    }

    /// 真实生产视图里的 Redis 连接端到端。
    ///
    /// 覆盖四点：
    /// 1. 不再有拓扑单选，主机/端口始终只有一个入口；
    /// 2. 高级页只剩覆盖项（主库名、强制单机、只读）与 AI 访问范围；
    /// 3. 默认保存为自动探测（档案不写死哨兵/集群节点）；
    /// 4. 强制单机开关会落进档案。
    #[gpui::test]
    fn redis_connection_uses_single_endpoint_and_auto_detection(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, view, temp) = redis_e2e_view(cx);
        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-kind-redis");
        // 连接页：主机/端口始终存在。
        assert!(cx.debug_bounds("connection-host").is_some());
        assert!(cx.debug_bounds("connection-port").is_some());

        redis_e2e_click(cx, "new-connection-tab-2");
        assert!(
            cx.debug_bounds("new-connection-redis-topology-sentinel")
                .is_none(),
            "拓扑选择已移除，应改为连接时自动识别"
        );
        assert!(
            cx.debug_bounds("connection-sentinel-endpoints").is_none(),
            "哨兵节点列表不再单独维护"
        );
        assert!(
            cx.debug_bounds("connection-cluster-start-nodes").is_none(),
            "集群种子列表不再单独维护"
        );
        // 高级页保留探测不到时的覆盖项。
        redis_e2e_scroll(cx, -8.);
        assert!(cx.debug_bounds("connection-sentinel-master-name").is_some());
        assert!(cx.debug_bounds("connection-ai-key-prefixes").is_some());
        redis_e2e_fill(cx, "connection-ai-key-prefixes", "app:");
        redis_e2e_click(cx, "new-connection-save-only");

        cx.update(|_, cx| {
            let state = view.read(cx).controller.state();
            assert_eq!(state.connections.len(), 1, "仅保存应落库一条连接");
            let config = &state.connections[0].config;
            let profile = config
                .redis_profile
                .as_ref()
                .expect("Redis 连接必须带结构化档案");
            assert_eq!(
                profile.topology.kind,
                fluxdb_core::RedisTopologyKind::Standalone,
                "未指定主库名时应交给连接器自动探测"
            );
            assert!(profile.topology.sentinel.endpoints.is_empty());
            assert!(profile.topology.cluster.start_nodes.is_empty());
            assert!(!profile.topology.force_standalone);
            assert_eq!(
                config.options.get("ai_key_prefixes").map(String::as_str),
                Some("app:"),
                "AI Key 前缀应单独成键，不再经过自由参数框"
            );
            assert!(
                !config.options.contains_key("url_params"),
                "Redis 不再写入自由参数串"
            );
        });

        // 第二条：打开「强制按单机连接」，确认落档。
        redis_e2e_open_advanced(cx);
        redis_e2e_scroll(cx, -8.);
        redis_e2e_click(cx, "new-connection-toggle-force-standalone");
        redis_e2e_click(cx, "new-connection-save-only");

        cx.update(|_, cx| {
            let state = view.read(cx).controller.state();
            assert_eq!(state.connections.len(), 2, "第二条连接应落库");
            let profile = state.connections[1]
                .config
                .redis_profile
                .as_ref()
                .expect("Redis 连接必须带结构化档案");
            assert!(
                profile.topology.force_standalone,
                "强制单机开关应写入档案"
            );
            assert_eq!(
                view.read(cx).new_connection_kind,
                None,
                "保存成功后弹框应关闭"
            );
        });
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
    }

    /// 真实哨兵链路端到端：只填哨兵地址、不选拓扑、不填主库名，
    /// 点「测试连接」由连接器自动识别并连到真实主库。
    ///
    /// 需要测试进程可直连的真实哨兵（通常是本地 SSH 端口转发）：
    ///
    /// ```text
    /// FLUXDB_REDIS_SENTINEL_ENDPOINTS=127.0.0.1:26379,127.0.0.1:26380 \
    /// cargo test --locked -p fluxdb-desktop redis_sentinel_connection_test_reaches_real_master
    /// ```
    ///
    /// 未设置环境变量时直接跳过并打印原因，不伪称通过。
    #[gpui::test]
    fn redis_sentinel_connection_test_reaches_real_master(cx: &mut TestAppContext) {
        let Ok(endpoints) = std::env::var("FLUXDB_REDIS_SENTINEL_ENDPOINTS") else {
            eprintln!("skip: FLUXDB_REDIS_SENTINEL_ENDPOINTS 未设置，跳过真实哨兵链路");
            return;
        };
        // 单地址模型：取第一个哨兵节点即可，其余节点由服务端发现。
        let (host, port) = endpoints
            .split(',')
            .next()
            .and_then(|item| item.trim().rsplit_once(':'))
            .unwrap_or(("127.0.0.1", "26379"));

        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);
        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-kind-redis");
        redis_e2e_fill(cx, "connection-host", host);
        redis_e2e_fill(cx, "connection-port", port);
        redis_e2e_click(cx, "new-connection-test");

        // 真实网络 I/O 在后台线程：轮询回执，不用固定 sleep 赌速度。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            cx.run_until_parked();
            redis_e2e_redraw(cx);
            let outcome = cx.update(|_, cx| {
                match view.read(cx).new_connection_form.test_status.as_ref() {
                    Some(crate::ConnectionTestStatus::Success(_)) => Some(Ok(())),
                    Some(crate::ConnectionTestStatus::Error(text)) => Some(Err(text.clone())),
                    Some(crate::ConnectionTestStatus::Pending(_)) | None => None,
                }
            });
            match outcome {
                Some(Ok(())) => break,
                Some(Err(text)) => panic!("哨兵测试连接失败：{text}"),
                None => {}
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待真实哨兵测试连接回执超时"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        cx.update(|_, cx| {
            let this = view.read(cx);
            assert!(
                this.controller.state().connections.is_empty(),
                "仅测试连接不应落库"
            );
            assert!(
                this.new_connection_kind.is_some(),
                "测试成功后弹框应保持打开"
            );
        });
    }
    /// 旧档案编辑：界面只剩单个主机/端口，必须回填真正会拨号的地址，
    /// 并带回 AI 前缀与强制单机开关。
    #[test]
    fn redis_legacy_profile_edits_show_effective_endpoint() {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, RedisConnectionProfile,
            RedisTopologyKind,
        };

        let mut profile = RedisConnectionProfile::default();
        profile.basic.host = "10.0.0.9".to_string();
        profile.basic.port = 6380;
        profile.topology.kind = RedisTopologyKind::Sentinel;
        profile.topology.sentinel.master_name = "mymaster".to_string();
        profile.topology.force_standalone = true;
        // 旧哨兵的目标在节点列表里，与基础主机不同。
        profile.topology.sentinel.endpoints = vec!["1.2.3.4:26379".to_string()];
        let mut options = std::collections::BTreeMap::new();
        options.insert("ai_key_prefixes".to_string(), "app:".to_string());
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "legacy-redis".into(),
            kind: DatabaseKind::Redis,
            endpoint: Endpoint::Tcp {
                host: "10.0.0.9".into(),
                port: 6380,
                database: Some("0".into()),
            },
            options,
            redis_profile: Some(profile),
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };

        let form = crate::NewConnectionForm::from_config(&config);
        assert_eq!(form.host, "1.2.3.4", "应展示实际拨号的哨兵地址");
        assert_eq!(form.port, "26379");
        assert!(form.force_standalone);
        assert_eq!(form.ai_key_prefixes, "app:");
        // 保存时保留显式哨兵模式，不因界面隐藏而丢失主库/节点信息。
        let rebuilt = form.build_redis_profile();
        assert_eq!(rebuilt.topology.kind, RedisTopologyKind::Sentinel);
        assert_eq!(rebuilt.topology.sentinel.endpoints, vec!["1.2.3.4:26379"]);
        assert!(rebuilt.topology.force_standalone);
    }
