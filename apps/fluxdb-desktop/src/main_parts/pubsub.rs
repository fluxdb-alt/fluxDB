// Redis Pub/Sub 在 NavicatMain 上的接线：按 tab 懒建订阅会话、后台轮询消息、合并进 UI。
//
// 本文件只负责「按 tab 建立订阅会话 + 后台读取 + 合并上屏」的会话管理；真正的绘制见
// `redis_pubsub_content`（content_views.rs）。会话实体按 `tab_id` 缓存，一个 tab 一条订阅
// 连接。连接建立 / 消息读取都在后台线程完成（socket 阻塞读不卡主线程），后台结果经 mpsc
// 通道送回主线程 pump 统一合并。
//
// 注意：本文件经 include! 汇入 crate-root，避免顶层 `use`，统一全限定使用（与 terminal
// 接线一致），以免与其它 include! 文件 / main.rs 的 glob 导入产生 E0252 重名冲突。

/// 单条广播消息的可展示视图（payload 按文本呈现）。
#[derive(Clone)]
pub(crate) struct PubSubMessageView {
    /// 命中的 pattern（普通订阅为空）。
    pub pattern: Option<String>,
    /// 实际发布通道名。
    pub channel: String,
    /// payload 文本（UTF-8，二进制按 lossy 呈现）。
    pub payload: String,
    /// 本地接收时间（Unix 秒）。由 UI 在合并上屏时打点，仅用于消息流表格「时间」列
    /// 的展示，不参与业务逻辑 / 连接器协议。
    pub received_at: i64,
}

/// 后台线程发回主线程的消息：连接成败 + 每轮轮询结果。
pub(crate) enum PubSubBackMsg {
    /// 建连失败 / 协议错误（含文案；连接成功隐式由 `Poll` 表达）。
    Failed(String),
    /// 一轮轮询结果。
    Poll(fluxdb_app::PubSubPollOutcome),
}

/// UI 线程发起的订阅 / 发布命令，入队后由后台轮询线程统一写入 socket。
pub(crate) enum PubSubCommand {
    Subscribe(String),
    Unsubscribe(String),
    Publish(String, String),
}

/// 单个 Pub/Sub tab 的会话模型：连接 + 命令队列 + 回传通道 + UI 呈现状态。
pub(crate) struct PubSubSessionModel {
    /// 共享订阅连接（后台线程建连并轮询；UI 线程入队命令）。
    session: std::sync::Arc<std::sync::Mutex<Option<fluxdb_app::RedisPubSubSession>>>,
    /// 待写出的命令队列（订阅 / 取消 / 发布）。
    commands: std::sync::Arc<std::sync::Mutex<Vec<PubSubCommand>>>,
    /// 建连所需配置与目标库（后台线程懒建连时用）。
    config: fluxdb_core::ConnectionConfig,
    database: u32,
    /// 后台结果回传通道（每轮轮询投递一条）。
    back_rx: std::sync::mpsc::Receiver<PubSubBackMsg>,
    back_tx: std::sync::mpsc::Sender<PubSubBackMsg>,
    // ---- UI 呈现状态 ----
    /// 已订阅的通道（用于展示与「取消订阅」入口）。
    pub subscribed: Vec<String>,
    /// 收到的消息流（保留最近 `PUBSUB_MAX_MESSAGES` 条）。
    pub messages: Vec<PubSubMessageView>,
    /// 最近一次发布成功收到的订阅者数量（发布框旁提示，<0 表示尚无）。
    pub last_publish_count: i64,
    /// 连接状态（用于页面顶部「未连接 / 已连接 / 失败」提示，不静默失败）。
    pub connected: bool,
    pub connecting: bool,
    /// 最近一次错误（连接失败 / 协议错误时呈现）。
    pub error: Option<String>,
    // ---- 消息流表格分页呈现状态 ----
    /// 当前页码（0 起始；消息流倒序分页，最新消息在首页）。
    pub message_page: usize,
    /// 每页条数。
    pub message_page_size: usize,
    // 输入框文本缓存（订阅通道 / 发布通道 / 发布内容）。
    pub subscribe_input: String,
    pub publish_channel: String,
    pub publish_message: String,
    // 输入实体与订阅回调（懒建）。
    pub subscribe_input_entity: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub subscribe_input_sub: Option<gpui::Subscription>,
    pub publish_channel_input_entity: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub publish_channel_input_sub: Option<gpui::Subscription>,
    pub publish_message_input_entity: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub publish_message_input_sub: Option<gpui::Subscription>,
}

/// 每个 Pub/Sub tab 最多保留的消息条数（防内存无限增长）。
const PUBSUB_MAX_MESSAGES: usize = 500;
/// 单次轮询的读超时：把「无消息」识别为一次空轮询（既保证响应又不卡后台线程）。
const PUBSUB_POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
/// 主线程触发一次轮询 / 消费后台结果并合并的周期。
const PUBSUB_PUMP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

impl NavicatMain {
    /// 解析某个连接的连接配置（供订阅会话建连）。
    fn pubsub_config_for(
        &self,
        connection_id: fluxdb_core::ConnectionId,
    ) -> Option<fluxdb_core::ConnectionConfig> {
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| connection.config.clone())
    }

    /// 取 / 建指定 Pub/Sub tab 的会话模型，并确保后台 pump 已启动。
    ///
    /// 找不到连接配置时返回 None，由内容页呈现「连接不存在」错误态（不静默失败）。
    pub(crate) fn pubsub_session_for(
        &mut self,
        tab_id: TabId,
        pubsub: &fluxdb_app::RedisPubSubState,
        cx: &mut gpui::Context<NavicatMain>,
    ) -> Option<()> {
        self.ensure_pubsub_pump(cx);
        if self.pubsub_sessions.contains_key(&tab_id) {
            return Some(());
        }
        let config = self.pubsub_config_for(pubsub.connection_id)?;
        let (back_tx, back_rx) = std::sync::mpsc::channel();
        self.pubsub_sessions.insert(
            tab_id,
            PubSubSessionModel {
                session: std::sync::Arc::new(std::sync::Mutex::new(None)),
                commands: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                config,
                database: pubsub.database,
                back_rx,
                back_tx,
                subscribed: Vec::new(),
                messages: Vec::new(),
                last_publish_count: -1,
                connected: false,
                connecting: true,
                error: None,
                message_page: 0,
                message_page_size: 50,
                subscribe_input: String::new(),
                publish_channel: String::new(),
                publish_message: String::new(),
                subscribe_input_entity: None,
                subscribe_input_sub: None,
                publish_channel_input_entity: None,
                publish_channel_input_sub: None,
                publish_message_input_entity: None,
                publish_message_input_sub: None,
            },
        );
        cx.notify();
        Some(())
    }

    /// 删除指定 tab 的 Pub/Sub 会话（tab 关闭时调用，随会话 drop 断开订阅连接）。
    pub(crate) fn close_pubsub_session(&mut self, tab_id: TabId) {
        self.pubsub_sessions.remove(&tab_id);
    }

    /// 订阅一个通道（入队，后台线程写盘；找不到会话则忽略——正常打开流程必有会话）。
    pub(crate) fn pubsub_subscribe(&self, tab_id: TabId, channel: &str) {
        if let Some(model) = self.pubsub_sessions.get(&tab_id)
            && let Ok(mut queue) = model.commands.lock()
        {
            queue.push(PubSubCommand::Subscribe(channel.to_string()));
        }
    }

    /// 取消订阅一个通道（入队）：由「已订阅通道」列表点击触发。
    pub(crate) fn pubsub_unsubscribe(&self, tab_id: TabId, channel: &str) {
        if let Some(model) = self.pubsub_sessions.get(&tab_id)
            && let Ok(mut queue) = model.commands.lock()
        {
            queue.push(PubSubCommand::Unsubscribe(channel.to_string()));
        }
    }

    /// 发布一条消息（入队）。
    pub(crate) fn pubsub_publish(&self, tab_id: TabId, channel: &str, message: &str) {
        if let Some(model) = self.pubsub_sessions.get(&tab_id)
            && let Ok(mut queue) = model.commands.lock()
        {
            queue.push(PubSubCommand::Publish(
                channel.to_string(),
                message.to_string(),
            ));
        }
    }

    /// 点击「订阅」：读取订阅输入框通道并订阅，随后清空输入框与模型缓存。
    pub(crate) fn pubsub_subscribe_clicked(
        &mut self,
        tab_id: TabId,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<NavicatMain>,
    ) {
        let channel = self
            .pubsub_sessions
            .get(&tab_id)
            .map(|m| m.subscribe_input.clone())
            .unwrap_or_default();
        let channel = channel.trim().to_string();
        if channel.is_empty() {
            return;
        }
        self.pubsub_subscribe(tab_id, &channel);
        if let Some(model) = self.pubsub_sessions.get_mut(&tab_id) {
            model.subscribe_input.clear();
            if let Some(entity) = model.subscribe_input_entity.take() {
                entity.update(cx, |state, cx| state.set_value("", window, cx));
                // 保留实体（复用焦点），仅清空文本。
                model.subscribe_input_entity = Some(entity);
            }
        }
        cx.notify();
    }

    /// 点击「发布」：读取发布通道与内容并发布，随后清空内容输入（保留通道便于连续发布）。
    pub(crate) fn pubsub_publish_clicked(
        &mut self,
        tab_id: TabId,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<NavicatMain>,
    ) {
        let (channel, message) = self
            .pubsub_sessions
            .get(&tab_id)
            .map(|m| (m.publish_channel.clone(), m.publish_message.clone()))
            .unwrap_or_default();
        if channel.trim().is_empty() {
            return;
        }
        self.pubsub_publish(tab_id, channel.trim(), &message);
        if let Some(model) = self.pubsub_sessions.get_mut(&tab_id) {
            model.publish_message.clear();
            if let Some(entity) = model.publish_message_input_entity.take() {
                entity.update(cx, |state, cx| state.set_value("", window, cx));
                model.publish_message_input_entity = Some(entity);
            }
        }
        cx.notify();
    }

    /// 清空已收到的消息列表。
    pub(crate) fn pubsub_clear_messages(&mut self, tab_id: TabId) {
        if let Some(model) = self.pubsub_sessions.get_mut(&tab_id) {
            model.messages.clear();
        }
    }

    /// 懒建 Pub/Sub 页面的三个输入框（订阅通道 / 发布通道 / 发布内容）。
    /// 输入内容变化时回写模型对应字段；页面打开期间复用同一实体以保留焦点与状态。
    fn ensure_pubsub_inputs(
        &mut self,
        tab_id: TabId,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<NavicatMain>,
    ) {
        let Some(model) = self.pubsub_sessions.get_mut(&tab_id) else {
            return;
        };
        if model.subscribe_input_entity.is_none() {
            let entity = cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("订阅通道")
            });
            let sub = cx.subscribe(
                &entity,
                move |this: &mut NavicatMain,
                      input,
                      event: &gpui_component::input::InputEvent,
                      cx| {
                    if matches!(event, gpui_component::input::InputEvent::Change)
                        && let Some(model) = this.pubsub_sessions.get_mut(&tab_id)
                    {
                        model.subscribe_input = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            );
            model.subscribe_input_entity = Some(entity);
            model.subscribe_input_sub = Some(sub);
        }
        if model.publish_channel_input_entity.is_none() {
            let entity = cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("发布通道")
            });
            let sub = cx.subscribe(
                &entity,
                move |this: &mut NavicatMain,
                      input,
                      event: &gpui_component::input::InputEvent,
                      cx| {
                    if matches!(event, gpui_component::input::InputEvent::Change)
                        && let Some(model) = this.pubsub_sessions.get_mut(&tab_id)
                    {
                        model.publish_channel = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            );
            model.publish_channel_input_entity = Some(entity);
            model.publish_channel_input_sub = Some(sub);
        }
        if model.publish_message_input_entity.is_none() {
            let entity = cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("消息内容")
            });
            let sub = cx.subscribe(
                &entity,
                move |this: &mut NavicatMain,
                      input,
                      event: &gpui_component::input::InputEvent,
                      cx| {
                    if matches!(event, gpui_component::input::InputEvent::Change)
                        && let Some(model) = this.pubsub_sessions.get_mut(&tab_id)
                    {
                        model.publish_message = input.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            );
            model.publish_message_input_entity = Some(entity);
            model.publish_message_input_sub = Some(sub);
        }
    }

    /// 首开会话时启动一个周期 pump：
    /// 每 tick 为每个存活会话派发一次一次性后台轮询，再在主线程消费回传结果并合并上屏。
    /// 后台任务读完即返回（不长期占用线程）；会话被移除后不再派发，回传通道随之失效。
    fn ensure_pubsub_pump(&mut self, cx: &mut gpui::Context<NavicatMain>) {
        if self._pubsub_pump.is_some() {
            return;
        }
        let task = cx.spawn(async move |view, cx| {
            let mut timer = cx.background_executor().timer(PUBSUB_PUMP_INTERVAL);
            loop {
                timer.await;
                timer = cx.background_executor().timer(PUBSUB_PUMP_INTERVAL);
                // 快照存活会话的共享句柄。
                let snapshot = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return Vec::new();
                    };
                    view.read(cx)
                        .pubsub_sessions
                        .iter()
                        .map(|(_, model)| {
                            (
                                model.session.clone(),
                                model.commands.clone(),
                                model.config.clone(),
                                model.database,
                                model.back_tx.clone(),
                            )
                        })
                        .collect::<Vec<_>>()
                });
                // 后台一次轮询（detach，不阻塞主循环）。
                for (session, commands, config, database, back_tx) in snapshot {
                    let _ = cx
                        .background_executor()
                        .spawn(async move {
                            let mut guard = session.lock().unwrap();
                            if guard.is_none() {
                                match fluxdb_app::RedisPubSubSession::connect(
                                    &config,
                                    database,
                                    PUBSUB_POLL_TIMEOUT,
                                ) {
                                    Ok(s) => *guard = Some(s),
                                    Err(e) => {
                                        let _ =
                                            back_tx.send(PubSubBackMsg::Failed(e.message.clone()));
                                        return;
                                    }
                                }
                            }
                            let session = guard.as_mut().unwrap();
                            // 应用等待写入的命令。
                            if let Ok(mut queue) = commands.lock() {
                                for command in queue.drain(..) {
                                    Self::apply_pubsub_command(session, command);
                                }
                            }
                            match session.poll() {
                                Ok(outcome) => {
                                    let _ = back_tx.send(PubSubBackMsg::Poll(outcome));
                                }
                                Err(e) => {
                                    let _ =
                                        back_tx.send(PubSubBackMsg::Failed(e.message.clone()));
                                }
                            }
                        })
                        .detach();
                }
                // 主线程合并回传结果。
                let alive = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return false;
                    };
                    view.update(cx, |this, cx| this.drain_pubsub_outcomes(cx));
                    true
                });
                if !alive {
                    break;
                }
            }
        });
        self._pubsub_pump = Some(task);
    }

    /// 把一条 UI 命令写入会话（订阅 / 取消 / 发布）。
    fn apply_pubsub_command(session: &mut fluxdb_app::RedisPubSubSession, command: PubSubCommand) {
        match command {
            PubSubCommand::Subscribe(channel) => session.subscribe(&channel),
            PubSubCommand::Unsubscribe(channel) => session.unsubscribe(&channel),
            PubSubCommand::Publish(channel, message) => session.publish(&channel, &message),
        }
    }

    /// 主线程消费各会话的回传结果并合并进 UI 呈现状态。
    fn drain_pubsub_outcomes(&mut self, cx: &mut gpui::Context<NavicatMain>) {
        for model in self.pubsub_sessions.values_mut() {
            let msgs: Vec<PubSubBackMsg> = model.back_rx.try_iter().collect();
            for msg in msgs {
                match msg {
                    PubSubBackMsg::Failed(message) => {
                        model.connecting = false;
                        model.error = Some(message);
                    }
                    PubSubBackMsg::Poll(outcome) => {
                        model.connected = true;
                        model.connecting = false;
                        model.error = if outcome.errors.is_empty() {
                            None
                        } else {
                            Some(outcome.errors.join("；"))
                        };
                        for event in &outcome.subscriptions {
                            if event.subscribed {
                                if !model.subscribed.contains(&event.channel) {
                                    model.subscribed.push(event.channel.clone());
                                }
                            } else {
                                model.subscribed.retain(|ch| ch != &event.channel);
                            }
                        }
                        for message in pubsub_messages_into_view(&outcome.messages) {
                            model.messages.push(message);
                        }
                        if model.messages.len() > PUBSUB_MAX_MESSAGES {
                            let excess = model.messages.len() - PUBSUB_MAX_MESSAGES;
                            model.messages.drain(..excess);
                        }
                        // 消息被裁剪后，若当前页已越界则回退到最后一页，避免空列表分页态。
                        let page_count = model.messages.len().div_ceil(model.message_page_size);
                        if page_count > 0 {
                            model.message_page = model.message_page.min(page_count - 1);
                        } else {
                            model.message_page = 0;
                        }
                        for published in &outcome.published {
                            model.last_publish_count = published.count;
                        }
                    }
                }
            }
        }
        cx.notify();
    }
}

/// 把连接器消息转成可展示视图（payload 按 UTF-8 文本呈现）。
///
/// 同一轮轮询的消息视为同一时刻到达，统一打上「接收时间」；该时间仅用于消息流表格的
/// 时间列展示，不参与连接器 / 业务逻辑。
fn pubsub_messages_into_view(messages: &[fluxdb_app::PubSubMessage]) -> Vec<PubSubMessageView> {
    let received_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    messages
        .iter()
        .map(|message| PubSubMessageView {
            pattern: message.pattern.clone(),
            channel: message.channel.clone(),
            payload: String::from_utf8_lossy(&message.payload).to_string(),
            received_at,
        })
        .collect()
}
