// Redis CLI 终端在 NavicatMain 上的接线：按 tab 懒建 TerminalComponent、周期 pumping、
// tab 关闭时终止 PTY。
//
// 本文件只负责「打开 / 复用 / 销毁 + 后台读取」的会话管理；真正的绘制内容见 render.rs，
// 输入/危险命令确认见 model.rs。会话实体按 `tab_id` 缓存，一个 tab 一整个 PTY 会话。
//
// 注意：本文件经 include! 汇入 crate-root，避免顶层 `use std::time::Duration`，
// 以免与其它 include! 文件 / main.rs 的 glob 导入产生 E0252 重名冲突，统一全限定使用。

impl NavicatMain {
    /// 取 / 建指定 Redis CLI tab 的终端会话实体。
    ///
    /// 同一 tab 复用同一 `TerminalComponent`（即同一个 PTY 会话，连上的 redis-cli 不清掉）。
    /// 连接配置直接取自 `controller.state().connections[].config`，与 Workbench/各连接器
    /// 共用同一条解析路径；password/username 等已由 storage 解析成明文存放在 options 中，
    /// 故 redis-cli 可直接以此建参（`-a PASS` 仅出现在本地进程 argv，不打印到日志）。
    pub(crate) fn terminal_component_for(
        &mut self,
        tab_id: TabId,
        cli: &fluxdb_app::RedisCliState,
        cx: &mut Context<NavicatMain>,
    ) -> Entity<TerminalComponent> {
        // 确认 pump 已在跑（幂等）；先于下面的不可变借用来做，避免与 `get` 的借重冲突。
        self.ensure_terminal_pump(cx);
        if let Some(session) = self.terminal_sessions.get(&tab_id) {
            return session.clone();
        }

        // 与其余入口一致的连接配置解析：找不到时退化为一个空后端参数（正常打开流程必能命中）。
        let config = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == cli.connection_id)
            .map(|connection| connection.config.clone())
            .unwrap_or_else(|| fluxdb_core::ConnectionConfig {
                id: cli.connection_id,
                name: String::new(),
                kind: fluxdb_core::DatabaseKind::Redis,
                endpoint: fluxdb_core::Endpoint::Tcp {
                    host: String::new(),
                    port: 0,
                    database: Some(cli.database.to_string()),
                },
                credential_ref: None,
                options: std::collections::BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
            });

        let adapter: Box<dyn TerminalSessionAdapter> =
            Box::new(fluxdb_app::RedisCliAdapter::new(config, cli.database));
        let completion: Box<dyn TerminalCompletionAdapter> =
            Box::new(fluxdb_app::RedisCliCompletion);

        let component = cx.new(|child_cx| {
            TerminalComponent::new(adapter, completion, child_cx)
        });
        component.update(cx, |session, session_cx| {
            session.spawn_process(session_cx);
        });

        self.terminal_sessions.insert(tab_id, component.clone());
        component
    }

    /// 首开终端时启动一个周期 pumping 各 PTY 的后台任务。
    ///
    /// 后台读线程把增量字节丢进每个会话的 mpsc channel；此处每 30ms 消费一次并触发重绘，
    /// 保证 PTY 输出能实时上屏（`pump_transport` 内有变更时才 notify）。会话关闭被移除后，
    /// 对应实体句柄失效，pump 继续只作用于存活会话。
    fn ensure_terminal_pump(&mut self, cx: &mut Context<NavicatMain>) {
        if self._terminal_session_pump.is_some() {
            return;
        }
        let task = cx.spawn(async move |view, cx| {
            let mut timer = smol::Timer::after(std::time::Duration::from_millis(30));
            loop {
                timer.await;
                timer = smol::Timer::after(std::time::Duration::from_millis(30));
                // 主线程仍存活时才继续；升级失败说明 NavicatMain 已释放，结束循环。
                let alive = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return false;
                    };
                    view.update(cx, |this, cx| {
                        let sessions: Vec<Entity<TerminalComponent>> =
                            this.terminal_sessions.values().cloned().collect();
                        for session in sessions {
                            session.update(cx, |comp, comp_cx| comp.pump_transport(comp_cx));
                        }
                    });
                    true
                });
                if !alive {
                    break;
                }
            }
        });
        self._terminal_session_pump = Some(task);
    }
}
