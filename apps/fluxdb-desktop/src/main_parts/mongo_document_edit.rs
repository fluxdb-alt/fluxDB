// MongoDB 文档编辑：右侧抽屉里整篇替换一篇文档、把一篇文档复制成新文档，或新增一篇文档。
//
// 交互与 Compass 一致：卡片头的编辑按钮打开抽屉，抽屉即编辑器；保存成功后重新查询本页
// 并关闭抽屉。`_id` 是定位条件、不可修改（连接器侧同样拒绝），因此这里只做整篇替换，
// 不做逐字段编辑——那需要把 BSON 类型拆开再拼回去，收益不抵复杂度。
//
// 「复制文档」与结果条的「+」复用同一个抽屉外壳（`Duplicate` / `Insert`）：正文都是同一篇
// 文档的 JSON 编辑，差别只在落库动作（替换 vs 插入）、初值与头部文案。统一容器让用户只学
// 一套「打开 / 取消 / 关闭」语义，也不必被居中弹框的遮罩挡住右侧文档列表。
// 新增的初值只预填一个客户端生成的 ObjectId：插入前就能看到 `_id`，插完可直接拿去查询。
//
// 编辑草稿只存在于界面的编辑会话里，不写回 `MongoCollectionState`：取消、切页、关标签即丢弃。
// 保存失败时草稿必须留在抽屉里，用户改完可以直接重试，不能因为一次网络/权限失败白丢输入。
// 抽屉自带的「取消 / 关闭」本来就是明确的丢弃动作，不再追问；但点另一张卡片的编辑按钮
// 属于隐式切换，有未保存改动时先弹确认框，避免辛辛苦苦改完点错一下就没了。

/// 编辑抽屉的宽度：比只读详情宽一档，给多行 JSON 编辑器留出可读的行宽。
const MONGO_DOCUMENT_EDITOR_WIDTH: f32 = 520.;

/// 抽屉在干什么：决定头部文案、底栏按钮与落库命令。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MongoDocumentEditorMode {
    /// 整篇替换：落库走 `SaveMongoDocument`，先按原 `_id` 定位再覆盖。
    Edit,
    /// 复制为新文档：正文已剥掉 `_id`，落库走 `InsertMongoDocument` 由服务端生成新 `_id`。
    Duplicate,
    /// 新增文档：正文只预填一个本机生成的 `_id`，其余字段由用户补。
    Insert,
}

impl MongoDocumentEditorMode {
    /// 是否「插入一篇文档」（复制或新增）：底栏主按钮走插入，而不是整篇替换。
    fn is_insert(self) -> bool {
        matches!(self, Self::Duplicate | Self::Insert)
    }
}

/// 一次编辑会话：`JsonComponent` 自带多行编辑框（JSON 语法高亮 + 树内建诊断）。
struct MongoDocumentEditor {
    tab_id: TabId,
    /// 本页下标：用来判断「点的是不是同一篇」，切换文档时才需要确认。
    /// 插入类模式（复制 / 新增）没有源文档位置，固定为 `None`。
    index: Option<usize>,
    mode: MongoDocumentEditorMode,
    /// 打开时的 pretty 基线：既用于判断是否真改过，也用于让连接器取原始 `_id` 定位。
    /// 插入类模式下是初值副本，只用于判断「有没有改过」。
    base_json: String,
    /// 复制模式下源文档的 `_id` 文本，用于抽屉副标题——用户不必回头确认复制的是哪一篇。
    /// 编辑与新增模式用不到。
    source_id: String,
    component: JsonComponent,
    /// 保存/插入请求在途：按钮禁用，避免重复提交。
    saving: bool,
    /// 编辑框内容变化 → 重绘抽屉：保存按钮的可用状态取决于草稿是否改过，
    /// 而编辑框只通知自己，不重绘外层，订阅是让按钮及时变灰/变亮的唯一途径。
    _subscription: Subscription,
}

/// 待确认的文档切换：当前草稿有未保存改动时，先让用户选「继续编辑」还是「放弃修改」。
#[derive(Clone, Copy)]
struct PendingMongoDocumentSwitch {
    tab_id: TabId,
    index: usize,
}

/// 打开抽屉的待办请求：草稿有未保存改动时先确认，确认后再按它重开抽屉。
#[derive(Clone)]
enum MongoDocumentEditorRequest {
    /// 复制某篇文档。留着源文档 JSON 而不是下标：确认期间可能重查过本页，
    /// 下标会失效，原始内容不会。
    Duplicate { document_json: String },
    /// 新增一篇文档（只预填 `_id`，没有源文档可存）。
    Insert,
}

/// 待确认的抽屉切换：草稿有未保存改动时点了「复制文档」或结果条的「+」。
#[derive(Clone)]
struct PendingMongoDocumentOpen {
    tab_id: TabId,
    request: MongoDocumentEditorRequest,
}

/// 待确认的文档删除。
///
/// 删除不可撤销，必须先让用户确认；`document_json` 是点击那一刻的文档内容，
/// 连接器用它取原始 `_id` 定位——确认期间翻页/刷新不会删错另一篇。
#[derive(Clone)]
struct PendingMongoDocumentDelete {
    tab_id: TabId,
    document_json: String,
}

/// 把文档的 Extended JSON 去掉 `_id`，并取出该 `_id` 的可读文本。
///
/// 解析失败时原样返回（连接器会再校验一次），不在这里消化掉错误。
fn mongo_document_without_id(document_json: &str) -> (String, String) {
    let Ok(mut value) = serde_json::from_str::<JsonValue>(document_json) else {
        return (document_json.to_string(), "—".to_string());
    };
    let source_id = value
        .get("_id")
        .map(mongo_id_text)
        .unwrap_or_else(|| "—".to_string());
    if let JsonValue::Object(fields) = &mut value {
        // 必须用 `shift_remove`：`preserve_order` 生效时 `Map::remove` 等价于
        // `swap_remove`，会把最后一个字段搬到 `_id` 的位置，副本字段顺序与卡片对不上。
        fields.shift_remove("_id");
    }
    (
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| document_json.to_string()),
        source_id,
    )
}

/// `_id` 值的可读文本：单键 BSON 包装（`$oid` / `$date` …）取内层值，其余转字符串。
fn mongo_id_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Object(map) if map.len() == 1 => map
            .values()
            .next()
            .map(|inner| match inner {
                JsonValue::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "—".to_string()),
        JsonValue::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// 本机生成一个 24 位十六进制的 ObjectId 文本。
///
/// 与驱动同构：4 字节秒级时间戳 + 5 字节进程内固定随机数 + 3 字节自增计数。不为此引入
/// `bson` / `rand` 依赖：界面只需要一个与驱动同格式、进程内不会撞的值，连接器会把它
/// 当作 `{"$oid": …}` 解析成真正的 ObjectId。
fn mongo_new_object_id() -> String {
    use std::hash::BuildHasher as _;
    use std::sync::OnceLock;

    // 进程内固定：同一进程生成的 ObjectId 只是计数与时间不同，与驱动行为一致。
    static PROCESS_RANDOM: OnceLock<[u8; 5]> = OnceLock::new();
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    let random = PROCESS_RANDOM.get_or_init(|| {
        // `RandomState` 自带每进程随机种子，省掉一条随机数依赖。
        let seed = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let bytes = seed.to_be_bytes();
        [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]]
    });
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as u32)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed) & 0x00ff_ffff;

    let mut hex = String::with_capacity(24);
    for byte in seconds
        .to_be_bytes()
        .iter()
        .chain(random.iter())
        .chain(counter.to_be_bytes()[1..].iter())
    {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// 新增文档的初值：只预填一个 `_id`，其余字段由用户补。
///
/// 用 `{"$oid": …}` 而不是裸字符串：裸的 24 位十六进制串会被当成字符串 `_id`，
/// 与集合里既有的 ObjectId 类型对不上。
fn mongo_insert_draft() -> String {
    format!(
        "{{\n  \"_id\": {{\n    \"$oid\": \"{}\"\n  }}\n}}",
        mongo_new_object_id()
    )
}

/// 草稿是否真的改过（与基线比对，忽略首尾空白）。
fn mongo_document_editor_dirty(editor: &MongoDocumentEditor, cx: &Context<NavicatMain>) -> bool {
    editor.component.edit_text(cx).trim() != editor.base_json.trim()
}

impl NavicatMain {
    /// 打开某篇文档的抽屉。视图（只读对象）只给只读详情，不进入编辑态。
    pub(crate) fn open_mongo_document_drawer(
        &mut self,
        tab_id: TabId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 已有未保存草稿又去开另一篇：先确认，别静默丢弃。
        if self.mongo_document_editor.as_ref().is_some_and(|editor| {
            editor.tab_id == tab_id
                && editor.index != Some(index)
                && mongo_document_editor_dirty(editor, cx)
        }) {
            self.pending_mongo_document_switch = Some(PendingMongoDocumentSwitch { tab_id, index });
            cx.notify();
            return;
        }
        let Some((json, read_only)) = mongo_collection_state(self.controller.state(), tab_id)
            .and_then(|state| {
                state
                    .page
                    .as_ref()
                    .and_then(|page| page.documents.get(index))
                    .map(|document| (document.json.clone(), state.read_only))
            })
        else {
            return;
        };
        if read_only {
            // 只读对象没有可编辑的落点，直接给详情抽屉（同一位置、同一外壳）。
            self.select_mongo_document(tab_id, Some(index), cx);
            cx.notify();
            return;
        }
        let mut component =
            JsonComponent::new_with_mode(JsonEditorConfig::default(), JsonMode::Edit, window, cx);
        component.load(&json);
        // 以 pretty 后的文本作为编辑基线：用户不动内容时保存按钮应保持禁用。
        let base_json = component.source();
        component.set_edit_text(base_json.clone(), window, cx);
        component.focus_edit(window, cx);

        let subscription = cx.subscribe(
            component.input().expect("Edit 模式必持有编辑框"),
            |this: &mut Self, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
                let _ = this;
            },
        );

        // 抽屉复用 `selected_document`：打开编辑器时卡片同样高亮。
        self.select_mongo_document(tab_id, Some(index), cx);
        self.mongo_document_editor = Some(MongoDocumentEditor {
            tab_id,
            index: Some(index),
            mode: MongoDocumentEditorMode::Edit,
            base_json,
            source_id: String::new(),
            component,
            saving: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// 关闭抽屉（编辑态、复制态或只读详情），不写任何数据。
    pub(crate) fn close_mongo_document_drawer(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.mongo_document_editor = None;
        // 整篇清空选中：复制会话本来就没选中任何一行，编辑会话关闭后也不该留着高亮。
        self.select_mongo_document(tab_id, None, cx);
        cx.notify();
    }

    /// 确认放弃草稿并切到目标文档。
    pub(crate) fn confirm_mongo_document_switch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_mongo_document_switch.take() else {
            return;
        };
        self.mongo_document_editor = None;
        self.open_mongo_document_drawer(pending.tab_id, pending.index, window, cx);
        cx.notify();
    }

    /// 确认框的「留在编辑器」：两个 pending 都清掉（同一时刻只会有一个）。
    pub(crate) fn cancel_mongo_document_discard(&mut self, cx: &mut Context<Self>) {
        self.pending_mongo_document_switch = None;
        self.pending_mongo_document_open = None;
        cx.notify();
    }

    /// 确认框的「放弃并继续」：按当初被打断的动作分流。
    pub(crate) fn confirm_mongo_document_discard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_mongo_document_open.is_some() {
            self.confirm_mongo_document_open_switch(window, cx);
        } else {
            self.confirm_mongo_document_switch(window, cx);
        }
    }

    /// 请求删除一篇文档：先弹确认框，用户确认后才真正发写请求。
    pub(crate) fn request_delete_mongo_document(
        &mut self,
        tab_id: TabId,
        document_json: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_mongo_document_delete = Some(PendingMongoDocumentDelete {
            tab_id,
            document_json,
        });
        cx.notify();
    }

    /// 关掉删除确认框，不写任何数据。
    pub(crate) fn cancel_mongo_document_delete(&mut self, cx: &mut Context<Self>) {
        self.pending_mongo_document_delete = None;
        cx.notify();
    }

    /// 确认删除：发后台写请求；成功后关闭抽屉并重查本页。
    pub(crate) fn confirm_mongo_document_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_mongo_document_delete.take() else {
            return;
        };
        let tab_id = pending.tab_id;
        let command = AppCommand::DeleteMongoDocument {
            tab_id,
            document_json: pending.document_json,
        };
        let task = self.spawn_controller_command(
            command,
            move |this, event, cx| {
                match event {
                    AppEvent::MongoDocumentDeleted(_) => {
                        // 被删的那篇可能正开在抽屉里：收起抽屉（含编辑会话），
                        // 避免继续显示一篇已经不存在的文档。
                        this.close_mongo_document_drawer(tab_id, cx);
                        this.show_message(
                            tr("app.mongo.delete_done").as_ref(),
                            AppMessageKind::Success,
                            cx,
                        );
                        // 删除会改变本页文档数量与下标，重查拿服务端真实结果。
                        this.run_mongo_query(tab_id, cx);
                    }
                    AppEvent::Failed(error) => {
                        this.show_message(
                            tr("app.mongo.delete_failed").replace("%{error}", &error.message),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    _ => {}
                }
                cx.notify();
            },
            cx,
        );
        self._mongo_document_delete_task = Some(task);
        cx.notify();
    }

    /// 打开「复制文档」抽屉：文档剥掉 `_id` 后作为新文档的初值。
    pub(crate) fn open_mongo_document_duplicate(
        &mut self,
        tab_id: TabId,
        document_json: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_mongo_document_open(
            tab_id,
            MongoDocumentEditorRequest::Duplicate {
                document_json: document_json.to_string(),
            },
            window,
            cx,
        );
    }

    /// 打开「新增文档」抽屉：正文只预填一个本机生成的 ObjectId。
    ///
    /// 不预填任何业务字段——新增本来就没有源文档可抄，凭空造字段只会诱导用户留下垃圾数据。
    pub(crate) fn open_mongo_document_insert(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_mongo_document_open(tab_id, MongoDocumentEditorRequest::Insert, window, cx);
    }

    /// 打开插入类抽屉的统一入口：已有未保存草稿时先确认。
    ///
    /// 复制与新增都会把抽屉切走，静默丢弃用户刚编辑的内容比多问一句糟糕得多。
    fn request_mongo_document_open(
        &mut self,
        tab_id: TabId,
        request: MongoDocumentEditorRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mongo_document_editor.as_ref().is_some_and(|editor| {
            editor.tab_id == tab_id && mongo_document_editor_dirty(editor, cx)
        }) {
            self.pending_mongo_document_open = Some(PendingMongoDocumentOpen { tab_id, request });
            cx.notify();
            return;
        }
        self.start_mongo_document_editor(tab_id, request, window, cx);
    }

    /// 真正建立插入类会话（已确认过丢弃草稿，或本来就没有草稿）。
    fn start_mongo_document_editor(
        &mut self,
        tab_id: TabId,
        request: MongoDocumentEditorRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mode, draft, source_id) = match request {
            MongoDocumentEditorRequest::Duplicate { document_json } => {
                let (draft, source_id) = mongo_document_without_id(&document_json);
                (MongoDocumentEditorMode::Duplicate, draft, source_id)
            }
            MongoDocumentEditorRequest::Insert => (
                MongoDocumentEditorMode::Insert,
                mongo_insert_draft(),
                String::new(),
            ),
        };
        let mut component =
            JsonComponent::new_with_mode(JsonEditorConfig::default(), JsonMode::Edit, window, cx);
        component.load(&draft);
        // 以初值作为基线：没动过时「复制」不该亮着（副本必须与新内容不同才有意义）。
        let base_json = component.source();
        component.set_edit_text(base_json.clone(), window, cx);
        component.focus_edit(window, cx);
        let subscription = cx.subscribe(
            component.input().expect("Edit 模式必持有编辑框"),
            |this: &mut Self, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
                let _ = this;
            },
        );
        // 待插入的文档还没有对应的行，不该让任何卡片看起来被选中。
        self.select_mongo_document(tab_id, None, cx);
        self.mongo_document_editor = Some(MongoDocumentEditor {
            tab_id,
            index: None,
            mode,
            base_json,
            source_id,
            component,
            saving: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// 确认放弃草稿并切到插入类会话。
    pub(crate) fn confirm_mongo_document_open_switch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_mongo_document_open.take() else {
            return;
        };
        self.mongo_document_editor = None;
        self.start_mongo_document_editor(pending.tab_id, pending.request, window, cx);
        cx.notify();
    }

    /// 确认插入：把抽屉里的 JSON 插成一条新文档；成功后重查本页。复制与新增共用。
    ///
    /// 与编辑共用抽屉，所以按 `mode` 分流：只有插入类会话走插入。
    pub(crate) fn confirm_mongo_document_insert(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.mongo_document_editor.as_mut() else {
            return;
        };
        if !editor.mode.is_insert() || editor.saving {
            return;
        }
        let tab_id = editor.tab_id;
        let document_json = match format_pretty(&editor.component.edit_text(cx), MONGO_JSON_INDENT)
        {
            Ok(pretty) => pretty,
            Err(diagnostic) => {
                self.show_message(diagnostic.summary().as_str(), AppMessageKind::Error, cx);
                return;
            }
        };
        editor.saving = true;

        let task = self.spawn_controller_command(
            AppCommand::InsertMongoDocument {
                tab_id,
                document_json,
            },
            move |this, event, cx| {
                match event {
                    AppEvent::MongoDocumentInserted(_, new_id) => {
                        this.close_mongo_document_drawer(tab_id, cx);
                        // 提示里给出可读的新 `_id`：用户据此确认插进去的到底是哪一条。
                        let new_id = serde_json::from_str::<JsonValue>(&new_id)
                            .map(|value| mongo_id_text(&value))
                            .unwrap_or(new_id);
                        this.show_message(
                            tr("app.mongo.insert_done").replace("%{id}", &new_id),
                            AppMessageKind::Success,
                            cx,
                        );
                        // 新文档在排序里的位置未知（可能不在本页），重查拿真实结果。
                        this.run_mongo_query(tab_id, cx);
                    }
                    AppEvent::Failed(error) => {
                        // 失败保留抽屉与草稿：用户改完可直接重试。
                        if let Some(editor) = this.mongo_document_editor.as_mut() {
                            editor.saving = false;
                        }
                        this.show_message(
                            tr("app.mongo.insert_failed").replace("%{error}", &error.message),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    _ => {}
                }
                cx.notify();
            },
            cx,
        );
        self._mongo_document_insert_task = Some(task);
        cx.notify();
    }

    /// 提交整篇替换。JSON 非法时留在编辑器并给出定位错误，不发写请求。
    pub(crate) fn save_mongo_document_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.mongo_document_editor.as_mut() else {
            return;
        };
        if editor.saving {
            return;
        }
        let tab_id = editor.tab_id;
        let before_json = editor.base_json.clone();
        let after_json = editor.component.edit_text(cx);
        let after_json = match format_pretty(&after_json, MONGO_JSON_INDENT) {
            Ok(pretty) => pretty,
            Err(diagnostic) => {
                self.show_message(diagnostic.summary().as_str(), AppMessageKind::Error, cx);
                return;
            }
        };
        // 没改过就直接收起，不必白发一次写。
        if after_json.trim() == before_json.trim() {
            self.close_mongo_document_drawer(tab_id, cx);
            return;
        }
        editor.saving = true;

        let task = self.spawn_controller_command(
            AppCommand::SaveMongoDocument {
                tab_id,
                before_json,
                after_json,
            },
            move |this, event, cx| {
                match event {
                    AppEvent::MongoDocumentSaved(_) => {
                        this.close_mongo_document_drawer(tab_id, cx);
                        this.show_message(
                            tr("app.mongo.editor.saved").as_ref(),
                            AppMessageKind::Success,
                            cx,
                        );
                        // 整篇替换可能改变字段集合，重查本页拿服务端真实结果。
                        this.run_mongo_query(tab_id, cx);
                    }
                    AppEvent::Failed(error) => {
                        // 失败保留抽屉与草稿：用户改完可直接重试。
                        if let Some(editor) = this.mongo_document_editor.as_mut() {
                            editor.saving = false;
                        }
                        this.show_message(
                            tr("app.mongo.editor.save_failed").replace("%{error}", &error.message),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    _ => {}
                }
                cx.notify();
            },
            cx,
        );
        self._mongo_document_editor_task = Some(task);
        cx.notify();
    }
}

/// 删除文档的确认框。
///
/// 删除不可撤销，遮罩点击、Esc 与右上角关闭都按「取消」处理——破坏性操作只能由
/// 用户显式点「删除」按钮触发。
fn mongo_document_delete_modal(
    pending: &PendingMongoDocumentDelete,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .debug_selector(|| "mongo-document-delete-modal".into())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_mongo_document_delete(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(430.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .key_context("MongoDocumentDeleteModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_mongo_document_delete(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.mongo.delete_title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_mongo_document_delete(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(
                            // `_id` 是这段提示里唯一能区分文档的信息。
                            tr("app.mongo.delete_message")
                                .replace("%{id}", &mongo_pending_delete_id(pending)),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("mongo-document-delete-cancel")
                                .debug_selector(|| "mongo-document-delete-cancel".into())
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_mongo_document_delete(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("mongo-document-delete-confirm")
                                .debug_selector(|| "mongo-document-delete-confirm".into())
                                .label(tr("app.common.delete"))
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_mongo_document_delete(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

/// 从待删文档的 Extended JSON 里取出 `_id` 文本，用于确认框的提示语。
///
/// 解析失败（理论上不会发生，内容来自连接器）时退回占位符，不阻断删除确认。
fn mongo_pending_delete_id(pending: &PendingMongoDocumentDelete) -> String {
    let Ok(value) = serde_json::from_str::<JsonValue>(&pending.document_json) else {
        return "—".to_string();
    };
    value
        .get("_id")
        .map(mongo_id_text)
        .unwrap_or_else(|| "—".to_string())
}

/// 文档抽屉：卡头同详情（图标 + 标题 + 关闭），正文换成多行 JSON 编辑器，底部取消 / 保存。
///
/// 编辑、复制与新增共用这一副外壳，只有标题、副标题、提示语和主按钮按 `mode` 分化：
/// 正文本来就是同一篇文档的 JSON，三个动作的差别只在初值与落库方式。
fn mongo_document_editor_drawer(
    editor: &MongoDocumentEditor,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tab_id = editor.tab_id;
    let saving = editor.saving;
    let mode = editor.mode;
    let inserting = mode.is_insert();
    let duplicating = mode == MongoDocumentEditorMode::Duplicate;
    let dirty = mongo_document_editor_dirty(editor, cx);
    // 插入没有「修改基线」可参照：主按钮只要 JSON 合法就该能点（新文档本来就该是新内容）。
    let valid = format_pretty(&editor.component.edit_text(cx), MONGO_JSON_INDENT).is_ok();
    let can_submit = !saving && if inserting { valid } else { dirty };
    // 头部：复制模式下带一行源 `_id` 副标题，用户不必回头确认复制的是哪一篇。
    let header = if duplicating {
        div()
            .flex()
            .flex_col()
            .gap_0p5()
            .flex_1()
            .min_w(px(0.))
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(colors.text)
                    .child(tr("app.mongo.duplicate_title")),
            )
            .child(
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(tr("app.mongo.duplicate_from").replace("%{id}", &editor.source_id)),
            )
    } else {
        div()
            .flex_1()
            .text_size(ui_text_size(12.))
            .text_color(colors.text)
            .child(if inserting {
                tr("app.mongo.insert_title")
            } else {
                tr("app.mongo.editor.title")
            })
    };
    div()
        .w(px(MONGO_DOCUMENT_EDITOR_WIDTH))
        .flex_none()
        .h_full()
        .border_l_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .debug_selector(move || match mode {
            MongoDocumentEditorMode::Duplicate => "mongo-document-duplicate-drawer".into(),
            MongoDocumentEditorMode::Insert => "mongo-document-insert-drawer".into(),
            MongoDocumentEditorMode::Edit => "mongo-document-editor".into(),
        })
        .child(
            div()
                .h(px(MONGO_TOOLBAR_HEIGHT))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(colors.border)
                .child(app_icon(
                    match mode {
                        MongoDocumentEditorMode::Duplicate => AppIcon::CopyPlus,
                        MongoDocumentEditorMode::Insert => AppIcon::Plus,
                        MongoDocumentEditorMode::Edit => AppIcon::Edit,
                    },
                    14.,
                    if inserting { colors.text } else { colors.info },
                ))
                .child(header)
                .child(
                    // 抽屉头只有 38px 高：通用 30px 描边按钮会顶满整条头部、显得笨重，
                    // 这里用卡片头同款 24px 无边框图标按钮，hover 才出色块。
                    mongo_card_icon_button(
                        format!("mongo-document-drawer-close-{}", tab_id.0),
                        "mongo-document-editor-close".to_string(),
                        AppIcon::Close,
                        tr("app.common.close"),
                        colors.text,
                        true,
                        false,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.close_mongo_document_drawer(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.mongo.extended_json"))
                .child(div().flex_1())
                .child(if duplicating {
                    tr("app.mongo.duplicate_hint")
                } else if inserting {
                    tr("app.mongo.insert_hint")
                } else {
                    tr("app.mongo.editor.hint")
                }),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .p_2()
                .child(editor.component.render_edit_body(colors, saving)),
        )
        .child(
            div()
                .px_2()
                .py_2()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .border_t_1()
                .border_color(colors.border)
                .child(
                    Button::new(("mongo-document-editor-cancel", tab_id.0))
                        .debug_selector(|| "mongo-document-editor-cancel".into())
                        .label(tr("app.common.cancel"))
                        .small()
                        .outline()
                        .w(px(78.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_mongo_document_drawer(tab_id, cx);
                            cx.stop_propagation();
                        })),
                )
                .child(
                    // 编辑：没改动或保存中时禁用，避免空写与重复提交。
                    // 插入（复制 / 新增）：JSON 非法时禁用，主按钮永远不可能发出坏写入。
                    Button::new(("mongo-document-editor-submit", tab_id.0))
                        .debug_selector(move || match mode {
                            MongoDocumentEditorMode::Duplicate => {
                                "mongo-document-duplicate-confirm".into()
                            }
                            MongoDocumentEditorMode::Insert => {
                                "mongo-document-insert-confirm".into()
                            }
                            MongoDocumentEditorMode::Edit => "mongo-document-editor-save".into(),
                        })
                        .label(if inserting {
                            tr("app.mongo.insert_confirm")
                        } else {
                            tr("app.mongo.editor.save")
                        })
                        .small()
                        .primary()
                        .w(px(78.))
                        .disabled(!can_submit)
                        .loading(saving)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if inserting {
                                this.confirm_mongo_document_insert(cx);
                            } else {
                                this.save_mongo_document_edit(cx);
                            }
                            cx.stop_propagation();
                        })),
                ),
        )
}

/// 有未保存草稿时切换文档的确认框：留在编辑器 / 放弃修改并切换。
///
/// 遮罩点击与 Esc（全局 `escape → CancelDialog`）都按「留下继续编辑」处理——
/// 破坏性操作只能是用户显式点按钮。
fn mongo_document_discard_modal(
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .debug_selector(|| "mongo-document-discard".into())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_mongo_document_discard(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(430.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .key_context("MongoDocumentDiscardModal")
                .track_focus(&focus_handle)
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_mongo_document_discard(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.mongo.editor.discard_title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_mongo_document_discard(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(tr("app.mongo.editor.discard_message")),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("mongo-document-discard-cancel")
                                .debug_selector(|| "mongo-document-discard-cancel".into())
                                .label(tr("app.mongo.editor.discard_stay"))
                                .w(px(90.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_mongo_document_discard(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("mongo-document-discard-confirm")
                                .debug_selector(|| "mongo-document-discard-confirm".into())
                                .label(tr("app.mongo.editor.discard_confirm"))
                                .danger()
                                .w(px(110.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_mongo_document_discard(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

#[cfg(test)]
mod mongo_document_delete_tests {
    use super::*;

    fn pending(json: &str) -> PendingMongoDocumentDelete {
        PendingMongoDocumentDelete {
            tab_id: TabId(1),
            document_json: json.to_string(),
        }
    }

    /// 确认框要能说清删的是哪一篇：ObjectId / 字符串 / 内嵌文档都要取出可读值。
    #[test]
    fn delete_prompt_shows_readable_id() {
        assert_eq!(
            mongo_pending_delete_id(&pending(r#"{"_id":{"$oid":"651700000000000000000001"}}"#)),
            "651700000000000000000001"
        );
        assert_eq!(
            mongo_pending_delete_id(&pending(r#"{"_id":"fluxdb-e2e-delete"}"#)),
            "fluxdb-e2e-delete"
        );
        assert_eq!(mongo_pending_delete_id(&pending(r#"{"_id":42}"#)), "42");
    }

    /// 内容异常时退回占位符，不因为提示语取不到 `_id` 就阻断删除确认。
    #[test]
    fn delete_prompt_falls_back_when_id_missing() {
        assert_eq!(mongo_pending_delete_id(&pending(r#"{"name":"x"}"#)), "—");
        assert_eq!(mongo_pending_delete_id(&pending("not json")), "—");
    }
}

#[cfg(test)]
mod mongo_document_insert_tests {
    use super::*;

    /// 新 `_id` 必须是 24 位十六进制：短一位、混入非十六进制字符，驱动都会当成字符串。
    #[test]
    fn new_object_id_is_24_hex_chars() {
        let id = mongo_new_object_id();
        assert_eq!(id.len(), 24, "ObjectId 文本必须是 24 位: {id}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "必须是 24 位小写十六进制: {id}"
        );
    }

    /// 同一进程连续生成不能重复（计数器兜底），否则插入第二次就撞主键。
    #[test]
    fn new_object_ids_do_not_repeat() {
        let ids: Vec<String> = (0..64).map(|_| mongo_new_object_id()).collect();
        let unique: BTreeSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "同一进程内的 _id 不能重复");
    }

    /// 初值必须是「只有 `_id` 的 Extended JSON 对象」，且 `_id` 用 `$oid` 包装。
    ///
    /// 包装不能省：裸的 24 位十六进制串会被当成字符串 `_id`，与集合里既有的
    /// ObjectId 类型对不上，插进去就查不回来。
    #[test]
    fn insert_draft_only_has_object_id_field() {
        let draft = mongo_insert_draft();
        let value: JsonValue = serde_json::from_str(&draft).expect("初值必须是合法 JSON");
        let fields = value.as_object().expect("初值必须是对象");
        assert_eq!(fields.len(), 1, "初值只应有 _id: {draft}");
        let id = fields["_id"]["$oid"]
            .as_str()
            .unwrap_or_else(|| panic!("_id 必须用 $oid 包装: {draft}"));
        assert_eq!(id.len(), 24, "初值里的 _id 也必须是 24 位: {draft}");
    }
}

#[cfg(test)]
mod mongo_document_duplicate_tests {
    use super::*;

    /// 复制时必须剥掉 `_id`，否则带着源文档的 `_id` 插入必然撞唯一键。
    #[test]
    fn duplicate_draft_drops_id_but_keeps_other_fields() {
        let (draft, source_id) = mongo_document_without_id(
            r#"{"_id":{"$oid":"651700000000000000000001"},"name":"a","n":1}"#,
        );
        assert_eq!(source_id, "651700000000000000000001");
        let value: JsonValue = serde_json::from_str(&draft).unwrap();
        assert!(value.get("_id").is_none(), "副本不能带 _id: {draft}");
        assert_eq!(value["name"], serde_json::json!("a"));
        assert_eq!(value["n"], serde_json::json!(1));
    }

    /// 字符串 `_id` 同样剥掉，且提示语取到的是可读值。
    #[test]
    fn duplicate_draft_handles_string_id() {
        let (draft, source_id) = mongo_document_without_id(r#"{"_id":"probe","x":true}"#);
        assert_eq!(source_id, "probe");
        assert!(!draft.contains("_id"), "副本不能带 _id: {draft}");
    }

    /// 副本字段顺序必须与源文档一致（只少了 `_id`）。
    ///
    /// 回归：`Map::remove` 在 `preserve_order` 下是 `swap_remove`，会把末尾字段搬到
    /// `_id` 的位置——弹框字段顺序与卡片对不上，用户会以为复制错了文档。
    #[test]
    fn duplicate_draft_keeps_field_order() {
        let (draft, _) = mongo_document_without_id(
            r#"{"_id":{"$oid":"651700000000000000000001"},"name":"a","at":{"$date":"2026-10-22T08:30:00.000Z"},"amt":{"$numberDecimal":"100"}}"#,
        );
        let keys: Vec<String> = serde_json::from_str::<JsonValue>(&draft)
            .expect("草稿必须是合法 JSON")
            .as_object()
            .expect("草稿必须是对象")
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, ["name", "at", "amt"], "字段顺序不能变: {draft}");
    }

    /// BSON 类型标注要原样保留：复制不该把 Date / Decimal128 降级成字符串。
    #[test]
    fn duplicate_draft_keeps_bson_type_wrappers() {
        let (draft, _) = mongo_document_without_id(
            r#"{"_id":{"$oid":"651700000000000000000001"},"at":{"$date":"2026-10-22T08:30:00.000Z"},"amt":{"$numberDecimal":"100"}}"#,
        );
        let value: JsonValue = serde_json::from_str(&draft).unwrap();
        assert_eq!(
            value["at"]["$date"],
            serde_json::json!("2026-10-22T08:30:00.000Z")
        );
        assert_eq!(value["amt"]["$numberDecimal"], serde_json::json!("100"));
    }

    /// 非法输入不在这里消化错误：原样交给连接器校验，`_id` 提示退回占位符。
    #[test]
    fn duplicate_draft_passes_through_invalid_json() {
        let (draft, source_id) = mongo_document_without_id("not json");
        assert_eq!(draft, "not json");
        assert_eq!(source_id, "—");
    }
}
