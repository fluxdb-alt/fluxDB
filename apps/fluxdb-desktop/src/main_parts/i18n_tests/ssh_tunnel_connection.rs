    /// 真实生产视图里的 MongoDB SSH 隧道端到端：点击选择类型、切 SSH 页签、填表并保存。
    ///
    /// 由 `scripts/ssh-tunnel-e2e.sh` 提供环境变量与 Podman 容器；未设置时直接跳过。
    /// 目标库只在容器网络内可达，保存成功即证明 UI 表单 → 档案 → 隧道 → 驱动整条链路可用。
    #[gpui::test]
    fn mongo_connection_form_connects_through_ssh_tunnel(cx: &mut TestAppContext) {
        use fluxdb_core::DatabaseKind;

        let Some(settings) = ssh_e2e_settings() else {
            return;
        };
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        let controller = AppController::new();
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let (_, cx) = cx.add_window_view(move |window, cx| {
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
        // SSH 页签字段较多：窗口给足高度，避免点击落在可视区外被静默丢弃。
        cx.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(1600.)));
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
        let redraw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        };
        redraw(cx);

        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing control: {selector}"));
            cx.simulate_click(bounds.center(), Modifiers::default());
            redraw(cx);
        };
        // 表单在滚动容器里：SSH 密码等靠下字段初始在可视区外，点击会被静默丢弃。
        let scroll = |cx: &mut gpui::VisualTestContext, lines: f32| {
            let center = cx.update(|window, _| window.bounds()).center();
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: center,
                delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
                ..Default::default()
            });
            redraw(cx);
        };
        let fill = |cx: &mut gpui::VisualTestContext, selector: &'static str, text: &str| {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing input: {selector}"));
            cx.simulate_click(bounds.center(), Modifiers::default());
            redraw(cx);
            // 部分字段带默认值（端口 22/27017），先全选再输入，避免把默认值拼在前面。
            cx.simulate_keystrokes("cmd-a");
            cx.simulate_input(text);
            redraw(cx);
        };

        click(cx, "home-new-connection");
        click(cx, "new-connection-kind-mongodb");
        // 「连接信息」页签的字段先填；切到 SSH 页签后这些输入框不再渲染。
        fill(cx, "connection-name", "Mongo SSH E2E");
        fill(cx, "connection-host", &settings.mongo_host);
        fill(cx, "connection-port", &settings.mongo_port.to_string());
        fill(cx, "connection-username", &settings.mongo_user);
        // 认证库留空时驱动默认用 admin；不额外填认证库，避免滚动区域外字段点击被丢弃。
        fill(cx, "connection-password", &settings.mongo_password);

        // 页签顺序固定：0 连接信息 / 1 加密 / 2 SSH / 3 高级。
        click(cx, "new-connection-tab-2");
        assert!(
            cx.debug_bounds("connection-ssh-host").is_some(),
            "MongoDB 必须提供 SSH 页签"
        );
        click(cx, "new-connection-toggle-ssh");

        fill(cx, "connection-ssh-host", &settings.ssh_host);
        fill(cx, "connection-ssh-port", &settings.ssh_port.to_string());
        fill(cx, "connection-ssh-username", &settings.ssh_user);
        // 密码行在滚动视口之外：先向下滚，避免点击被容器裁剪后静默丢弃。
        scroll(cx, -8.);
        fill(cx, "connection-ssh-password", &settings.ssh_password);

        // 先确认键盘输入真的落到表单：避免把输入法/焦点问题误判成隧道失败。
        cx.update(|_, cx| {
            let form = &view.read(cx).new_connection_form;
            assert_eq!(form.host, settings.mongo_host, "MongoDB host");
            assert_eq!(form.port, settings.mongo_port.to_string(), "MongoDB port");
            assert_eq!(form.username, settings.mongo_user, "MongoDB user");
            assert_eq!(form.password, settings.mongo_password, "MongoDB password");
            assert_eq!(form.ssh_host, settings.ssh_host, "SSH host");
            assert_eq!(form.ssh_port, settings.ssh_port.to_string(), "SSH port");
            assert_eq!(form.ssh_username, settings.ssh_user, "SSH user");
            assert_eq!(form.ssh_password, settings.ssh_password, "SSH password");
            assert!(form.ssh_enabled, "SSH toggle");
        });

        click(cx, "new-connection-save");

        // 真实网络 I/O 在后台线程完成：轮询到保存成功或失败，而不是固定 sleep。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            cx.run_until_parked();
            redraw(cx);
            let (saved, failure) = cx.update(|_, cx| {
                let this = view.read(cx);
                (
                    this.controller.state().connections.len(),
                    match this.new_connection_form.test_status.as_ref() {
                        Some(crate::ConnectionTestStatus::Error(text)) => Some(text.clone()),
                        _ => None,
                    },
                )
            });
            if saved == 1 {
                break;
            }
            if let Some(text) = failure {
                panic!("MongoDB SSH 隧道保存失败: {text}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待 MongoDB SSH 隧道连接超时"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        cx.update(|_, cx| {
            let this = view.read(cx);
            let connection = &this.controller.state().connections[0];
            assert_eq!(connection.config.kind, DatabaseKind::MongoDb);
            let profile = connection
                .config
                .mongo_profile
                .as_ref()
                .expect("MongoDB profile");
            assert!(profile.ssh.enabled, "SSH 档案必须随连接保存");
            assert_eq!(profile.ssh.host, settings.ssh_host);
            assert!(
                this.new_connection_kind.is_none(),
                "连接成功后弹框应关闭"
            );
        });
    }

    /// SSH E2E 容器参数；缺失时返回 `None`，用例跳过。
    struct SshE2eSettings {
        ssh_host: String,
        ssh_port: u16,
        ssh_user: String,
        ssh_password: String,
        mongo_host: String,
        mongo_port: u16,
        mongo_user: String,
        mongo_password: String,
    }

    fn ssh_e2e_settings() -> Option<SshE2eSettings> {
        fn host_port(value: &str) -> Option<(String, u16)> {
            let (host, port) = value.trim().rsplit_once(':')?;
            Some((host.to_string(), port.parse().ok()?))
        }
        let (ssh_host, ssh_port) = host_port(&std::env::var("FLUXDB_SSH_E2E_SSH").ok()?)?;
        let (mongo_host, mongo_port) =
            host_port(&std::env::var("FLUXDB_SSH_E2E_MONGO").ok()?)?;
        let ssh_password = std::env::var("FLUXDB_SSH_E2E_SSH_PASSWORD").ok()?;
        Some(SshE2eSettings {
            ssh_host,
            ssh_port,
            ssh_user: std::env::var("FLUXDB_SSH_E2E_SSH_USER").ok()?,
            ssh_password,
            mongo_host,
            mongo_port,
            mongo_user: std::env::var("FLUXDB_SSH_E2E_MONGO_USER").ok()?,
            mongo_password: std::env::var("FLUXDB_SSH_E2E_MONGO_PASSWORD").ok()?,
        })
    }
