    /// MongoDB 备份/恢复真实端到端：生产视图点击 → AppCommand → mongodump/mongorestore。
    ///
    /// 环境变量与连接器层 live 测试一致（见 `crates/fluxdb-connectors/tests/mongo_backup_live.rs`）：
    /// - `FLUXDB_MONGO_SMOKE=host:port:user:password`
    /// - `FLUXDB_MONGO_TOOLS_DIR=<mongodump/mongorestore 所在目录>`，调用方需把该目录放进 PATH
    ///
    /// 只创建并清理自己命名的两个库；归档与 profile 都放在临时目录。
    #[gpui::test]
    fn mongo_backup_and_restore_through_production_views(cx: &mut TestAppContext) {
        use fluxdb_app::{AppCommand, MongoClientTool};

        let Some(env) = mongo_backup_e2e_settings() else {
            return;
        };
        assert!(
            fluxdb_app::resolve_mongo_client_tool(MongoClientTool::Dump).is_some()
                && fluxdb_app::resolve_mongo_client_tool(MongoClientTool::Restore).is_some(),
            "已设置 FLUXDB_MONGO_SMOKE，但 PATH 中找不到 mongodump/mongorestore"
        );

        // 备份/恢复任务里的 smol::Timer 由 async-io 线程唤醒；测试调度器默认把
        // 跨线程唤醒视为非确定性错误。本用例明确验证真实子进程链路，允许其停放。
        cx.dispatcher.allow_parking();

        let source = format!("fluxdb_mongo_ui_src_{}", std::process::id());
        let target = format!("fluxdb_mongo_ui_dst_{}", std::process::id());
        let _cleanup = MongoBackupE2eCleanup {
            shell: env.shell.clone(),
            uri: env.uri.clone(),
            databases: vec![source.clone(), target.clone()],
        };
        for database in [&source, &target] {
            mongo_backup_e2e_shell(&env, &format!(
                "db.getSiblingDB({database:?}).dropDatabase()"
            ));
        }
        mongo_backup_e2e_shell(
            &env,
            &format!(
                "db.getSiblingDB({source:?}).c.insertMany([{{_id:1,v:'x'}},{{_id:2,v:'y'}}])"
            ),
        );
        assert_eq!(mongo_backup_e2e_count(&env, &source), 2);
        let config = env.config.clone();

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let bootstrap = init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        assert!(bootstrap.issues.is_empty(), "{:?}", bootstrap.issues);
        let backup_dir = temp.path().join("backups");
        fs::create_dir_all(&backup_dir).expect("create backup dir");

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::SaveSettings(Settings {
            backup_dir: backup_dir.to_string_lossy().into_owned(),
            ..Settings::default()
        }));
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
        redis_e2e_redraw(cx);

        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenBackupList(fluxdb_core::ObjectPath {
                    connection_id: this.controller.state().connections[0].config.id,
                    database: Some(source.clone()),
                    schema: None,
                    name: source.clone(),
                    kind: fluxdb_core::ObjectKind::Database,
                }),
                cx,
            );
        });
        redis_e2e_redraw(cx);
        assert!(
            cx.debug_bounds("backup-new").is_some(),
            "备份列表页应提供新建备份入口"
        );

        // 新建备份：真实点击打开弹框，填写输出目录与固定文件名。
        redis_e2e_click(cx, "backup-new");
        redis_e2e_fill(cx, "backup-target-dir", &backup_dir.to_string_lossy());
        redis_e2e_fill(cx, "backup-file-name", "e2e-mongo");
        redis_e2e_click(cx, "backup-start");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            cx.run_until_parked();
            redis_e2e_redraw(cx);
            let finished = cx.update(|_, cx| {
                let this = view.read(cx);
                this.backup_tasks
                    .iter()
                    .find(|task| task.database.as_deref() == Some(source.as_str()))
                    .map(|task| (!task.running(), task.error.clone()))
            });
            match finished {
                Some((false, _)) => {}
                Some((true, Some(error))) => panic!("MongoDB 备份失败: {error}"),
                Some((true, None)) => break,
                None => panic!("备份任务未创建"),
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待 MongoDB 备份完成超时"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let record = cx.update(|_, cx| {
            view.read(cx)
                .storage
                .load_backup_records()
                .expect("load backup records")
                .into_iter()
                .find(|record| record.database == source)
                .expect("备份记录应落库")
        });
        let archive = std::path::PathBuf::from(&record.output_path);
        assert!(archive.is_file(), "归档文件应存在: {}", archive.display());
        assert_eq!(
            record.manifest.as_ref().and_then(|m| m.database.as_deref()),
            Some(source.as_str()),
            "备份记录必须保留原始库名供恢复映射"
        );

        // 关闭日志弹框，回到列表行，用行内「恢复」进入恢复弹框（携带 manifest）。
        redis_e2e_click(cx, "backup-close");
        assert!(
            cx.debug_bounds("backup-row-path").is_some(),
            "备份列表路径列应可点击复制"
        );
        redis_e2e_click(cx, "backup-row-path");
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some(record.output_path.as_str()));
        assert!(
            cx.debug_bounds("backup-row-restore").is_some(),
            "备份完成后列表应出现恢复入口"
        );
        redis_e2e_click(cx, "backup-row-restore");
        assert!(
            cx.update(|_, cx| view.read(cx).restore_modal.is_some()),
            "MongoDB 归档应允许打开恢复弹框"
        );
        redis_e2e_fill(cx, "restore-target", &target);
        redis_e2e_click(cx, "restore-check");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            cx.run_until_parked();
            redis_e2e_redraw(cx);
            let state = cx.update(|_, cx| {
                let this = view.read(cx);
                this.restore_modal.as_ref().map(|modal| {
                    let runtime = modal.runtime.borrow();
                    (runtime.running, runtime.prepared.is_some(), runtime.logs.clone())
                })
            });
            let Some((running, prepared, logs)) = state else {
                panic!("恢复弹框被意外关闭");
            };
            if prepared {
                break;
            }
            if !running {
                panic!("恢复预检查失败: {}", logs.join(" | "));
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待恢复预检查超时"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let run_bounds = cx
            .debug_bounds("restore-run")
            .expect("恢复确认按钮应可点击");
        let window_bounds = cx.update(|window, _| window.bounds());
        assert!(
            window_bounds.contains(&run_bounds.center()),
            "恢复确认按钮落在窗口外: {run_bounds:?} / {window_bounds:?}"
        );
        redis_e2e_click(cx, "restore-run");
        // mongorestore 可能在点击处理过程中就已完成：以恢复记录落库为完成信号，
        // 不用运行态轮询（会与同步完成抢跑）。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            cx.run_until_parked();
            redis_e2e_redraw(cx);
            let finished = cx.update(|_, cx| {
                view.read(cx)
                    .storage
                    .load_restore_records()
                    .expect("load restore records")
                    .into_iter()
                    .find(|record| record.target == target)
            });
            if let Some(record) = finished {
                assert!(
                    record.success,
                    "MongoDB 恢复记录标记失败: {}",
                    record.result
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待 MongoDB 恢复完成超时"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(
            mongo_backup_e2e_count(&env, &target),
            2,
            "目标库文档数应与源库一致"
        );
        assert_eq!(
            mongo_backup_e2e_count(&env, &source),
            2,
            "恢复不应修改源库"
        );
    }

    /// E2E 环境：应用配置、工具目录与用于装数/计数的 mongosh。
    ///
    /// 不在测试进程里用 Mongo 驱动：它的异步 IO 线程会唤醒 gpui 测试调度器，
    /// 触发 “activity on thread async-io” 的非确定性断言。
    struct MongoBackupE2eEnv {
        config: fluxdb_core::ConnectionConfig,
        _tools_dir: std::path::PathBuf,
        shell: std::path::PathBuf,
        uri: String,
    }

    struct MongoBackupE2eCleanup {
        shell: std::path::PathBuf,
        uri: String,
        databases: Vec<String>,
    }

    impl Drop for MongoBackupE2eCleanup {
        fn drop(&mut self) {
            for database in &self.databases {
                let _ = std::process::Command::new(&self.shell)
                    .args([
                        "--quiet",
                        "--norc",
                        &self.uri,
                        "--eval",
                        &format!("db.getSiblingDB({database:?}).dropDatabase()"),
                    ])
                    .output();
            }
        }
    }

    /// 与连接器层 live 测试共用环境变量；额外需要 `mongosh`（可用 `FLUXDB_MONGOSH` 指定）。
    fn mongo_backup_e2e_settings() -> Option<MongoBackupE2eEnv> {
        use fluxdb_core::{
            Endpoint, MongoBasicOptions, MongoConnectionProfile, MongoHost, SecretRef,
        };

        let raw = std::env::var("FLUXDB_MONGO_SMOKE").ok()?;
        let tools_dir = std::env::var("FLUXDB_MONGO_TOOLS_DIR").ok()?;
        let parts = raw.splitn(4, ':').collect::<Vec<_>>();
        if parts.len() != 4 {
            return None;
        }
        let port = parts[1].parse().ok()?;
        let shell = std::path::PathBuf::from(
            std::env::var("FLUXDB_MONGOSH").unwrap_or_else(|_| "mongosh".into()),
        );
        let version = std::process::Command::new(&shell)
            .arg("--version")
            .output()
            .ok()?;
        if !version.status.success() {
            return None;
        }
        Some(MongoBackupE2eEnv {
            config: fluxdb_core::ConnectionConfig {
                id: fluxdb_core::ConnectionId(9_942),
                name: "MongoDB backup UI E2E".into(),
                kind: fluxdb_core::DatabaseKind::MongoDb,
                endpoint: Endpoint::Tcp {
                    host: parts[0].into(),
                    port,
                    database: None,
                },
                options: Default::default(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                mongo_profile: Some(MongoConnectionProfile {
                    basic: MongoBasicOptions {
                        hosts: vec![MongoHost {
                            host: parts[0].into(),
                            port,
                        }],
                        username: parts[2].into(),
                        password: SecretRef::inline(parts[3]),
                        auth_database: "admin".into(),
                        ..MongoBasicOptions::default()
                    },
                    ..MongoConnectionProfile::default()
                }),
                sqlserver_profile: None,
                oracle_profile: None,
            },
            _tools_dir: std::path::PathBuf::from(tools_dir),
            shell,
            uri: format!(
                "mongodb://{}:{}@{}:{}/?authSource=admin",
                parts[2], parts[3], parts[0], port
            ),
        })
    }

    /// 用 mongosh 执行脚本；失败直接带 stderr 报错，避免 E2E 悄无声息。
    fn mongo_backup_e2e_shell(env: &MongoBackupE2eEnv, script: &str) -> String {
        let output = std::process::Command::new(&env.shell)
            .args(["--quiet", "--norc", &env.uri, "--eval", script])
            .output()
            .expect("启动 mongosh 失败");
        assert!(
            output.status.success(),
            "mongosh 执行失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn mongo_backup_e2e_count(env: &MongoBackupE2eEnv, database: &str) -> i64 {
        let text = mongo_backup_e2e_shell(
            env,
            &format!("db.getSiblingDB({database:?}).c.countDocuments({{}})"),
        );
        text.parse()
            .unwrap_or_else(|_| panic!("count 输出不是数字：{text}"))
    }
