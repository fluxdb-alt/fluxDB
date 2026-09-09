impl NavicatMain {
    // 发起 string 值加载（preview 或完整值）。full=false 走 STRLEN+GETRANGE 预览，
    // full=true 走 GET 完整值。同一 (tab,key) 的在飞请求由 redis_string_loading 去重。
    fn request_redis_string_value_load(
        &mut self,
        tab_id: TabId,
        key: String,
        full: bool,
        cx: &mut Context<Self>,
    ) {
        // 同 key 已有在飞加载则跳过，避免重复请求覆盖 loading 状态
        if self
            .redis_string_loading
            .as_ref()
            .is_some_and(|loading| loading.0 == tab_id && loading.1 == key)
        {
            return;
        }
        self.redis_string_loading = Some((tab_id, key.clone(), full));
        let mut controller = self.controller.clone();
        let req_key = key.clone();
        let task_id = redis_string_value_load_task_id(tab_id);
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisStringValue {
                        tab_id,
                        key: req_key.clone(),
                        full,
                    }) {
                        AppEvent::RedisStringValueLoaded {
                            value,
                            len,
                            loaded_all,
                            ..
                        } => Ok(RedisStringValueState { value, len, loaded_all }),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载失败".to_string(),
                            message: "String 值加载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    this.redis_string_loading = None;
                    match result {
                        Ok(state) => {
                            this.redis_string_values.insert((tab_id, key.clone()), state);
                        }
                        Err(error) => {
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    // 下载 string / JSON 值到本地文件：大 key 也始终可用（不依赖完整加载），导出完整原始字节。
    // 先让用户选保存位置，再在后台取回字节并写盘，完成后统一上报结果。
    fn request_redis_string_download(
        &mut self,
        tab_id: TabId,
        detail: RedisKeyDetail,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        // 同一 (tab, key) 已有进行中的下载则跳过
        if self.redis_string_downloading.contains(&(tab_id, key.clone())) {
            return;
        }
        // 默认文件名：Key 安全片段；JSON 类型用 .json，其余（String 等）用 .txt，落在系统下载目录。
        let ext = if redis_key_value_is_json_kind(&detail.kind) { "json" } else { "txt" };
        let suggested = format!("{}.{}", safe_data_export_filename_segment(&key), ext);
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        self.redis_string_downloading.insert((tab_id, key.clone()));

        let mut controller = self.controller.clone();
        let task_id = redis_string_download_task_id(tab_id);
        let task = cx.spawn(async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(path))) => Some(path),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择保存位置失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
                Err(error) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择保存位置失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
            };
            // 用户取消保存：清除下载中标记后返回
            let Some(path) = path else {
                let _ = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this.redis_string_downloading.remove(&(tab_id, key.clone()));
                        cx.notify();
                    });
                });
                return;
            };

            // 后台取回完整原始字节（Bulk/Simple 原始 bytes，不经 UTF-8 容错）
            let fetch_key = key.clone();
            let fetch = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::DownloadRedisStringValue {
                        tab_id,
                        key: fetch_key,
                    }) {
                        AppEvent::RedisStringValueDownloaded { bytes, .. } => Ok(bytes),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "下载失败".to_string(),
                            message: "String 值下载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            // 取回成功则后台写盘（写盘用克隆 path，保留原 path 供成功提示），失败则透传错误消息
            let result: Result<(), String> = match fetch {
                Ok(bytes) => {
                    let path_for_write = path.clone();
                    cx.background_spawn(async move {
                        fs::write(&path_for_write, bytes).map_err(|error| error.to_string())
                    })
                    .await
                }
                Err(error) => Err(error.message),
            };

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_load_tasks.remove(&task_id);
                    this.redis_string_downloading.remove(&(tab_id, key.clone()));
                    match result {
                        Ok(()) => {
                            this.show_message(
                                format!("已导出到 {}", path.display()),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        Err(error) => {
                            this.show_message(error, AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(task_id, task);
        cx.notify();
    }

    // 每帧同步 string 值状态：
    // 1. 切换 key 时自动放弃未显式取消的编辑态；
    // 2. 对可编辑类型、尚未缓存值的 key 自动发起 preview 加载。
    fn sync_redis_string_value(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        let active = (tab_id, key.clone());

        // 离开即清理：抽屉由上一个 key 切换到当前 key 时，清除上一个 key 的 String 值缓存，
        // 使下次进入该 key 重新 preview 加载（避免完整值长期驻留前端）。
        if let Some((prev_tab, prev_key)) = self.redis_string_active.clone() {
            if prev_tab != tab_id || prev_key != key {
                self.clear_redis_string_cache_for(prev_tab, prev_key);
            }
        }
        self.redis_string_active = Some((tab_id, key.clone()));

        // 切换 key：残留的编辑态自动放弃（草稿一并清除，下帧 sync 会重置输入框）
        if let Some((editing_tab, editing_key)) = self.redis_string_editing.clone() {
            if editing_tab == tab_id && editing_key != key {
                self.redis_string_editing = None;
                self.redis_key_value_drafts
                    .remove(&(editing_tab, editing_key));
            }
        }

        // 自动加载 preview：可编辑类型且尚无缓存值、无在飞请求时才发起
        let should_load = redis_key_value_editable_for(&detail.kind, &detail.value)
            && !self.redis_string_values.contains_key(&active)
            && !self
                .redis_string_loading
                .as_ref()
                .is_some_and(|loading| loading.0 == tab_id && loading.1 == key);
        if should_load {
            self.request_redis_string_value_load(tab_id, key, false, cx);
        }
    }

    // 清理某个 (tab, key) 的 String 值缓存：值、格式、以及仍残留的编辑态/草稿。
    fn clear_redis_string_cache_for(&mut self, tab_id: TabId, key: String) {
        self.redis_string_values.remove(&(tab_id, key.clone()));
        self.redis_string_format.remove(&(tab_id, key.clone()));
        if self.redis_string_editing.as_ref() == Some(&(tab_id, key.clone())) {
            self.redis_string_editing = None;
            self.redis_key_value_drafts.remove(&(tab_id, key));
        }
    }

    // 抽屉关闭（无选中行）时清理当前 string key 缓存，使下次进入重新 preview 加载。
    fn redis_string_evict_active_cache(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some((prev_tab, prev_key)) = self.redis_string_active.take() {
            if prev_tab == tab_id {
                self.clear_redis_string_cache_for(prev_tab, prev_key);
            }
        }
        cx.notify();
    }

    // 进入 string 编辑态：以「已完整加载值」为输入框基线，避免把 preview 片段带进编辑。
    fn begin_redis_string_edit(
        &mut self,
        tab_id: TabId,
        detail: RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if redis_value_is_binary(&detail.value) {
            // 二进制值不可编辑（编辑按钮已禁用，这里是双保险）
            return;
        }
        let key = detail.key.clone();
        let loaded = self
            .redis_string_values
            .get(&(tab_id, key.clone()))
            .map(|state| state.value.clone())
            .unwrap_or_else(|| detail.value.clone());
        self.redis_key_value_drafts
            .insert((tab_id, key.clone()), loaded.clone());
        self.redis_string_editing = Some((tab_id, key));
        let input = self.redis_key_value_input_for_kind(&detail.kind);
        self.redis_key_value_syncing = true;
        input.update(cx, |input, cx| input.set_value(loaded, window, cx));
        self.redis_key_value_syncing = false;
        input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    // 取消 string 编辑：清除编辑态与草稿，下帧 sync 会把输入框重置回已加载值。
    fn cancel_redis_string_edit(
        &mut self,
        tab_id: TabId,
        detail: RedisKeyDetail,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        self.redis_string_editing = None;
        self.redis_key_value_drafts.remove(&(tab_id, key));
        cx.notify();
    }
}
