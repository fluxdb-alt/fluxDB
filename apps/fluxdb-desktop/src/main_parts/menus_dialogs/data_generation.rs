// Included in crate-root scope by ../menus_dialogs.rs; 测试数据生成弹窗。
//
// 与导入向导共用“常规 + 消息日志”版式和日志组件：生成只负责产出数据，
// 写入仍走导入管线，因此进度、取消、清空语义与导入完全一致。

/// 单列生成配置。外键目标可能来自物理外键，也可能来自用户在 ER 设计里
/// 维护的逻辑关系，两种来源在打开弹窗时已归一为 `ForeignKeyInfo`。
#[derive(Clone, Debug, PartialEq)]
struct GenerationColumnForm {
    name: String,
    type_name: String,
    nullable: bool,
    primary_key: bool,
    /// 表结构上的唯一约束：勾选后不可关闭，生成值必须唯一。
    unique_locked: bool,
    /// 当前是否要求唯一（约束列恒为 true，其余列由用户开关控制）。
    unique: bool,
    generator: String,
    reference: Option<fluxdb_core::GenerationReference>,
    reference_label: Option<String>,
    /// 外键取值方式：空为随机，`sequence` 为顺序。
    pick: String,
    /// 父表取值池：打开弹窗时预取，预览与生成共用；生成前应用层会再刷新。
    reference_pool: Vec<fluxdb_core::GenerationValue>,
}

/// 右栏页签：数据预览与生成日志共用一栏，不再占用弹窗顶部。
#[derive(Clone, Copy, Debug, PartialEq)]
enum GenerationPreviewTab {
    Data,
    Sql,
    Log,
}

/// 生成器选择下拉：锚在触发的生成器按钮下方，固定高度、可搜索、可滚动。
#[derive(Clone)]
struct DataGenerationPicker {
    /// 正在编辑的列名。
    column: String,
    search: Entity<InputState>,
    /// 触发按钮左下角（窗口坐标）；按钮尚未绘制出时回退到点击位置。
    anchor: Point<Pixels>,
    /// 被折叠的分组；搜索时忽略折叠，保证命中项可见。
    collapsed: Vec<fluxdb_core::GeneratorFamily>,
}

/// 生成弹窗状态。
#[derive(Clone, Debug)]
struct TableDataGenerationForm {
    object: ObjectPath,
    kind: DatabaseKind,
    /// 连接展示名，仅用于头部路径摘要。
    connection_name: String,
    loading: bool,
    error: Option<String>,
    columns: Vec<GenerationColumnForm>,
    /// 原始表结构，用于「重新自动推断 / 全部重置」重建列配置。
    source_columns: Vec<fluxdb_app::DataGenerationSourceColumn>,
    foreign_keys: Vec<ForeignKeyInfo>,
    mode: fluxdb_core::TableImportMode,
    locale: String,
    unique_conflict: fluxdb_core::UniqueConflict,
    preview_tab: GenerationPreviewTab,
    logs: Vec<ImportDialogLogEntry>,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    generated_rows: u64,
    skipped_rows: u64,
    running: bool,
    /// 运行中用于跨线程中止；结束后清空。
    cancel: Option<Arc<AtomicBool>>,
    task_center_id: Option<u64>,
    rows_input: Entity<InputState>,
    seed_input: Entity<InputState>,
    /// 每批写入行数；空或非法时按 500 处理。
    batch_input: Entity<InputState>,
    filter_input: Entity<InputState>,
    /// 每列参数输入框；键为「列 + 生成器 + 参数名」。输入框由渲染创建，执行时从这里读值，
    /// 避免在事件回调里访问窗口元素状态。
    params_inputs: Rc<std::cell::RefCell<BTreeMap<String, Entity<InputState>>>>,
    /// 每列空值比例输入框，键为列名。
    null_inputs: Rc<std::cell::RefCell<BTreeMap<String, Entity<InputState>>>>,
    /// 每列生成器按钮左下角的窗口坐标，键为列名；绘制时记录，打开下拉时定位。
    generator_anchors: Rc<std::cell::RefCell<BTreeMap<String, Point<Pixels>>>>,
}

/// 结构化参数控件定义；`label` 是 i18n key。
struct GenerationParam {
    key: &'static str,
    label: &'static str,
    width: f32,
}

/// 各生成器的可编辑参数。没有定义即表示该生成器不需要参数。
fn generation_param_fields(generator: &str) -> &'static [GenerationParam] {
    const SEQUENCE: &[GenerationParam] = &[
        GenerationParam { key: "start", label: "app.data.data_generation.param_start", width: 74. },
        GenerationParam { key: "step", label: "app.data.data_generation.param_step", width: 64. },
    ];
    const INT: &[GenerationParam] = &[
        GenerationParam { key: "min", label: "app.data.data_generation.param_min", width: 74. },
        GenerationParam { key: "max", label: "app.data.data_generation.param_max", width: 74. },
    ];
    const DECIMAL: &[GenerationParam] = &[
        GenerationParam { key: "min", label: "app.data.data_generation.param_min", width: 74. },
        GenerationParam { key: "max", label: "app.data.data_generation.param_max", width: 74. },
        GenerationParam { key: "scale", label: "app.data.data_generation.param_scale", width: 54. },
    ];
    const RANGE: &[GenerationParam] = &[
        GenerationParam { key: "low", label: "app.data.data_generation.param_low", width: 70. },
        GenerationParam { key: "high", label: "app.data.data_generation.param_high", width: 70. },
        GenerationParam { key: "step", label: "app.data.data_generation.param_step", width: 54. },
    ];
    const RATIO: &[GenerationParam] =
        &[GenerationParam { key: "ratio", label: "app.data.data_generation.param_ratio", width: 54. }];
    const DIGIT: &[GenerationParam] =
        &[GenerationParam { key: "count", label: "app.data.data_generation.param_count", width: 54. }];
    const CONSTANT: &[GenerationParam] =
        &[GenerationParam { key: "value", label: "app.data.data_generation.param_value", width: 200. }];
    const VALUES: &[GenerationParam] =
        &[GenerationParam { key: "values", label: "app.data.data_generation.param_values", width: 260. }];
    const REGEX: &[GenerationParam] =
        &[GenerationParam { key: "pattern", label: "app.data.data_generation.param_pattern", width: 260. }];
    const FORMAT: &[GenerationParam] =
        &[GenerationParam { key: "pattern", label: "app.data.data_generation.param_format", width: 200. }];
    const TEMPLATE: &[GenerationParam] =
        &[GenerationParam { key: "template", label: "app.data.data_generation.param_template", width: 300. }];
    const DOMAIN: &[GenerationParam] =
        &[GenerationParam { key: "domain", label: "app.data.data_generation.param_domain", width: 160. }];
    const PRICE: &[GenerationParam] = &[
        GenerationParam { key: "min", label: "app.data.data_generation.param_min", width: 74. },
        GenerationParam { key: "max", label: "app.data.data_generation.param_max", width: 74. },
    ];
    const BETWEEN: &[GenerationParam] = &[
        GenerationParam { key: "from", label: "app.data.data_generation.param_from", width: 112. },
        GenerationParam { key: "to", label: "app.data.data_generation.param_to", width: 112. },
    ];
    const DAYS: &[GenerationParam] =
        &[GenerationParam { key: "days", label: "app.data.data_generation.param_days", width: 66. }];
    const SOURCE: &[GenerationParam] =
        &[GenerationParam { key: "source", label: "app.data.data_generation.param_source", width: 200. }];
    match generator {
        "sequence" => SEQUENCE,
        "random.int" => INT,
        "random.decimal" => DECIMAL,
        "random.range" => RANGE,
        "random.boolean" => RATIO,
        "random.digit" => DIGIT,
        "constant" => CONSTANT,
        "categorical" => VALUES,
        "regex" => REGEX,
        "format" => FORMAT,
        "template" => TEMPLATE,
        "internet.email_safe" => DOMAIN,
        "commerce.price" => PRICE,
        "date.datetime_between" => BETWEEN,
        "date.recent" => DAYS,
        "lorem.word" | "lorem.sentence" | "lorem.paragraph" => DIGIT,
        "copy_field" => SOURCE,
        _ => &[],
    }
}

/// 参数默认值；与核心生成器的默认值保持一致，留空即使用这些值。
fn generation_param_default(generator: &str, key: &str) -> &'static str {
    match (generator, key) {
        ("sequence", "start") => "1",
        ("sequence", "step") => "1",
        ("random.int", "min") => "0",
        ("random.int", "max") => "1000",
        ("random.decimal", "min") => "0.00",
        ("random.decimal", "max") => "100.00",
        ("random.decimal", "scale") => "2",
        ("random.range", "low") => "0",
        ("random.range", "high") => "100",
        ("random.range", "step") => "1",
        ("random.boolean", "ratio") => "50",
        ("random.digit", "count") => "1",
        ("categorical", "values") => "a,b,c",
        ("internet.email_safe", "domain") => "example.com",
        ("commerce.price", "min") => "9.90",
        ("commerce.price", "max") => "9999.00",
        ("date.datetime_between", "from") => "2025-01-01",
        ("date.datetime_between", "to") => "2026-12-31",
        ("date.recent", "days") => "365",
        ("lorem.word", "count") => "1",
        ("lorem.sentence", "count") => "8",
        ("lorem.paragraph", "count") => "3",
        _ => "",
    }
}

impl TableDataGenerationForm {
    fn new(
        object: ObjectPath,
        kind: DatabaseKind,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) -> Self {
        Self {
            object,
            kind,
            connection_name: String::new(),
            loading: true,
            error: None,
            columns: Vec::new(),
            source_columns: Vec::new(),
            foreign_keys: Vec::new(),
            mode: fluxdb_core::TableImportMode::Append,
            locale: generation_locale(),
            unique_conflict: fluxdb_core::UniqueConflict::SkipRow,
            preview_tab: GenerationPreviewTab::Data,
            logs: Vec::new(),
            started_at: None,
            finished_at: None,
            generated_rows: 0,
            skipped_rows: 0,
            running: false,
            cancel: None,
            task_center_id: None,
            rows_input: cx.new(|cx| InputState::new(window, cx).default_value("100")),
            seed_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr("app.data.data_generation.seed_placeholder"))
            }),
            batch_input: cx.new(|cx| InputState::new(window, cx).default_value("500")),
            filter_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr("app.data.data_generation.filter_columns"))
            }),
            params_inputs: Rc::new(std::cell::RefCell::new(BTreeMap::new())),
            null_inputs: Rc::new(std::cell::RefCell::new(BTreeMap::new())),
            generator_anchors: Rc::new(std::cell::RefCell::new(BTreeMap::new())),
        }
    }

    /// 清空模式在部分数据库无法回滚，需要在界面上明确提示。
    fn truncate_warning(&self) -> Option<String> {
        if self.mode != fluxdb_core::TableImportMode::Truncate {
            return None;
        }
        Some(match self.kind {
            DatabaseKind::Postgres | DatabaseKind::Kingbase | DatabaseKind::Sqlite => {
                tr("app.data.data_import.truncate_rollback_safe").to_string()
            }
            _ => tr("app.data.data_import.truncate_rollback_unsafe").to_string(),
        })
    }
}

/// 单个参数输入框，按「列 + 生成器 + 参数名」缓存；创建时写入该参数的默认值。
///
/// 只在渲染期间调用；创建后把实体存进表单，执行时直接读值，不要在事件回调里
/// 访问窗口元素状态（窗口元素状态只在绘制阶段可用）。
fn generation_param_input(
    form: &TableDataGenerationForm,
    column: &str,
    generator: &str,
    param: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Entity<InputState> {
    let key = format!("{column}:{generator}:{param}");
    if let Some(input) = form.params_inputs.borrow().get(&key) {
        return input.clone();
    }
    let element_key = SharedString::from(format!("data-generation-param:{key}"));
    let default = generation_param_default(generator, param).to_string();
    let keyed = window.use_keyed_state(element_key, cx, move |window, cx| {
        InputState::new(window, cx).default_value(default.clone())
    });
    form.params_inputs
        .borrow_mut()
        .insert(key, keyed.clone());
    keyed
}

fn generation_param_value(
    form: &TableDataGenerationForm,
    column: &str,
    generator: &str,
    param: &str,
    cx: &App,
) -> String {
    let key = format!("{column}:{generator}:{param}");
    form.params_inputs
        .borrow()
        .get(&key)
        .map(|input| generation_input_text(input, cx))
        .unwrap_or_else(|| generation_param_default(generator, param).to_string())
}

/// 可空列的空值比例输入框；默认 0，执行时读回。
fn generation_null_input(
    form: &TableDataGenerationForm,
    column: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Entity<InputState> {
    if let Some(input) = form.null_inputs.borrow().get(column) {
        return input.clone();
    }
    let element_key = SharedString::from(format!("data-generation-null:{column}"));
    let keyed = window.use_keyed_state(element_key, cx, move |window, cx| {
        InputState::new(window, cx).default_value("0")
    });
    form.null_inputs
        .borrow_mut()
        .insert(column.to_string(), keyed.clone());
    keyed
}

fn generation_null_percent(
    form: &TableDataGenerationForm,
    column: &str,
    cx: &App,
) -> u8 {
    form.null_inputs
        .borrow()
        .get(column)
        .map(|input| generation_input_text(input, cx))
        .and_then(|text| text.trim().parse::<u8>().ok())
        .map(|value| value.min(100))
        .unwrap_or(0)
}

/// 每批写入行数上限：单条 INSERT 参数不能无限增长。
const DATA_GENERATION_BATCH_ROWS_MAX: usize = 5_000;

/// 读取批大小输入；空或非法按默认 500，超出范围交给调用方提示。
fn generation_batch_rows(form: &TableDataGenerationForm, cx: &App) -> Result<usize, String> {
    let text = generation_input_text(&form.batch_input, cx)
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(500);
    }
    let value = text
        .parse::<usize>()
        .map_err(|_| tr("app.data.data_generation.batch_invalid").to_string())?;
    if value == 0 || value > DATA_GENERATION_BATCH_ROWS_MAX {
        return Err(tr("app.data.data_generation.batch_range")
            .replace("%{max}", &DATA_GENERATION_BATCH_ROWS_MAX.to_string()));
    }
    Ok(value)
}

fn generation_input_text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn generation_generator_label(generator: &str) -> SharedString {
    match generator {
        "sequence" => tr("app.data.data_generation.generator.sequence"),
        "random.int" => tr("app.data.data_generation.generator.random_int"),
        "random.decimal" => tr("app.data.data_generation.generator.random_decimal"),
        "random.range" => tr("app.data.data_generation.generator.random_range"),
        "random.boolean" => tr("app.data.data_generation.generator.random_boolean"),
        "random.digit" => tr("app.data.data_generation.generator.random_digit"),
        "random.uuid" => tr("app.data.data_generation.generator.random_uuid"),
        "random.ulid" => tr("app.data.data_generation.generator.random_ulid"),
        "constant" => tr("app.data.data_generation.generator.constant"),
        "categorical" => tr("app.data.data_generation.generator.categorical"),
        "regex" => tr("app.data.data_generation.generator.regex"),
        "format" => tr("app.data.data_generation.generator.format"),
        "template" => tr("app.data.data_generation.generator.template"),
        "name.first" => tr("app.data.data_generation.generator.name_first"),
        "name.last" => tr("app.data.data_generation.generator.name_last"),
        "name.full" => tr("app.data.data_generation.generator.name_full"),
        "name.title" => tr("app.data.data_generation.generator.name_title"),
        "internet.email" => tr("app.data.data_generation.generator.internet_email"),
        "internet.email_safe" => tr("app.data.data_generation.generator.internet_email_safe"),
        "internet.username" => tr("app.data.data_generation.generator.internet_username"),
        "internet.domain" => tr("app.data.data_generation.generator.internet_domain"),
        "phone.number" => tr("app.data.data_generation.generator.phone_number"),
        "address.city" => tr("app.data.data_generation.generator.address_city"),
        "address.country" => tr("app.data.data_generation.generator.address_country"),
        "address.state" => tr("app.data.data_generation.generator.address_state"),
        "address.street" => tr("app.data.data_generation.generator.address_street"),
        "address.postcode" => tr("app.data.data_generation.generator.address_postcode"),
        "company.name" => tr("app.data.data_generation.generator.company_name"),
        "company.job" => tr("app.data.data_generation.generator.company_job"),
        "company.industry" => tr("app.data.data_generation.generator.company_industry"),
        "commerce.price" => tr("app.data.data_generation.generator.commerce_price"),
        "color.hex" => tr("app.data.data_generation.generator.color_hex"),
        "date.datetime_between" => {
            tr("app.data.data_generation.generator.date_datetime_between")
        }
        "date.recent" => tr("app.data.data_generation.generator.date_recent"),
        "date.unix" => tr("app.data.data_generation.generator.date_unix"),
        "lorem.word" => tr("app.data.data_generation.generator.lorem_word"),
        "lorem.sentence" => tr("app.data.data_generation.generator.lorem_sentence"),
        "lorem.paragraph" => tr("app.data.data_generation.generator.lorem_paragraph"),
        "copy_field" => tr("app.data.data_generation.generator.copy_field"),
        "foreign_ref" => tr("app.data.data_generation.generator.foreign_ref"),
        "skip" => tr("app.data.data_generation.generator.skip"),
        "default" => tr("app.data.data_generation.generator.default"),
        _ => SharedString::from(generator.to_string()),
    }
}

impl NavicatMain {
    /// 从数据编辑页打开生成弹窗（工具栏入口）。
    fn open_data_generation_wizard_for_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let object = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some(editor.object.clone()),
                _ => None,
            });
        let Some(object) = object else {
            self.show_message(
                tr("app.data.data_generation.open_failed").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        };
        self.open_data_generation_wizard(object, window, cx);
    }

    /// 打开生成弹窗：先展示弹框，再在后台读取列结构、物理外键与 ER 逻辑关系。
    fn open_data_generation_wizard(
        &mut self,
        object: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_data_generation.as_ref().is_some_and(|form| form.running) {
            self.show_message(
                tr("app.data.data_generation.running").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let kind = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == object.connection_id)
            .map(|connection| connection.config.kind)
            .unwrap_or(DatabaseKind::Postgres);
        let connection_name = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == object.connection_id)
            .map(|connection| connection.config.name.clone())
            .unwrap_or_default();
        let mut form = TableDataGenerationForm::new(object.clone(), kind, window, cx);
        form.connection_name = connection_name;
        self.pending_data_generation = Some(form);

        let mut controller = self.controller.clone();
        let scope_key = fluxdb_core::er_relationship_scope_key(
            object.connection_id,
            object.database.as_deref().unwrap_or_default(),
            object.schema.as_deref(),
        );
        let task_object = object.clone();
        let task = cx.spawn(async move |view, cx| {
            // ER 关系与表结构都在后台读取；ER 是本地模型，与表结构读取无依赖。
            let (source_event, relationships) = cx
                .background_spawn(async move {
                    let er = controller.dispatch(AppCommand::LoadErRelationships {
                        scope_key: scope_key.clone(),
                    });
                    let source = controller.dispatch(AppCommand::LoadGenerationSource {
                        object: task_object.clone(),
                    });
                    (source, er)
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._data_generation_load_task = None;
                    let Some(form) = this.pending_data_generation.as_mut() else {
                        return;
                    };
                    if form.object != object {
                        return;
                    }
                    form.loading = false;
                    let relationships = match relationships {
                        AppEvent::ErRelationshipsLoaded {
                            scope_key: _,
                            relationships,
                        } => relationships,
                        _ => Vec::new(),
                    };
                    match source_event {
                        AppEvent::GenerationSourceLoaded {
                            result: Ok(source), ..
                        } => {
                            let foreign_keys = merge_generation_foreign_keys(
                                source.foreign_keys,
                                &object,
                                &relationships,
                            );
                            form.source_columns = source.columns.clone();
                            form.foreign_keys = foreign_keys.clone();
                            form.columns = source
                                .columns
                                .iter()
                                .map(|column| {
                                    generation_column_form(column, &object, &foreign_keys)
                                })
                                .collect();
                            if form.columns.is_empty() {
                                form.error = Some(
                                    tr("app.data.data_generation.no_columns").to_string(),
                                );
                            }
                            // 预览要用父表真实取值，否则外键列会显示回退随机整数。
                            this.load_data_generation_reference_pools(cx);
                        }
                        AppEvent::GenerationSourceLoaded {
                            result: Err(error), ..
                        } => form.error = Some(error.message),
                        AppEvent::Failed(error) => form.error = Some(error.message),
                        _ => {
                            form.error =
                                Some(tr("app.data.data_generation.load_failed").to_string())
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_generation_load_task = Some(task);
        cx.notify();
    }

    /// 关闭生成弹窗；运行中只请求停止，保留弹窗到任务收尾。
    fn cancel_data_generation_wizard(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_data_generation.as_mut()
            && form.running
        {
            if let Some(cancel) = &form.cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some(id) = form.task_center_id {
                self.task_center.mark_cancel_requested(id);
            }
            self.show_message(
                tr("app.task_center.cancel_requested"),
                AppMessageKind::Warning,
                cx,
            );
            cx.notify();
            return;
        }
        self.pending_data_generation = None;
        self.data_generation_picker = None;
        self._data_generation_task = None;
        self._data_generation_load_task = None;
        cx.notify();
    }

    /// 清除已完成任务：关闭弹框；运行中不允许清除，避免丢失进度。
    fn clear_data_generation_task(&mut self, cx: &mut Context<Self>) {
        if self
            .pending_data_generation
            .as_ref()
            .is_some_and(|form| form.running)
        {
            self.show_message(
                tr("app.data.data_import.running_no_clear"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.pending_data_generation = None;
        self._data_generation_task = None;
        cx.notify();
    }

    fn set_data_generation_mode(
        &mut self,
        mode: fluxdb_core::TableImportMode,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_generation.as_mut() {
            form.mode = mode;
        }
        cx.notify();
    }

    fn set_data_generation_generator(
        &mut self,
        column: &str,
        generator: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_generation.as_mut()
            && let Some(target) = form
                .columns
                .iter_mut()
                .find(|candidate| candidate.name == column)
        {
            // 保留外键目标：切走再切回 foreign_ref 时仍能引用原来的父表。
            target.generator = generator;
        }
        cx.notify();
    }

    /// 打开生成器下拉；搜索框随下拉创建并聚焦，关闭即丢弃。
    fn open_data_generation_picker(
        &mut self,
        column: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = tr("app.data.data_generation.generator_search").to_string();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        search.update(cx, |input, cx| input.focus(window, cx));
        // 搜索词变化时重绘弹窗，列表按关键字过滤。
        cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        // 优先贴着按钮左边缘展开；首次绘制前回退到当前鼠标位置。
        let anchor = self
            .pending_data_generation
            .as_ref()
            .and_then(|form| form.generator_anchors.borrow().get(column).copied())
            .unwrap_or_else(|| window.mouse_position());
        self.data_generation_picker = Some(DataGenerationPicker {
            column: column.to_string(),
            search,
            anchor,
            collapsed: Vec::new(),
        });
        cx.notify();
    }

    /// 折叠/展开下拉里的生成器分组。
    fn toggle_data_generation_group(
        &mut self,
        family: fluxdb_core::GeneratorFamily,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.data_generation_picker.as_mut() else {
            return;
        };
        if let Some(index) = picker
            .collapsed
            .iter()
            .position(|collapsed| *collapsed == family)
        {
            picker.collapsed.remove(index);
        } else {
            picker.collapsed.push(family);
        }
        cx.notify();
    }

    fn close_data_generation_picker(&mut self, cx: &mut Context<Self>) {
        self.data_generation_picker = None;
        cx.notify();
    }

    fn choose_data_generation_generator(
        &mut self,
        generator: String,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.data_generation_picker.take() else {
            return;
        };
        self.set_data_generation_generator(&picker.column, generator, cx);
        cx.notify();
    }

    /// 预取外键取值池，供数据预览使用；真实生成前应用层还会再读一次最新值。
    fn load_data_generation_reference_pools(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_generation.as_ref() else {
            return;
        };
        let references = form
            .columns
            .iter()
            .filter(|column| column.generator == "foreign_ref")
            .filter_map(|column| {
                column
                    .reference
                    .clone()
                    .map(|reference| (column.name.clone(), reference))
            })
            .collect::<Vec<_>>();
        if references.is_empty() {
            return;
        }
        let connection_id = form.object.connection_id;
        let object = form.object.clone();
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::LoadGenerationReferencePools {
                        connection_id,
                        references,
                    })
                })
                .await;
            let AppEvent::GenerationReferencePoolsLoaded { pools, .. } = event else {
                return;
            };
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let Some(form) = this.pending_data_generation.as_mut() else {
                        return;
                    };
                    if form.object != object {
                        return;
                    }
                    for (column, pool) in pools {
                        let Ok(pool) = pool else {
                            continue;
                        };
                        if let Some(target) = form
                            .columns
                            .iter_mut()
                            .find(|candidate| candidate.name == column)
                        {
                            target.reference_pool = pool;
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_generation_pool_task = Some(task);
    }

    fn set_data_generation_locale(&mut self, locale: String, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_data_generation.as_mut() {
            form.locale = locale;
        }
        cx.notify();
    }

    fn set_data_generation_conflict(
        &mut self,
        conflict: fluxdb_core::UniqueConflict,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_generation.as_mut() {
            form.unique_conflict = conflict;
        }
        cx.notify();
    }

    fn set_data_generation_unique(
        &mut self,
        column: &str,
        unique: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_generation.as_mut()
            && let Some(target) = form
                .columns
                .iter_mut()
                .find(|candidate| candidate.name == column)
            && !target.unique_locked
        {
            target.unique = unique;
            // 唯一开关变化后旧值可能已占用容量，清空参数框缓存让输入回到默认值。
        }
        cx.notify();
    }

    fn set_data_generation_pick(&mut self, column: &str, pick: String, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_data_generation.as_mut()
            && let Some(target) = form
                .columns
                .iter_mut()
                .find(|candidate| candidate.name == column)
        {
            target.pick = pick;
        }
        cx.notify();
    }

    fn set_data_generation_preview_tab(
        &mut self,
        tab: GenerationPreviewTab,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_generation.as_mut() {
            form.preview_tab = tab;
        }
        cx.notify();
    }

    /// 按当前表结构重新推断生成器，丢弃用户改动。
    fn reinfer_data_generation_columns(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_data_generation.as_mut() {
            let foreign_keys = form.foreign_keys.clone();
            let object = form.object.clone();
            let pools = form
                .columns
                .iter()
                .map(|column| (column.name.clone(), column.reference_pool.clone()))
                .collect::<BTreeMap<_, _>>();
            form.columns = form
                .source_columns
                .iter()
                .map(|column| generation_column_form(column, &object, &foreign_keys))
                .collect();
            // 重新推断不重新读父表：列名不变时沿用已取到的取值池。
            for column in form.columns.iter_mut() {
                if let Some(pool) = pools.get(&column.name) {
                    column.reference_pool = pool.clone();
                }
            }
            form.params_inputs.borrow_mut().clear();
            form.null_inputs.borrow_mut().clear();
        }
        cx.notify();
    }

    /// 全部重置：列配置、行数、种子、写入方式与冲突策略都回到初始值。
    fn reset_data_generation_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_data_generation.as_mut() {
            let foreign_keys = form.foreign_keys.clone();
            let object = form.object.clone();
            let pools = form
                .columns
                .iter()
                .map(|column| (column.name.clone(), column.reference_pool.clone()))
                .collect::<BTreeMap<_, _>>();
            form.columns = form
                .source_columns
                .iter()
                .map(|column| generation_column_form(column, &object, &foreign_keys))
                .collect();
            for column in form.columns.iter_mut() {
                if let Some(pool) = pools.get(&column.name) {
                    column.reference_pool = pool.clone();
                }
            }
            form.params_inputs.borrow_mut().clear();
            form.null_inputs.borrow_mut().clear();
            form.mode = fluxdb_core::TableImportMode::Append;
            form.unique_conflict = fluxdb_core::UniqueConflict::SkipRow;
            form.locale = generation_locale();
            form.rows_input
                .update(cx, |input, cx| input.set_value("100", window, cx));
            form.seed_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            form.batch_input
                .update(cx, |input, cx| input.set_value("500", window, cx));
            form.filter_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// 复制生成日志为纯文本，格式与导入日志一致。
    fn copy_data_generation_log(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_generation.as_ref() else {
            return;
        };
        let mut lines = vec![
            tr("app.data.data_import.log_table").replace("%{name}", &form.object.name),
            tr("app.data.data_generation.log_rows")
                .replace("%{count}", &form.generated_rows.to_string()),
        ];
        if let Some(start) = form.started_at {
            let end = form.finished_at.unwrap_or_else(Instant::now);
            lines.push(
                tr("app.data.data_import.log_elapsed").replace(
                    "%{ms}",
                    &end.saturating_duration_since(start).as_millis().to_string(),
                ),
            );
        }
        if let Some(error) = &form.error {
            lines.push(error.clone());
        }
        for (index, entry) in form.logs.iter().enumerate() {
            lines.push(format!(
                "#{} [{}] {} ms {}",
                index + 1,
                entry.stage,
                entry.elapsed_ms,
                entry.message
            ));
        }
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
        self.show_message(
            tr("app.data.data_import.log_copied"),
            AppMessageKind::Success,
            cx,
        );
    }

    /// 按当前弹窗内容构造生成配置；参数取自对应列的结构化输入框。
    fn data_generation_spec(
        &self,
        form: &TableDataGenerationForm,
        rows: u64,
        seed: String,
        cx: &App,
    ) -> fluxdb_core::GenerationSpec {
        generation_spec_from_form(form, rows, seed, cx)
    }

    /// 执行生成并写入；确认步骤由调用方负责。
    fn confirm_data_generation(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_generation.clone() else {
            return;
        };
        if form.running || form.loading || form.columns.is_empty() {
            return;
        }
        let rows_text = generation_input_text(&form.rows_input, cx);
        let Ok(rows) = rows_text.trim().parse::<u64>() else {
            if let Some(current) = self.pending_data_generation.as_mut() {
                current.error =
                    Some(tr("app.data.data_generation.rows_invalid").to_string());
            }
            cx.notify();
            return;
        };
        if rows == 0 || rows > fluxdb_core::DATA_GENERATION_MAX_ROWS {
            if let Some(current) = self.pending_data_generation.as_mut() {
                current.error = Some(
                    tr("app.data.data_generation.rows_range")
                        .replace(
                            "%{max}",
                            &fluxdb_core::DATA_GENERATION_MAX_ROWS.to_string(),
                        ),
                );
            }
            cx.notify();
            return;
        }
        let batch_rows = match generation_batch_rows(&form, cx) {
            Ok(batch_rows) => batch_rows,
            Err(error) => {
                if let Some(current) = self.pending_data_generation.as_mut() {
                    current.error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        let seed = generation_input_text(&form.seed_input, cx)
            .trim()
            .to_string();
        let seed = if seed.is_empty() {
            generation_default_seed()
        } else {
            seed
        };
        let spec = self.data_generation_spec(&form, rows, seed, cx);

        let cancel = Arc::new(AtomicBool::new(false));
        let task_center_id = self.task_center.begin(
            TaskCenterKind::TableGeneration,
            tr("app.task_center.title.table_generation")
                .replace("%{table}", &form.object.name),
            tr("app.data.data_generation.log_rows").replace("%{count}", &rows.to_string()),
            Some(cancel.clone()),
        );
        if let Some(current) = self.pending_data_generation.as_mut() {
            current.running = true;
            current.error = None;
            current.logs.clear();
            current.started_at = Some(Instant::now());
            current.finished_at = None;
            current.generated_rows = 0;
            current.cancel = Some(cancel.clone());
            current.task_center_id = Some(task_center_id);
            current.preview_tab = GenerationPreviewTab::Log;
            current.logs.push(ImportDialogLogEntry {
                stage: tr("app.data.data_import.stage_prepare").to_string(),
                message: tr("app.data.data_generation.log_starting").to_string(),
                success: true,
                elapsed_ms: 0,
            });
        }

        let controller = self.controller.clone();
        let object = form.object.clone();
        let mode = form.mode;
        let cancel_for_status = cancel.clone();
        let task = cx.spawn(async move |view, cx| {
            // 进度在后台线程产生，经通道逐条回到 UI 线程；生成期间界面保持可中止。
            let (progress_tx, progress_rx) = mpsc::channel::<(String, String)>();
            let result = cx
                .background_spawn(async move {
                    controller.dispatch_database_task(
                        AppCommand::GenerateTableData {
                            object: object.clone(),
                            spec,
                            mode,
                            batch_rows,
                        },
                        cancel.as_ref(),
                        &mut |progress| {
                            let _ = progress_tx.send((progress.stage, progress.message));
                        },
                    )
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok((stage, message)) = progress_rx.try_recv() {
                    cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.push_data_generation_log(stage.clone(), message.clone(), true);
                            this.task_center.update_progress(
                                task_center_id,
                                None,
                                Some(stage.clone()),
                                Some(format!("{stage}：{message}")),
                            );
                            cx.notify();
                        });
                    });
                }
                if let Some(event) = result.as_mut().now_or_never() {
                    cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._data_generation_task = None;
                            // 通道收尾：完成前最后一批进度仍需落日志。
                            while let Ok((stage, message)) = progress_rx.try_recv() {
                                this.push_data_generation_log(stage.clone(), message.clone(), true);
                                this.task_center.update_progress(
                                    task_center_id,
                                    None,
                                    Some(stage.clone()),
                                    Some(format!("{stage}：{message}")),
                                );
                            }
                            match event {
                                AppEvent::TableDataGenerated { object, report } => {
                                    let message = tr("app.data.data_generation.succeeded")
                                        .replace("%{rows}", &report.rows.to_string());
                                    if let Some(current) =
                                        this.pending_data_generation.as_mut()
                                    {
                                        current.running = false;
                                        current.finished_at = Some(Instant::now());
                                        current.generated_rows = report.rows;
                                        current.skipped_rows = report.skipped_rows;
                                        current.cancel = None;
                                        current.preview_tab = GenerationPreviewTab::Log;
                                        current.logs.push(ImportDialogLogEntry {
                                            stage: tr("app.data.data_import.stage_done")
                                                .to_string(),
                                            message: message.clone(),
                                            success: true,
                                            elapsed_ms: current
                                                .started_at
                                                .map(|start| {
                                                    start.elapsed().as_millis() as u64
                                                })
                                                .unwrap_or(0),
                                        });
                                        for warning in &report.warnings {
                                            current.logs.push(ImportDialogLogEntry {
                                                stage: tr(
                                                    "app.data.data_import.stage_warning",
                                                )
                                                .to_string(),
                                                message: crate::localize_backend_error_text(
                                                    warning,
                                                ),
                                                success: false,
                                                elapsed_ms: current
                                                    .started_at
                                                    .map(|start| {
                                                        start.elapsed().as_millis() as u64
                                                    })
                                                    .unwrap_or(0),
                                            });
                                        }
                                    }
                                    this.finish_task_center_record(
                                        task_center_id,
                                        TaskCenterStatus::Success,
                                        tr("app.task_center.status.success").to_string(),
                                        None,
                                    );
                                    let mut message = message;
                                    if !report.warnings.is_empty() {
                                        message.push_str(": ");
                                        message.push_str(
                                            &report
                                                .warnings
                                                .iter()
                                                .map(|warning| {
                                                    crate::localize_backend_error_text(warning)
                                                })
                                                .collect::<Vec<_>>()
                                                .join("; "),
                                        );
                                    }
                                    this.show_message(message, AppMessageKind::Success, cx);
                                    this.request_data_editor_refresh_for_object(&object, cx);
                                }
                                event => {
                                    let message = match event {
                                        AppEvent::Failed(error) => error.message,
                                        _ => tr("app.data.data_generation.no_result")
                                            .to_string(),
                                    };
                                    let canceled =
                                        cancel_for_status.load(Ordering::Relaxed);
                                    let status = if canceled {
                                        TaskCenterStatus::Canceled
                                    } else {
                                        TaskCenterStatus::Failed
                                    };
                                    if let Some(current) =
                                        this.pending_data_generation.as_mut()
                                    {
                                        current.running = false;
                                        current.finished_at = Some(Instant::now());
                                        current.cancel = None;
                                        current.preview_tab = GenerationPreviewTab::Log;
                                        current.error =
                                            (!canceled).then_some(message.clone());
                                        current.logs.push(ImportDialogLogEntry {
                                            stage: if canceled {
                                                tr("app.data.data_import.stage_canceled")
                                                    .to_string()
                                            } else {
                                                tr("app.data.data_import.stage_failed")
                                                    .to_string()
                                            },
                                            message: message.clone(),
                                            success: false,
                                            elapsed_ms: current
                                                .started_at
                                                .map(|start| {
                                                    start.elapsed().as_millis() as u64
                                                })
                                                .unwrap_or(0),
                                        });
                                    }
                                    this.finish_task_center_record(
                                        task_center_id,
                                        status,
                                        status.label(false).to_string(),
                                        if canceled { None } else { Some(message) },
                                    );
                                }
                            }
                            cx.notify();
                        });
                    });
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        });
        self._data_generation_task = Some(task);
        cx.notify();
    }

    /// 追加一条生成日志，并统一换算相对开始时间的耗时。
    fn push_data_generation_log(&mut self, stage: String, message: String, success: bool) {
        if let Some(current) = self.pending_data_generation.as_mut() {
            let elapsed_ms = current
                .started_at
                .map(|start| start.elapsed().as_millis() as u64)
                .unwrap_or(0);
            current.logs.push(ImportDialogLogEntry {
                stage,
                message,
                success,
                elapsed_ms,
            });
        }
    }
}

/// 按弹窗当前配置构造生成 spec；预览与真实生成共用，保证两者一致。
fn generation_spec_from_form(
    form: &TableDataGenerationForm,
    rows: u64,
    seed: String,
    cx: &App,
) -> fluxdb_core::GenerationSpec {
    let columns = form
        .columns
        .iter()
        .map(|column| {
            let mut params = BTreeMap::new();
            for field in generation_param_fields(&column.generator) {
                let value =
                    generation_param_value(form, &column.name, &column.generator, field.key, cx);
                let value = value.trim().to_string();
                // 与默认值相同的输入不写参数，交给核心默认值处理。
                if !value.is_empty()
                    && value != generation_param_default(&column.generator, field.key)
                {
                    params.insert(field.key.to_string(), value);
                }
            }
            if column.generator == "foreign_ref" && !column.pick.is_empty() {
                params.insert("pick".into(), column.pick.clone());
            }
            fluxdb_core::GenerationColumnSpec {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
                nullable: column.nullable,
                unique: column.unique,
                generator: column.generator.clone(),
                params,
                null_percent: if column.nullable {
                    generation_null_percent(form, &column.name, cx)
                } else {
                    0
                },
                // 只有当前生成器确实是外键引用时才带上目标。
                reference: (column.generator == "foreign_ref")
                    .then(|| column.reference.clone())
                    .flatten(),
                reference_pool: column.reference_pool.clone(),
            }
        })
        .collect();
    fluxdb_core::GenerationSpec {
        rows,
        seed,
        locale: form.locale.clone(),
        unique_conflict: form.unique_conflict,
        columns,
    }
}

/// 语义生成器的数据集语言跟随界面语言：中文界面用中文姓名/地址，其余用英文。
fn generation_locale() -> String {
    if fluxdb_i18n::active_locale()
        .to_ascii_lowercase()
        .starts_with("zh")
    {
        "zh_cn".to_string()
    } else {
        "en".to_string()
    }
}

/// 当前时间毫秒串作为默认随机种子；同一种子会生成相同数据。
fn generation_default_seed() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "fluxdb".to_string())
}

/// 合并物理外键与用户自定义 ER 逻辑关系。
///
/// 很多库没有物理外键，但 ER 设计里维护的关系同样应该能取到父表已有值。
/// 物理外键排在前面：同一列两处都有描述时，`generation_column_form` 命中物理约束，
/// 它才是数据库真实会校验的目标。
fn merge_generation_foreign_keys(
    physical: Vec<ForeignKeyInfo>,
    object: &ObjectPath,
    relationships: &[fluxdb_core::ErRelationship],
) -> Vec<ForeignKeyInfo> {
    let mut foreign_keys = physical;
    foreign_keys.extend(logical_relationship_foreign_keys(object, relationships));
    foreign_keys
}

fn generation_column_form(
    column: &fluxdb_app::DataGenerationSourceColumn,
    object: &ObjectPath,
    foreign_keys: &[ForeignKeyInfo],
) -> GenerationColumnForm {
    // 主键列不引用父表：ER 逻辑关系是双向的，反向投影会把「本表主键被子表引用」
    // 误当成外键，而且主键必须唯一，引用池的有限取值会直接撞唯一约束。
    // 复合外键也不能只取其中一列的值，否则写入必然违反约束；这类列按普通列生成。
    let foreign_key = (!column.primary_key).then(|| foreign_keys.iter().find(|foreign_key| {
        foreign_key.columns.len() <= 1
            && foreign_key.column.eq_ignore_ascii_case(&column.name)
            && !foreign_key.ref_table.is_empty()
            && !foreign_key.ref_column.is_empty()
    }))
    .flatten();
    let reference = foreign_key.map(|foreign_key| fluxdb_core::GenerationReference {
        database: object.database.clone(),
        schema: foreign_key.ref_schema.clone(),
        table: foreign_key.ref_table.clone(),
        column: foreign_key.ref_column.clone(),
        label: format!("{}.{}", foreign_key.ref_table, foreign_key.ref_column),
    });
    GenerationColumnForm {
        name: column.name.clone(),
        type_name: column.type_name.clone(),
        nullable: column.nullable,
        primary_key: column.primary_key,
        // 表结构已声明的唯一约束不可取消，只允许在约束之外额外开启。
        unique_locked: column.unique,
        unique: column.unique,
        generator: if reference.is_some() {
            "foreign_ref".to_string()
        } else {
            fluxdb_core::infer_generator(&column.name, &column.type_name).to_string()
        },
        reference_label: reference.as_ref().map(|reference| reference.label.clone()),
        reference,
        pick: String::new(),
        reference_pool: Vec::new(),
    }
}

/// 用真实生成引擎产出预览行；与写入使用同一份确定性规则，前几行即实际数据。
fn data_generation_preview(
    form: &TableDataGenerationForm,
    cx: &App,
) -> Vec<Vec<String>> {
    if form.columns.is_empty() {
        return Vec::new();
    }
    let seed = generation_input_text(&form.seed_input, cx).trim().to_string();
    let seed = if seed.is_empty() {
        "fluxdb".to_string()
    } else {
        seed
    };
    // 预览与真实生成共用同一份 spec 构造逻辑，只改行数；外键池为空，
    // 预览里外键列显示随机整数，真实生成会读取父表值。
    let spec = generation_spec_from_form(form, 8, seed, cx);
    let mut rows = Vec::new();
    let _ = fluxdb_core::generate_table_rows(
        &spec,
        &AtomicBool::new(false),
        &mut |_| {},
        &mut |row| {
            rows.push(row.into_iter().map(generation_value_label).collect());
            Ok(())
        },
    );
    rows
}

fn generation_value_label(value: fluxdb_core::GenerationValue) -> String {
    match value {
        fluxdb_core::GenerationValue::Null => tr("app.data.data_generation.null").to_string(),
        fluxdb_core::GenerationValue::Bool(value) => value.to_string(),
        fluxdb_core::GenerationValue::Int(value) => value.to_string(),
        fluxdb_core::GenerationValue::Float(value) => value.to_string(),
        fluxdb_core::GenerationValue::Decimal(value)
        | fluxdb_core::GenerationValue::Text(value) => value,
    }
}

/// 生成弹窗：顶部配置条、左侧列配置、右侧「数据预览 / 消息日志」、底部写入统计。
fn table_data_generation_modal(
    form: TableDataGenerationForm,
    picker: Option<DataGenerationPicker>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_generate = !form.loading && !form.running && !form.columns.is_empty();

    let panel = div()
        .w(px(1180.))
        .max_w(px(1180.))
        .h(px(760.))
        .max_h(px(760.))
        .rounded(px(12.))
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .shadow(vec![box_shadow(
            px(0.),
            px(28.),
            px(70.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.55 } else { 0.2 }),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .text_size(ui_text_size(12.))
        .text_color(colors.text)
        .key_context("TableDataGenerationModal")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(data_generation_header(&form, colors, cx))
        .child(data_generation_config_bar(&form, colors, window, cx))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .px_3()
                .pt_2()
                .pb_3()
                .flex()
                .gap_3()
                .child(data_generation_columns_panel(&form, colors, window, cx))
                .child(data_generation_preview_panel(&form, colors, cx)),
        )
        .child(data_generation_footer(&form, can_generate, colors, cx));

    let modal = div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.6)
        } else {
            opaque_grey(0.75, 0.3)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.cancel_data_generation_wizard(cx);
            cx.stop_propagation();
        }))
        .child(panel);

    div()
        .absolute()
        .inset_0()
        .child(modal)
        .when_some(picker, |this, picker| {
            this.child(data_generation_generator_picker(
                &form, picker, colors, window, cx,
            ))
        })
}

fn data_generation_log_view(form: &TableDataGenerationForm) -> ImportDialogLogView<'_> {
    ImportDialogLogView {
        running: form.running,
        finished: form.finished_at.is_some(),
        imported_label: tr("app.data.data_generation.log_rows")
            .replace("%{count}", &form.generated_rows.to_string()),
        started_at: form.started_at,
        finished_at: form.finished_at,
        logs: &form.logs,
    }
}

/// 下拉内的分组顺序，与 ui 稿一致；目录数组本身按实现分类排列，分组按列表现。
const GENERATOR_FAMILY_ORDER: &[fluxdb_core::GeneratorFamily] = &[
    fluxdb_core::GeneratorFamily::Numeric,
    fluxdb_core::GeneratorFamily::Text,
    fluxdb_core::GeneratorFamily::Person,
    fluxdb_core::GeneratorFamily::Contact,
    fluxdb_core::GeneratorFamily::Address,
    fluxdb_core::GeneratorFamily::Company,
    fluxdb_core::GeneratorFamily::Commerce,
    fluxdb_core::GeneratorFamily::Date,
    fluxdb_core::GeneratorFamily::Relation,
    fluxdb_core::GeneratorFamily::Advanced,
];

/// 分组标题文案。
fn generation_family_label(family: fluxdb_core::GeneratorFamily) -> SharedString {
    use fluxdb_core::GeneratorFamily;
    let key = match family {
        GeneratorFamily::Numeric => "app.data.data_generation.group_numeric",
        GeneratorFamily::Text => "app.data.data_generation.group_text",
        GeneratorFamily::Person => "app.data.data_generation.group_person",
        GeneratorFamily::Contact => "app.data.data_generation.group_contact",
        GeneratorFamily::Address => "app.data.data_generation.group_address",
        GeneratorFamily::Company => "app.data.data_generation.group_company",
        GeneratorFamily::Commerce => "app.data.data_generation.group_commerce",
        GeneratorFamily::Date => "app.data.data_generation.group_date",
        GeneratorFamily::Relation => "app.data.data_generation.group_relation",
        GeneratorFamily::Advanced => "app.data.data_generation.group_advanced",
    };
    tr(key)
}

/// 右侧类型徽章文案。
fn generation_output_label(output: fluxdb_core::GeneratorOutput) -> SharedString {
    use fluxdb_core::GeneratorOutput;
    let key = match output {
        GeneratorOutput::Any => "app.data.data_generation.output_any",
        GeneratorOutput::Int => "app.data.data_generation.output_int",
        GeneratorOutput::Float => "app.data.data_generation.output_float",
        GeneratorOutput::Text => "app.data.data_generation.output_text",
        GeneratorOutput::Bool => "app.data.data_generation.output_bool",
        GeneratorOutput::Date => "app.data.data_generation.output_date",
    };
    tr(key)
}

/// 生成器下拉：锚在触发按钮左下角，分组 + 类型徽章 + 搜索过滤 + 列表滚动。
///
/// 与列类型不兼容的项灰显且不可点，底部提示原因；用 `anchored` 定位，
/// 窗口底部空间不足时自动翻到按钮上方。
fn data_generation_generator_picker(
    form: &TableDataGenerationForm,
    picker: DataGenerationPicker,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = generation_input_text(&picker.search, cx)
        .trim()
        .to_lowercase();
    let column = form
        .columns
        .iter()
        .find(|column| column.name == picker.column);
    let current = column
        .map(|column| column.generator.clone())
        .unwrap_or_default();
    let column_type = column
        .map(|column| column.type_name.clone())
        .unwrap_or_default();
    let primary = ComponentTheme::global(cx).primary;

    let mut list = div().flex().flex_col().pb_2();
    let mut shown = 0usize;
    // 逐项累计内容高度，用于决定下拉朝下还是朝上展开。
    let mut content_height = 8f32;
    for family in GENERATOR_FAMILY_ORDER {
        let items = fluxdb_core::DATA_GENERATORS
            .iter()
            .filter(|generator| generator.family == *family)
            .filter(|generator| {
                query.is_empty()
                    || generator.id.to_lowercase().contains(&query)
                    || generation_generator_label(generator.id)
                        .to_string()
                        .to_lowercase()
                        .contains(&query)
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            continue;
        }
        // 搜索时忽略折叠，避免命中项被收起的组藏起来。
        let family_value = *family;
        let collapsed = query.is_empty() && picker.collapsed.contains(&family_value);
        list = list.child(
            div()
                .px_3()
                .pt_2()
                .pb_1()
                .flex()
                .items_center()
                .gap_1()
                .hover(|this| this.bg(colors.hover))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    this.toggle_data_generation_group(family_value, cx);
                    cx.stop_propagation();
                }))
                .child(app_icon(
                    if collapsed {
                        AppIcon::ChevronRight
                    } else {
                        AppIcon::ChevronDown
                    },
                    12.,
                    colors.muted,
                ))
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(generation_family_label(family_value)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(9.5))
                        .text_color(colors.muted)
                        .child(items.len().to_string()),
                ),
        );
        content_height += 26.;
        if collapsed {
            continue;
        }
        for generator in items {
            shown += 1;
            content_height += 28.;
            let id = generator.id.to_string();
            let checked = id == current;
            let compatible = fluxdb_core::generator_compatible_with_type(&id, &column_type);
            let mut row = div()
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .text_size(ui_text_size(12.))
                .when(checked, |this| {
                    this.bg(hsla(primary.h, primary.s, primary.l, 0.12))
                        .text_color(primary)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .when(!checked, |this| this.text_color(colors.text))
                .when(!compatible, |this| this.opacity(0.34))
                .when(compatible && !checked, |this| {
                    this.hover(|this| this.bg(colors.hover)).cursor_pointer()
                })
                .child(generation_generator_label(generator.id))
                .child(div().flex_1())
                .child(
                    div()
                        .px_1()
                        .rounded(px(3.))
                        .border_1()
                        .text_size(ui_text_size(9.5))
                        .when(checked, |this| {
                            this.text_color(primary)
                                .border_color(hsla(primary.h, primary.s, primary.l, 0.45))
                        })
                        .when(!checked, |this| {
                            this.text_color(colors.muted).border_color(colors.border_soft)
                        })
                        .child(generation_output_label(fluxdb_core::generator_output(
                            generator.id,
                        ))),
                );
            if compatible {
                row = row.on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    this.choose_data_generation_generator(id.clone(), cx);
                    cx.stop_propagation();
                }));
            }
            list = list.child(row);
        }
    }
    if shown == 0 {
        content_height += 56.;
        list = list.child(
            div()
                .p_4()
                .text_size(ui_text_size(11.5))
                .text_color(colors.muted)
                .child(tr("app.data.data_generation.generator_empty")),
        );
    }
    // 面板给固定高度：滚轮的命中与滚动都依赖滚动容器有确定高度（与筛选浮层同一套写法），
    // 只给 max_h 时内容能画出滚动条却滚不动。
    let panel_height = (content_height + 80.).min(430.);

    let panel = div()
        .w(px(390.))
        .h(px(panel_height))
        .rounded(px(8.))
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .shadow(vec![box_shadow(
            px(0.),
            px(14.),
            px(36.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.5 } else { 0.16 }),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .text_size(ui_text_size(12.))
        // 下拉内的点击不能穿透到弹窗背景，否则会被当成「点击空白关闭弹窗」。
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex_none()
                .px_2()
                .py_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .child(
                    div()
                        .h(px(30.))
                        .px_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(app_icon(AppIcon::Search, 13., colors.muted))
                        .child(Input::new(&picker.search).appearance(false).focus_bordered(false)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(list),
        )
        .child(
            div()
                .flex_none()
                .h(px(30.))
                .px_3()
                .border_t_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .gap_1()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(
                    tr("app.data.data_generation.generator_count").replace(
                        "%{count}",
                        &fluxdb_core::DATA_GENERATORS.len().to_string(),
                    ),
                )
                .child(div().flex_1())
                .child(tr("app.data.data_generation.generator_incompatible_hint")),
        );

    // 位置自己算：`anchored` 的元素偏移不会同步移动 hitbox，
    // 命中测试仍落在下层字段列表，滚轮就滚不动下拉（滚动条却正常绘制）。
    let viewport = window.viewport_size();
    let left = f32::from(picker.anchor.x)
        .min(f32::from(viewport.width) - 398.)
        .max(8.);
    let below = f32::from(picker.anchor.y) + 6.;
    let top = if below + panel_height + 8. > f32::from(viewport.height) {
        (f32::from(picker.anchor.y) - panel_height - 6.).max(8.)
    } else {
        below
    };

    div()
        .absolute()
        .inset_0()
        // 遮罩挡住下层命中测试：否则滚轮同时命中后面的字段列表，两边一起滚。
        // 不能再加 `on_scroll_wheel(stop_propagation)`：遮罩的监听会先于列表执行，
        // 把列表自己的滚动一起停掉。
        .occlude()
        // 透明遮罩只负责「点外面关闭」，不压暗弹窗。
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_data_generation_picker(cx);
            cx.stop_propagation();
        }))
        .child(panel.absolute().left(px(left)).top(px(top)))
}

/// 头部：标题、连接/库/表摘要、结构徽章与关闭按钮。
fn data_generation_header(
    form: &TableDataGenerationForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let path = [
        form.connection_name.clone(),
        form.object.database.clone().unwrap_or_default(),
        form.object.schema.clone().unwrap_or_default(),
        form.object.name.clone(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" / ");

    div()
        .h(px(58.))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .gap_2p5()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .min_w(px(0.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(app_icon(AppIcon::Wand, 15., colors.text))
                        .child(
                            div()
                                .text_size(ui_text_size(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.data.data_generation.title")),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(path),
                ),
        )
        .child(generation_meta_chips(form, colors))
        .child(div().flex_1())
        .child(
            Button::new("data-generation-close")
                .icon(gpui_component::Icon::default().path(app_icon_path(AppIcon::Close)))
                .tooltip(tr("app.common.close"))
                .ghost()
                .compact()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_data_generation_wizard(cx);
                    cx.stop_propagation();
                })),
        )
}

/// 头部结构徽章：可写列 / 唯一 / 外键 / 跳过。
fn generation_meta_chips(form: &TableDataGenerationForm, colors: UiColors) -> Div {
    let writable = form
        .columns
        .iter()
        .filter(|column| column.generator != "skip")
        .count();
    let unique = form.columns.iter().filter(|column| column.unique).count();
    let foreign = form
        .columns
        .iter()
        .filter(|column| column.generator == "foreign_ref")
        .count();
    let skipped = form.columns.len().saturating_sub(writable);
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(generation_badge(
            SharedString::from(
                tr("app.data.data_generation.meta_writable").replace("%{count}", &writable.to_string()),
            ),
            colors.info,
            colors,
        ))
        .child(generation_badge(
            SharedString::from(
                tr("app.data.data_generation.meta_unique").replace("%{count}", &unique.to_string()),
            ),
            colors.warning,
            colors,
        ))
        .child(generation_badge(
            SharedString::from(
                tr("app.data.data_generation.meta_foreign").replace("%{count}", &foreign.to_string()),
            ),
            colors.success,
            colors,
        ))
        .when(skipped > 0, |this| {
            this.child(generation_badge(
                SharedString::from(
                    tr("app.data.data_generation.meta_skipped").replace("%{count}", &skipped.to_string()),
                ),
                colors.danger,
                colors,
            ))
        })
}

/// 配置条：行数、种子、语义语言、写入方式、唯一冲突策略。
fn data_generation_config_bar(
    form: &TableDataGenerationForm,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .px_4()
        .py_2p5()
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .flex()
        .items_end()
        .gap_3p5()
        .child(table_data_import_field(
            tr("app.data.data_generation.rows").as_ref(),
            div()
                .w(px(92.))
                .child(Input::new(&form.rows_input))
                .into_any_element(),
            colors,
        ))
        .child(table_data_import_field(
            tr("app.data.data_generation.seed").as_ref(),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(div().w(px(128.)).child(Input::new(&form.seed_input)))
                .child(
                    Button::new("data-generation-reroll-seed")
                        .icon(gpui_component::Icon::default().path(app_icon_path(AppIcon::Refresh)))
                        .tooltip(tr("app.data.data_generation.reroll_seed"))
                        .outline()
                        .compact()
                        .on_click(cx.listener(|this, _, window, cx| {
                            let seed = generation_default_seed();
                            if let Some(form) = this.pending_data_generation.as_mut() {
                                form.seed_input.update(cx, |input, cx| {
                                    input.set_value(seed, window, cx)
                                });
                            }
                            cx.notify();
                        })),
                )
                .into_any_element(),
            colors,
        ))
        .child(table_data_import_field(
            tr("app.data.data_generation.locale").as_ref(),
            div()
                .child(data_generation_locale_dropdown(form, cx))
                .into_any_element(),
            colors,
        ))
        .child(table_data_import_field(
            tr("app.data.data_generation.write_mode").as_ref(),
            div()
                .child(data_generation_mode_seg(form, colors, cx))
                .into_any_element(),
            colors,
        ))
        .child(table_data_import_field(
            tr("app.data.data_generation.unique_conflict").as_ref(),
            div()
                .child(data_generation_conflict_dropdown(form, cx))
                .into_any_element(),
            colors,
        ))
        .child(table_data_import_field(
            tr("app.data.data_generation.batch").as_ref(),
            div()
                .w(px(92.))
                .child(Input::new(&form.batch_input))
                .into_any_element(),
            colors,
        ))
        .when_some(form.truncate_warning(), |this, warning| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pb_1()
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.warning)
                    .child(app_icon(AppIcon::Info, 12., colors.warning))
                    .child(warning),
            )
        })
        .when(form.loading, |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(app_icon(AppIcon::Loader, 13., colors.info))
                    .child(tr("app.data.data_generation.loading")),
            )
        })
        .when_some(form.error.clone(), |this, error| {
            this.child(table_data_import_notice(error, colors.danger))
        })
}

/// 写入方式分段控件：一个带边框的容器内两段，选中段用主色浅底。
///
/// 与设计稿一致：不要再拆成两个独立按钮，否则视觉上像两个并列操作。
fn data_generation_mode_seg(
    form: &TableDataGenerationForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let primary = ComponentTheme::global(cx).primary;
    let accent_soft = hsla(primary.h, primary.s, primary.l, if colors.is_dark { 0.22 } else { 0.12 });
    let mut seg = div()
        .h(px(32.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden();
    for mode in [
        fluxdb_core::TableImportMode::Append,
        fluxdb_core::TableImportMode::Truncate,
    ] {
        let label = match mode {
            fluxdb_core::TableImportMode::Append => tr("app.data.data_import.mode_append"),
            fluxdb_core::TableImportMode::Truncate => tr("app.data.data_import.mode_truncate"),
        };
        let active = form.mode == mode;
        seg = seg.child(
            div()
                .h_full()
                .px_3()
                .flex()
                .items_center()
                .text_size(ui_text_size(11.5))
                .when(active, |this| {
                    this.bg(accent_soft)
                        .text_color(primary)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .when(!active, |this| {
                    this.text_color(colors.muted).hover(|this| this.bg(colors.hover))
                })
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    this.set_data_generation_mode(mode, cx);
                    cx.stop_propagation();
                }))
                .child(label),
        );
    }
    seg
}

fn data_generation_locale_dropdown(
    form: &TableDataGenerationForm,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let current = form.locale.clone();
    let label = if current.starts_with("zh") {
        tr("app.data.data_generation.locale_zh")
    } else {
        tr("app.data.data_generation.locale_en")
    };
    Button::new("data-generation-locale")
        .label(label)
        .outline()
        .dropdown_caret(true)
        .compact()
        .w(px(128.))
        .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
            let mut menu = menu.min_w(px(160.));
            for (locale, option_label) in [
                ("zh_cn", tr("app.data.data_generation.locale_zh")),
                ("en", tr("app.data.data_generation.locale_en")),
            ] {
                let view = weak.clone();
                let checked = current == locale;
                let locale = locale.to_string();
                menu = menu.item(
                    PopupMenuItem::new(option_label)
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            if let Some(view) = view.upgrade() {
                                let locale = locale.clone();
                                view.update(cx, |this, cx| {
                                    this.set_data_generation_locale(locale, cx);
                                });
                            }
                        }),
                );
            }
            menu
        })
}

fn data_generation_conflict_dropdown(
    form: &TableDataGenerationForm,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let current = form.unique_conflict;
    let label = match current {
        fluxdb_core::UniqueConflict::SkipRow => {
            tr("app.data.data_generation.conflict_skip")
        }
        fluxdb_core::UniqueConflict::Abort => tr("app.data.data_generation.conflict_abort"),
    };
    Button::new("data-generation-conflict")
        .label(label)
        .outline()
        .dropdown_caret(true)
        .compact()
        .w(px(112.))
        .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
            let mut menu = menu.min_w(px(180.));
            for (option, option_label) in [
                (
                    fluxdb_core::UniqueConflict::SkipRow,
                    tr("app.data.data_generation.conflict_skip"),
                ),
                (
                    fluxdb_core::UniqueConflict::Abort,
                    tr("app.data.data_generation.conflict_abort"),
                ),
            ] {
                let view = weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(option_label)
                        .checked(option == current)
                        .on_click(move |_, _, cx| {
                            if let Some(view) = view.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.set_data_generation_conflict(option, cx);
                                });
                            }
                        }),
                );
            }
            menu
        })
}

/// 左侧列配置：筛选、重新推断、全部重置与逐列规则。
fn data_generation_columns_panel(
    form: &TableDataGenerationForm,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filter = generation_input_text(&form.filter_input, cx)
        .trim()
        .to_lowercase();
    let skipped = form
        .columns
        .iter()
        .filter(|column| column.generator == "skip")
        .count();
    let visible = form
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| {
            filter.is_empty()
                || column.name.to_lowercase().contains(&filter)
                || generation_generator_label(&column.generator)
                    .to_string()
                    .to_lowercase()
                    .contains(&filter)
        })
        .collect::<Vec<_>>();

    let mut header_row = div()
        .flex()
        .flex_none()
        .h(px(26.))
        .px_2()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(10.))
        .text_color(colors.muted);
    for (label, width) in [
        (tr("app.data.data_generation.column_name"), Some(176.)),
        (tr("app.data.data_generation.column_generator"), Some(190.)),
        (tr("app.data.data_generation.column_params"), None),
        (tr("app.data.data_generation.column_null"), Some(64.)),
        (tr("app.data.data_generation.column_unique"), Some(54.)),
    ] {
        let mut cell = div().flex().items_center().child(label);
        if let Some(width) = width {
            cell = cell.w(px(width)).flex_none();
        } else {
            cell = cell.flex_1();
        }
        header_row = header_row.child(cell);
    }

    let mut list = div().flex().flex_col();
    if visible.is_empty() {
        list = list.child(
            div()
                .p_4()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.data.data_generation.columns_empty")),
        );
    }
    for (index, column) in visible {
        list = list.child(data_generation_column_row(
            form, column, index, colors, window, cx,
        ));
    }

    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .rounded(px(10.))
        .border_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .h(px(38.))
                .px_3()
                .flex()
                .items_center()
                .gap_2p5()
                .border_b_1()
                .border_color(colors.border_soft)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.data.data_generation.columns_title")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(
                                    tr("app.data.data_generation.columns_count")
                                        .replace("%{count}", &form.columns.len().to_string())
                                        .replace("%{skipped}", &skipped.to_string()),
                                ),
                        ),
                )
                .child(
                    div()
                        .w(px(150.))
                        .child(Input::new(&form.filter_input).small()),
                )
                .child(div().flex_1())
                .child(
                    Button::new("data-generation-reinfer")
                        .label(tr("app.data.data_generation.auto_infer"))
                        .icon(gpui_component::Icon::default().path(app_icon_path(AppIcon::Wand)))
                        .outline()
                        .compact()
                        .disabled(form.running)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reinfer_data_generation_columns(cx);
                            cx.stop_propagation();
                        })),
                )
                .child(
                    Button::new("data-generation-reset")
                        .label(tr("app.data.data_generation.reset_all"))
                        .ghost()
                        .compact()
                        .disabled(form.running)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.reset_data_generation_settings(window, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        .child(header_row)
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(list),
        )
}

fn data_generation_column_row(
    form: &TableDataGenerationForm,
    column: &GenerationColumnForm,
    index: usize,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut badges = div().flex().items_center().gap_1().flex_wrap();
    if column.primary_key {
        badges = badges.child(generation_badge(
            tr("app.data.data_generation.badge_primary_short"),
            colors.info,
            colors,
        ));
    }
    if column.unique_locked && !column.primary_key {
        badges = badges.child(generation_badge(
            tr("app.data.data_generation.badge_unique_short"),
            colors.warning,
            colors,
        ));
    }
    if !column.nullable && !column.primary_key {
        badges = badges.child(generation_badge(
            tr("app.data.data_generation.badge_not_null"),
            colors.muted,
            colors,
        ));
    }
    if column.generator == "foreign_ref"
        && let Some(reference) = &column.reference_label
    {
        // 「FK → 表.列」可能很长：徽章内中间省略保住表名头与列名尾，完整目标放悬停提示。
        let full_label = SharedString::from(format!(
            "{} {reference}",
            tr("app.data.data_generation.badge_foreign_short")
        ));
        let short_label = SharedString::from(shorten_generation_reference(&full_label, 24));
        let tooltip_label = full_label.clone();
        badges = badges.child(
            generation_badge(short_label, colors.success, colors)
                .id(SharedString::from(format!(
                    "data-generation-fk-badge:{}",
                    column.name
                )))
                .tooltip(move |window, cx| {
                    Tooltip::new(tooltip_label.clone()).build(window, cx)
                }),
        );
    }

    let column_name = column.name.clone();
    let generator_column_name = column_name.clone();
    let unique_weak = cx.entity().downgrade();
    let unique_column = column_name.clone();
    let locked = column.unique_locked;

    let mut params = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap_1p5()
        .flex_wrap()
        .text_size(ui_text_size(10.5))
        .text_color(colors.muted);
    if column.generator == "foreign_ref" {
        // 引用目标已经在左侧 FK 徽章里；参数区只留取值方式，避免窄行里换行撑高。
        params = params.child(data_generation_pick_dropdown(form, column, cx));
    } else {
        let fields = generation_param_fields(&column.generator);
        if fields.is_empty() {
            params = params.child(
                div()
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.muted)
                    .child(tr("app.data.data_generation.params_none")),
            );
        }
        for field in fields {
            params = params.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(10.5))
                            .text_color(colors.muted)
                            .child(tr(field.label)),
                    )
                    .child(
                        div()
                            .w(px(field.width))
                            .child(
                                Input::new(&generation_param_input(
                                    form,
                                    &column.name,
                                    &column.generator,
                                    field.key,
                                    window,
                                    cx,
                                ))
                                .small(),
                            ),
                    ),
            );
        }
    }

    let null_cell = if column.nullable {
        div()
            .w(px(64.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .w(px(44.))
                    .child(Input::new(&generation_null_input(
                        form,
                        &column.name,
                        window,
                        cx,
                    ))
                    .small()),
            )
            .child(
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child("%"),
            )
    } else {
        div()
            .w(px(64.))
            .flex_none()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child("—")
    };

    div()
        .min_h(px(46.))
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(colors.border_soft)
        .when(index % 2 == 1, |this| this.bg(colors.panel_alt))
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(220.))
                .flex_none()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .truncate()
                        .text_size(ui_text_size(12.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(column_name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(if column.type_name.is_empty() {
                            "—".to_string()
                        } else {
                            column.type_name.clone()
                        }),
                )
                .child(badges),
        )
        .child({
            let anchors = form.generator_anchors.clone();
            let anchor_column = column_name.clone();
            div()
                // 绘制时记录按钮左下角（窗口坐标），下拉据此贴着按钮展开。
                .on_children_prepainted(move |bounds, _, _| {
                    let Some(bounds) = bounds.first() else {
                        return;
                    };
                    anchors.borrow_mut().insert(
                        anchor_column.clone(),
                        point(bounds.origin.x, bounds.origin.y + bounds.size.height),
                    );
                })
                .child(
                    Button::new(SharedString::from(format!(
                        "data-generation-generator:{column_name}"
                    )))
                    .label(generation_generator_label(&column.generator))
                    .outline()
                    .dropdown_caret(true)
                    .compact()
                    .h(px(26.))
                    .w(px(186.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_data_generation_picker(&generator_column_name, window, cx);
                        cx.stop_propagation();
                    })),
                )
        })
        .child(params)
        .child(null_cell)
        .child(
            div().w(px(54.)).flex_none().flex().justify_center().child(
                Switch::new(SharedString::from(format!(
                    "data-generation-unique:{column_name}"
                )))
                .checked(column.unique)
                .disabled(locked || form.running)
                .on_click(move |checked, _, cx| {
                    if let Some(view) = unique_weak.upgrade() {
                        let checked = *checked;
                        let column = unique_column.clone();
                        view.update(cx, |this, cx| {
                            this.set_data_generation_unique(&column, checked, cx);
                        });
                    }
                }),
            ),
        )
}

/// 外键取值方式：随机或顺序。
fn data_generation_pick_dropdown(
    form: &TableDataGenerationForm,
    column: &GenerationColumnForm,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let column_name = column.name.clone();
    let current = column.pick.clone();
    let label = if current == "sequence" {
        tr("app.data.data_generation.pick_sequence")
    } else {
        tr("app.data.data_generation.pick_random")
    };
    Button::new(SharedString::from(format!(
        "data-generation-pick:{}",
        column.name
    )))
    .label(label)
    .outline()
    .dropdown_caret(true)
    .compact()
    // 与左侧生成器下拉统一高度：弹窗行内按钮都按 26px，避免两枚下拉高低不齐。
    .h(px(26.))
    .w(px(88.))
    .disabled(form.running)
    .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
        let mut menu = menu.min_w(px(140.));
        for (pick, option_label) in [
            ("", tr("app.data.data_generation.pick_random")),
            ("sequence", tr("app.data.data_generation.pick_sequence")),
        ] {
            let view = weak.clone();
            let column_name = column_name.clone();
            let pick = pick.to_string();
            let checked = current == pick;
            menu = menu.item(
                PopupMenuItem::new(option_label)
                    .checked(checked)
                    .on_click(move |_, _, cx| {
                        if let Some(view) = view.upgrade() {
                            let pick = pick.clone();
                            let column_name = column_name.clone();
                            view.update(cx, |this, cx| {
                                this.set_data_generation_pick(&column_name, pick, cx);
                            });
                        }
                    }),
            );
        }
        menu
    })
}

/// 外键徽章的中间省略：保留表名开头和列名结尾，中间用 `…`。
///
/// 按字符数而不是像素截断：GPUI 里量文本要额外测量，而这里只需要保证
/// 徽章不被撑长；真实的换行/省略仍由 `max_w + truncate` 兜底。
fn shorten_generation_reference(label: &str, max_chars: usize) -> String {
    let chars: Vec<char> = label.chars().collect();
    if chars.len() <= max_chars {
        return label.to_string();
    }
    const TAIL: usize = 10;
    let head = max_chars.saturating_sub(TAIL + 1);
    let mut short: String = chars[..head].iter().collect();
    short.push('…');
    short.extend(&chars[chars.len() - TAIL..]);
    short
}

fn generation_badge(label: SharedString, tone: gpui::Rgba, colors: UiColors) -> Div {
    div()
        .px_1p5()
        .max_w(px(140.))
        .rounded(px(4.))
        .border_1()
        .border_color(tone)
        .text_size(ui_text_size(9.5))
        .text_color(tone)
        .bg(colors.panel_bg)
        // 外键徽章可能带很长的「表.列」：截断而不是换行，避免整行被撑高。
        .child(div().truncate().child(label))
}

/// SQL 预览：按目标库方言展示前 3 行示例 INSERT（只读，不执行）。
fn data_generation_preview_sql(
    form: &TableDataGenerationForm,
    colors: UiColors,
    cx: &App,
) -> Div {
    let seed = generation_input_text(&form.seed_input, cx).trim().to_string();
    let seed = if seed.is_empty() {
        "fluxdb".to_string()
    } else {
        seed
    };
    let batch = generation_batch_rows(form, cx).unwrap_or(500);
    let spec = generation_spec_from_form(form, 3, seed, cx);
    let rows = spec.rows;
    let preview = fluxdb_core::preview_generation_sql(&spec, form.kind, &form.object, form.mode);
    let mono = ComponentTheme::global(cx).mono_font_family.to_string();

    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .px_3()
        .py_2();
    match preview {
        Ok(sql) => {
            body = body.child(
                div()
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.muted)
                    .child(
                        tr("app.data.data_generation.sql_hint")
                            .replace("%{batch}", &batch.to_string()),
                    ),
            );
            body = body.child(
                div()
                    .mt_1()
                    .font_family(mono.clone())
                    .text_size(ui_text_size(10.5))
                    .text_color(colors.text)
                    .child(sql),
            );
        }
        Err(error) => {
            body = body.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(error.message),
            );
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(body)
        .child(
            div()
                .flex_none()
                .px_3()
                .py_1p5()
                .border_t_1()
                .border_color(colors.border_soft)
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(
                    tr("app.data.data_generation.sql_footer")
                        .replace("%{rows}", &rows.to_string()),
                ),
        )
}

/// 右侧预览：「数据预览 / 消息日志」两页签，数据页用真实生成引擎取前 8 行。
fn data_generation_preview_panel(
    form: &TableDataGenerationForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let summary = {
        let writable = form
            .columns
            .iter()
            .filter(|column| column.generator != "skip")
            .count();
        let skipped = form.columns.len().saturating_sub(writable);
        let unique = form.columns.iter().filter(|column| column.unique).count();
        let foreign = form
            .columns
            .iter()
            .filter(|column| column.generator == "foreign_ref")
            .count();
        div()
            .flex()
            .flex_wrap()
            .gap_1p5()
            .px_2()
            .pt_2()
            .child(generation_badge(
                SharedString::from(
                    tr("app.data.data_generation.meta_writable")
                        .replace("%{count}", &writable.to_string()),
                ),
                colors.info,
                colors,
            ))
            .when(skipped > 0, |this| {
                this.child(generation_badge(
                    SharedString::from(
                        tr("app.data.data_generation.meta_skipped")
                            .replace("%{count}", &skipped.to_string()),
                    ),
                    colors.danger,
                    colors,
                ))
            })
            .child(generation_badge(
                SharedString::from(
                    tr("app.data.data_generation.meta_unique")
                        .replace("%{count}", &unique.to_string()),
                ),
                colors.warning,
                colors,
            ))
            .child(generation_badge(
                SharedString::from(
                    tr("app.data.data_generation.meta_foreign")
                        .replace("%{count}", &foreign.to_string()),
                ),
                colors.success,
                colors,
            ))
    };

    let seed = {
        let text = generation_input_text(&form.seed_input, cx).trim().to_string();
        if text.is_empty() {
            "—".to_string()
        } else {
            text
        }
    };

    let primary = ComponentTheme::global(cx).primary;
    let mut tabs = div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .h(px(36.))
        .border_b_1()
        .border_color(colors.border_soft);
    for (tab, label) in [
        (GenerationPreviewTab::Data, tr("app.data.data_generation.preview_data")),
        (GenerationPreviewTab::Sql, tr("app.data.data_generation.preview_sql")),
        (GenerationPreviewTab::Log, tr("app.data.data_generation.preview_log")),
    ] {
        let active = form.preview_tab == tab;
        tabs = tabs.child(
            div()
                .h_full()
                .px_3()
                .flex()
                .items_center()
                .border_b_2()
                .border_color(if active { primary.into() } else { rgba(0x00000000) })
                .text_size(ui_text_size(11.5))
                .font_weight(if active {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if active { colors.text } else { colors.muted })
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    this.set_data_generation_preview_tab(tab, cx);
                    cx.stop_propagation();
                }))
                .child(label),
        );
    }

    let body = match form.preview_tab {
        GenerationPreviewTab::Data => data_generation_preview_data(form, &seed, colors, cx)
            .into_any_element(),
        GenerationPreviewTab::Sql => data_generation_preview_sql(form, colors, cx)
            .into_any_element(),
        GenerationPreviewTab::Log => {
            import_dialog_log_body(data_generation_log_view(form), colors).into_any_element()
        }
    };

    div()
        .w(px(392.))
        .flex_none()
        .min_h(px(0.))
        .rounded(px(10.))
        .border_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(tabs)
        .child(summary)
        .child(body)
}

fn data_generation_preview_data(
    form: &TableDataGenerationForm,
    seed: &str,
    colors: UiColors,
    cx: &App,
) -> Div {
    let rows = data_generation_preview(form, cx);
    let mono = ComponentTheme::global(cx).mono_font_family.to_string();
    let mut content = div().flex().flex_col().px_2().py_2();
    if rows.is_empty() {
        content = content.child(
            div()
                .p_4()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.data.data_generation.preview_empty")),
        );
    } else {
        let mut header = div().flex().flex_none().h(px(24.));
        for column in &form.columns {
            header = header.child(
                div()
                    .w(px(140.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .truncate()
                    .text_size(ui_text_size(10.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.muted)
                    .child(column.name.clone()),
            );
        }
        content = content.child(header);
        for (row_index, row) in rows.iter().enumerate() {
            let mut line = div()
                .flex()
                .flex_none()
                .h(px(24.))
                .when(row_index % 2 == 1, |this| this.bg(colors.panel_alt));
            for column_index in 0..form.columns.len() {
                let value = row.get(column_index).cloned().unwrap_or_default();
                line = line.child(
                    div()
                        .w(px(140.))
                        .flex_none()
                        .px_2()
                        .flex()
                        .items_center()
                        .truncate()
                        .font_family(mono.clone())
                        .text_size(ui_text_size(10.5))
                        .child(value),
                );
            }
            content = content.child(line);
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .px_3()
                .py_1p5()
                .flex()
                .items_center()
                .justify_between()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(
                    tr("app.data.data_generation.preview_hint")
                        .replace("%{seed}", seed),
                )
                .child(
                    tr("app.data.data_generation.preview_rows")
                        .replace("%{count}", &rows.len().to_string()),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_x_scrollbar()
                .child(content),
        )
}

/// 底部统计小卡片：统一底色与内边距。
fn generation_footer_chip(text: String, colors: UiColors) -> Div {
    div()
        .px_2()
        .py_0p5()
        .rounded(px(6.))
        .bg(colors.panel_alt)
        .text_color(colors.text)
        .child(text)
}

/// 粗略估算写入体积：每行固定开销 48 字节 + 每列 8 字节（与设计稿同一口径）。
fn generation_estimated_size(rows: u64, columns: usize) -> String {
    let bytes = rows.saturating_mul(48 + columns as u64 * 8);
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024. / 1024.)
    } else {
        format!("{} KB", (bytes as f64 / 1024.).round().max(1.) as u64)
    }
}

/// 底部：预计写入量、跳过提示与执行按钮。
fn data_generation_footer(
    form: &TableDataGenerationForm,
    can_generate: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let finished = form.finished_at.is_some();
    let skipped_columns = form
        .columns
        .iter()
        .filter(|column| column.generator == "skip")
        .count();
    let rows = generation_input_text(&form.rows_input, cx)
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    let writable = form
        .columns
        .iter()
        .filter(|column| column.generator != "skip")
        .count();
    let size = generation_estimated_size(rows, writable);

    div()
        .h(px(56.))
        .flex_none()
        .px_4()
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(generation_footer_chip(
                    tr("app.data.data_generation.footer_will_write")
                        .replace("%{rows}", &rows.to_string()),
                    colors,
                ))
                .child(generation_footer_chip(
                    tr("app.data.data_generation.footer_size")
                        .replace("%{size}", &size),
                    colors,
                ))
                .when(skipped_columns > 0, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(colors.warning)
                            .child(app_icon(AppIcon::Info, 11., colors.warning))
                            .child(
                                tr("app.data.data_generation.footer_skipped")
                                    .replace("%{count}", &skipped_columns.to_string()),
                            ),
                    )
                })
                .when(finished, |this| {
                    this.child(
                        tr("app.data.data_generation.log_rows")
                            .replace("%{count}", &form.generated_rows.to_string()),
                    )
                    .when(form.skipped_rows > 0, |this| {
                        this.child(
                            tr("app.data.data_generation.footer_skipped_rows")
                                .replace("%{count}", &form.skipped_rows.to_string()),
                        )
                    })
                }),
        )
        .child(div().flex_1())
        // 日志页提供复制日志 / 清除任务；其余页保留常规取消与执行按钮。
        .when(form.preview_tab == GenerationPreviewTab::Log, |this| {
            this.child(
                Button::new("data-generation-copy-log")
                    .label(tr("app.data.data_import.copy_log"))
                    .disabled(form.logs.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.copy_data_generation_log(cx);
                        cx.stop_propagation();
                    })),
            )
            .when(finished, |this| {
                this.child(
                    Button::new("data-generation-clear-task")
                        .label(tr("app.data.data_import.clear_task"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_data_generation_task(cx);
                            cx.stop_propagation();
                        })),
                )
            })
        })
        .child(
            Button::new("data-generation-cancel")
                .label(tr("app.common.cancel"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_data_generation_wizard(cx);
                    cx.stop_propagation();
                })),
        )
        .when(form.running, |this| {
            this.child(
                Button::new("data-generation-stop")
                    .label(tr("app.data.data_generation.stop"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_data_generation_wizard(cx);
                        cx.stop_propagation();
                    })),
            )
        })
        .when(!form.running, |this| {
            this.child(
                Button::new("data-generation-run")
                    .label(tr("app.data.data_generation.start"))
                    .icon(gpui_component::Icon::default().path(app_icon_path(AppIcon::Play)))
                    .primary()
                    .disabled(!can_generate)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_data_generation(cx);
                        cx.stop_propagation();
                    })),
            )
        })
}

#[cfg(test)]
mod data_generation_foreign_key_tests {
    use super::*;

    fn orders_object() -> ObjectPath {
        ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("shop".to_string()),
            schema: Some("public".to_string()),
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        }
    }

    fn physical_foreign_key(column: &str, table: &str) -> ForeignKeyInfo {
        ForeignKeyInfo {
            name: format!("fk_{column}"),
            column: column.to_string(),
            ref_schema: Some("public".to_string()),
            ref_table: table.to_string(),
            ref_column: "id".to_string(),
            columns: Vec::new(),
            ref_columns: Vec::new(),
        }
    }

    /// 构造 ER 逻辑关系；`pairs` 控制列对数量，用于验证复合关系被跳过。
    fn er_relationship(
        left_table: &str,
        left_column: &str,
        right_table: &str,
        right_column: &str,
        pairs: usize,
    ) -> fluxdb_core::ErRelationship {
        let left = fluxdb_core::ErTableRef {
            database: "shop".to_string(),
            schema: Some("public".to_string()),
            name: left_table.to_string(),
        };
        let right = fluxdb_core::ErTableRef {
            database: "shop".to_string(),
            schema: Some("public".to_string()),
            name: right_table.to_string(),
        };
        fluxdb_core::ErRelationship {
            id: format!("rel_{left_table}_{right_table}"),
            revision: 1,
            left_entity: fluxdb_core::er_entity_id(&left),
            right_entity: fluxdb_core::er_entity_id(&right),
            role: "test".to_string(),
            column_pairs: (0..pairs)
                .map(|_| fluxdb_core::ErColumnPair {
                    left_column: fluxdb_core::er_column_id(&left, left_column),
                    right_column: fluxdb_core::er_column_id(&right, right_column),
                })
                .collect(),
            required_filters: Vec::new(),
            match_cardinality: fluxdb_core::ErMatchCardinality {
                left_to_right: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Unknown,
                },
                right_to_left: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Unknown,
                },
                basis: fluxdb_core::ErCardinalityBasis::Unknown,
            },
            origin: fluxdb_core::ErRelationshipOrigin::User,
            review: fluxdb_core::ErRelationshipReview {
                state: fluxdb_core::ErReviewState::Proposed,
                confirmed_revision: None,
                confirmed_by: None,
            },
            enforcement: fluxdb_core::ErRelationshipEnforcement {
                kind: fluxdb_core::ErEnforcementKind::None,
                constraint_ref: None,
                enforced: None,
            },
            validity: fluxdb_core::ErValidity {
                state: fluxdb_core::ErValidityState::Current,
                reason: None,
            },
            description: None,
            evidence_refs: Vec::new(),
        }
    }

    fn source_column(name: &str) -> fluxdb_app::DataGenerationSourceColumn {
        fluxdb_app::DataGenerationSourceColumn {
            name: name.to_string(),
            type_name: "integer".to_string(),
            nullable: true,
            primary_key: false,
            unique: false,
        }
    }

    #[test]
    fn merge_keeps_physical_key_and_adds_er_logical_target() {
        let object = orders_object();
        let merged = merge_generation_foreign_keys(
            vec![physical_foreign_key("user_id", "users")],
            &object,
            &[er_relationship("orders", "customer_id", "customers", "id", 1)],
        );
        assert_eq!(merged.len(), 2, "物理外键与 ER 关系都要保留");

        let user = generation_column_form(&source_column("user_id"), &object, &merged);
        assert_eq!(user.generator, "foreign_ref");
        let reference = user.reference.expect("物理外键应生成引用");
        assert_eq!(reference.table, "users");
        assert_eq!(reference.column, "id");
        assert_eq!(reference.schema.as_deref(), Some("public"));

        let customer = generation_column_form(&source_column("customer_id"), &object, &merged);
        assert_eq!(customer.generator, "foreign_ref");
        let reference = customer.reference.expect("ER 关系应生成引用");
        assert_eq!(reference.table, "customers");
        assert_eq!(reference.column, "id");
        assert_eq!(customer.reference_label.as_deref(), Some("customers.id"));

        // 没有任何关系来源的列仍按名称/类型推断。
        let plain = generation_column_form(&source_column("amount"), &object, &merged);
        assert_ne!(plain.generator, "foreign_ref");
        assert!(plain.reference.is_none());
    }

    #[test]
    fn physical_foreign_key_wins_when_er_describes_same_column() {
        let object = orders_object();
        let merged = merge_generation_foreign_keys(
            vec![physical_foreign_key("user_id", "users")],
            &object,
            &[er_relationship("orders", "user_id", "legacy_users", "id", 1)],
        );
        let column = generation_column_form(&source_column("user_id"), &object, &merged);
        let reference = column.reference.expect("应有引用");
        assert_eq!(
            reference.table, "users",
            "同列优先使用数据库真实校验的物理外键"
        );
    }

    #[test]
    fn primary_key_column_never_copies_parent_values() {
        let object = orders_object();
        let mut id = source_column("id");
        id.primary_key = true;
        id.unique = true;
        // ER 关系在子表侧生成反向投影：orders.id ← regions.country_id。
        let merged = merge_generation_foreign_keys(
            Vec::new(),
            &object,
            &[er_relationship("orders", "id", "regions", "country_id", 1)],
        );
        let column = generation_column_form(&id, &object, &merged);
        assert!(
            column.reference.is_none(),
            "主键不能取子表/父表引用池的值"
        );
        assert_eq!(column.generator, "sequence");
    }

    #[test]
    fn composite_er_relationship_is_not_used_for_column_values() {
        let object = orders_object();
        let merged = merge_generation_foreign_keys(
            Vec::new(),
            &object,
            &[er_relationship("orders", "customer_id", "customers", "id", 2)],
        );
        assert!(
            merged.is_empty(),
            "复合关系不能只取其中一列，生成阶段应完全忽略"
        );
        let column = generation_column_form(&source_column("customer_id"), &object, &merged);
        assert_ne!(column.generator, "foreign_ref");
    }

    #[test]
    fn long_reference_badge_keeps_table_head_and_column_tail() {
        // 短标签原样返回，只有超长时才省略中间。
        assert_eq!(shorten_generation_reference("FK → 3d_countries.id", 24), "FK → 3d_countries.id");
        let short = shorten_generation_reference(
            "FK → production_orders_archive_2024.legacy_customer_identifier",
            24,
        );
        assert_eq!(short.chars().count(), 24);
        assert!(short.starts_with("FK →"), "前缀要保留：{short}");
        assert!(short.contains('…'), "中间要有省略号：{short}");
        assert!(short.ends_with("identifier"), "列名结尾要保留：{short}");
    }
}
