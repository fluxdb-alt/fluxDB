// MongoDB 查询面板：三层布局（Filter 首行 + 更多选项 + 高级查询）。
//
// 不连库：只验面板自身的渲染与展开/收起，查询请求失败由结果区自己的错误块承担。

#[gpui::test]
fn mongo_query_panel_keeps_filter_row_and_collapses_options(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    // 「条数」跟随设置里的默认查询限制：这里选 500，面板初值就必须是 500 而不是内置默认值。
    controller.dispatch(AppCommand::SaveSettings(fluxdb_core::Settings {
        page_size: 500,
        ..Default::default()
    }));
    let connection_id = ConnectionId(11);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "mongo query panel".into(),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 27017,
            database: None,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }]));
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
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
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
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenMongoCollection {
                    object: ObjectPath {
                        connection_id,
                        database: Some("shop".into()),
                        schema: None,
                        name: "orders".into(),
                        kind: ObjectKind::Collection,
                    },
                    read_only: false,
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    // Filter 是首行主入口；投影/排序/条数默认收在 Options 里。
    let filter = cx
        .debug_bounds("mongo-query-filter")
        .expect("filter row must be visible");
    let find = cx
        .debug_bounds("mongo-query-find")
        .expect("find button must render");
    let reset = cx
        .debug_bounds("mongo-query-reset")
        .expect("reset button must render");
    // 同一行：三者垂直中心相同，否则按钮会掉到第二行（旧布局的问题）。
    let center = |b: &gpui::Bounds<gpui::Pixels>| b.center().y;
    assert!(
        (center(&filter) - center(&find)).abs() <= gpui::px(1.)
            && (center(&find) - center(&reset)).abs() <= gpui::px(1.),
        "filter and action buttons must share one row: {filter:?} {find:?} {reset:?}"
    );
    assert!(
        find.center().x > filter.center().x,
        "actions stay right of the filter input"
    );
    // 三个按钮同高：组内高度不一致会读成三块拼凑的控件。
    let options = cx
        .debug_bounds("mongo-query-options")
        .expect("options toggle must render");
    assert!(
        (find.size.height - reset.size.height).abs() <= gpui::px(0.5)
            && (find.size.height - options.size.height).abs() <= gpui::px(0.5),
        "find, reset and options must share one action height: {find:?} {reset:?} {options:?}"
    );
    // 但按钮要比输入框矮一档：跟输入框同高会让按钮显得又粗又笨。
    assert!(
        find.size.height < filter.size.height,
        "action buttons stay smaller than the filter input: \
         find={find:?} filter={filter:?}"
    );
    // 矮一档的按钮仍要落在这条行的中线上，不能贴顶或贴底。
    assert!(
        (center(&find) - filter.center().y).abs() <= gpui::px(1.)
            && (center(&options) - filter.center().y).abs() <= gpui::px(1.),
        "smaller action buttons stay on the row's center line: {find:?} {options:?} {filter:?}"
    );
    // 首行**字形**必须落在外框的垂直中线上，且编辑器顶边贴住内容区顶部。
    //
    // 多行编辑器的文本永远从「内容区顶部 + 8px 内边距」起排，这段内边距由
    // `Input::from_state` 写死，外部改不掉。所以居中的唯一判据是：
    //   编辑器顶边 + 8px + 行高/2 == 外框内容区中线
    //
    // 前两版都在这里翻车，且各自能骗过某条弱断言：
    // 「行高铺满内容区」→ 上内边距把字形整体压低 8px；「盒子垂直居中」→ 盒子高度随行数
    // 变化（实测两行时 52px），首行起点跟着行数漂。所以这里同时固定顶边位置。
    let editor = cx
        .debug_bounds("mongo-query-editor:filter")
        .expect("filter editor must render");
    let glyph_center = editor.origin.y
        + gpui::px(crate::mongo_query_editor_padding())
        + gpui::px(crate::mongo_query_line_height()) / 2.;
    assert!(
        (glyph_center - filter.center().y).abs() <= gpui::px(1.),
        "first line's glyphs sit on the frame's vertical center: \
         glyph_center={glyph_center:?} frame={filter:?} editor={editor:?}"
    );
    // 顶边贴内容区顶部，不能靠自适应居中：居中时首行位置会随内容行数漂移。
    assert!(
        (editor.origin.y - (filter.origin.y + gpui::px(1.))).abs() <= gpui::px(1.),
        "editor is pinned to the content box top: frame={filter:?} editor={editor:?}"
    );
    // 编辑器必须紧接着铭牌，而不是浮在整格中间。
    //
    // 外框曾经给内容多包了一层 `flex_1`，铭牌和编辑器成了两个等分兄弟：
    // 铭牌只占自身宽度、剩下的空白落在编辑器左边，`{}` 就飘到了整行正中。
    assert!(
        editor.origin.x < filter.origin.x + filter.size.width / 4.,
        "filter editor starts right after the label plate, not in the middle: \
         editor={editor:?} filter={filter:?}"
    );
    assert!(
        cx.debug_bounds("mongo-query-field:projection").is_none(),
        "options must start collapsed"
    );

    cx.simulate_click(options.center(), Modifiers::default());
    // 首层一行：条件构建入口 + 投影 / 排序 / 返回条数（交互稿 advanced-options 列序）。
    // 低频参数在第二层「高级查询」里，首层展开时不能出现——否则「两层折叠」等于一层。
    // 用宏而不是闭包：`debug_bounds` 要 `&mut` 上下文且选择器必须是 `'static`。
    macro_rules! field {
        ($name:literal) => {
            cx.debug_bounds(concat!("mongo-query-field:", $name))
                .unwrap_or_else(|| panic!("mongo-query-field:{} must render", $name))
        };
    }
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("condition builder entry sits in the options row");
    let projection = field!("projection");
    let sort = field!("sort");
    let limit = field!("limit");
    for hidden in [
        "mongo-query-field:collation",
        "mongo-query-field:skip",
        "mongo-query-field:hint",
        "mongo-query-field:max_time_ms",
    ] {
        assert!(
            cx.debug_bounds(hidden).is_none(),
            "advanced field {hidden} must stay collapsed until 高级查询 is expanded"
        );
    }
    assert!(
        projection.origin.y >= filter.origin.y + filter.size.height,
        "expanded options stack below the filter row"
    );
    // 四格同一行：条件构建在最左，其后依次是投影、排序、返回条数。名称已塞进框内铭牌，
    // 整格与输入框本体重合，这里直接比整格。
    let projection_box = cx
        .debug_bounds("mongo-query-editor-box:projection")
        .expect("projection input box must render");
    assert!(
        (builder_toggle.center().y - projection_box.center().y).abs() <= gpui::px(2.)
            && projection.origin.x < sort.origin.x
            && sort.origin.x < limit.origin.x
            && builder_toggle.origin.x < projection.origin.x,
        "builder entry, projection, sort and limit share one row, left to right: \
         {builder_toggle:?} {projection_box:?} {projection:?} {sort:?} {limit:?}"
    );
    // 展开区首列（条件构建）与 Filter 基本齐平：整块才是一张网，而不是往右缩进的子表单。
    // 只比首列——投影在第二列，本来就该往右排。
    // 允许 4px 差：交互稿里首行内边距是 8px、展开区是 7px 12px，这一档缩进是设计本身。
    assert!(
        (builder_toggle.origin.x - filter.origin.x).abs() <= gpui::px(4.5),
        "options must align with the filter row: {builder_toggle:?} {projection:?} {filter:?}"
    );
    // 数字格是定宽窄框，对象编辑器占剩余宽度：否则一行里的右边界散乱。
    assert!(
        limit.size.width < projection.size.width,
        "numeric fields stay short: {limit:?} {projection:?}"
    );
    // 附加格子的输入框比 Filter 更扁：它们不是主入口，纵向空间要留给下面的结果区。
    // 同时字形仍要在框的中线上，压过头会把首行字裁掉半截。
    let projection_editor = projection_box;
    assert!(
        projection_editor.size.height < filter.size.height
            && projection_editor.size.height >= gpui::px(30.),
        "option inputs stay flatter than the filter row without collapsing: \
         {projection_editor:?} {filter:?}"
    );
    let option_glyph_center = projection_editor.origin.y
        + gpui::px(crate::mongo_query_editor_padding())
        + gpui::px(crate::mongo_query_option_line_height()) / 2.;
    assert!(
        (option_glyph_center - projection_editor.center().y).abs() <= gpui::px(1.),
        "option text stays on the box's center line: \
         glyph_center={option_glyph_center:?} box={projection_editor:?}"
    );

    // 第二层：展开「高级查询」才出现低频参数，且四格排成一行（交互稿 expert-options）。
    //
    // 入口挂在首层那一行的末尾，不单独占一行——单独一行只放一个 10px 文字按钮，
    // 收起时白花一条横向带，正是结果区被挤走的来源之一。
    let advanced = cx
        .debug_bounds("mongo-query-advanced")
        .expect("advanced toggle must render");
    assert!(
        (advanced.center().y - projection.center().y).abs() <= gpui::px(2.)
            && advanced.origin.x > limit.origin.x,
        "advanced toggle rides the end of the common options row: {advanced:?} {projection:?}"
    );
    cx.simulate_click(advanced.center(), Modifiers::default());
    let collation = field!("collation");
    let skip = field!("skip");
    let hint = field!("hint");
    let max_time = field!("max_time_ms");
    assert!(
        collation.origin.y > advanced.origin.y,
        "advanced fields expand below the toggle: {collation:?} {advanced:?}"
    );
    assert!(
        (collation.origin.y - skip.origin.y).abs() <= gpui::px(0.5)
            && (hint.origin.y - max_time.origin.y).abs() <= gpui::px(0.5)
            && (collation.origin.y - max_time.origin.y).abs() <= gpui::px(0.5),
        "expert fields sit on one row: {collation:?} {skip:?} {hint:?} {max_time:?}"
    );
    assert!(
        collation.origin.x < skip.origin.x
            && skip.origin.x < hint.origin.x
            && hint.origin.x < max_time.origin.x,
        "expert cells read left to right: {collation:?} {skip:?} {hint:?} {max_time:?}"
    );
    // 两个数字格定宽且等宽；对象格吃掉剩余宽度，因此比它们宽。
    assert!(
        skip.size.width < collation.size.width
            && (max_time.size.width - skip.size.width).abs() <= gpui::px(0.5),
        "expert numeric cells keep their own widths: {skip:?} {max_time:?} {collation:?}"
    );
    // 数字格的字必须落在格子的中线上：单行 `Input` 只认自己的行高，不吃外面的
    // `h_full`，不额外居中就会贴住 32px 格子的顶边（截图里 `0` 比「起始位置」高一截）。
    let skip_body = cx
        .debug_bounds("mongo-query-number-body:skip")
        .expect("skip input body must render");
    assert!(
        (skip_body.center().y - skip.center().y).abs() <= gpui::px(1.5),
        "numeric input sits on its cell's center line: body={skip_body:?} cell={skip:?}"
    );
    let max_time_body = cx
        .debug_bounds("mongo-query-number-body:max_time_ms")
        .expect("max time input body must render");
    assert!(
        (max_time_body.center().y - max_time.center().y).abs() <= gpui::px(1.5),
        "max time input sits on its cell's center line: body={max_time_body:?} cell={max_time:?}"
    );
    // 收起第二层不影响首层。
    let advanced = cx.debug_bounds("mongo-query-advanced").expect("toggle stays");
    cx.simulate_click(advanced.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-field:collation").is_none()
            && cx.debug_bounds("mongo-query-field:projection").is_some(),
        "collapsing 高级查询 keeps the common options open"
    );
    let advanced = cx.debug_bounds("mongo-query-advanced").expect("toggle stays");
    cx.simulate_click(advanced.center(), Modifiers::default());

    // 条数初值来自设置里的「默认查询限制」，不是写死的常数；超出上限的输入必须被拒绝，
    // 否则用户填 1000 执行时被连接器悄悄夹到 500，看起来像丢了数据。
    let tab_id = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .active_tab
            .expect("mongo collection tab is active")
    });
    let limit_input = cx.update(|_, cx| {
        view.read(cx)
            .mongo_query_inputs
            .get(&tab_id)
            .expect("query inputs are created for the tab")
            .read(cx)
            .limit
            .clone()
    });
    let limit_value = cx.update(|_, cx| limit_input.read(cx).value().to_string());
    assert_eq!(limit_value, "500", "limit seeds from the query limit setting");
    // 名称现在塞在框内左侧铭牌里，整格都是输入框本体，点哪都聚焦；
    // 点偏右一些避开铭牌，确保落点在输入区。
    cx.simulate_click(
        gpui::point(
            limit.center().x + limit.size.width / 4.,
            limit.center().y,
        ),
        Modifiers::default(),
    );
    cx.simulate_keystrokes("backspace backspace backspace");
    cx.simulate_input("100");
    let limit_value = cx.update(|_, cx| limit_input.read(cx).value().to_string());
    assert_eq!(
        limit_value, "100",
        "the limit field takes typed digits (focus must be in the box)"
    );
    cx.simulate_input("0");
    let limit_value = cx.update(|_, cx| limit_input.read(cx).value().to_string());
    assert!(
        limit_value != "1000"
            && limit_value
                .parse::<u64>()
                .map_or(true, |limit| limit <= 500),
        "the limit field rejects values above the configured ceiling: {limit_value:?}"
    );

    // 条件构建：入口在「更多选项」那一行，展开后紧贴 Filter 下方。
    //
    // 展开时解析现有 Filter（这里是 `{}`），所以不会产生任何条件行；
    // 复杂表达式则只报错、不改 JSON——下面单独验。
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    let builder = cx
        .debug_bounds("mongo-query-builder")
        .expect("builder section renders under the filter row");
    assert!(
        builder.origin.y >= filter.origin.y + filter.size.height,
        "builder sits directly under the filter row: {builder:?} {filter:?}"
    );
    assert!(
        builder.origin.y < projection.origin.y,
        "builder comes before the options grid: {builder:?} {projection:?}"
    );
    let add = cx
        .debug_bounds("mongo-query-builder-add:root")
        .expect("add condition must render");
    cx.simulate_click(add.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-builder-row:0").is_some(),
        "adding a condition creates its row"
    );
    // 字段与值各自分到实际宽度：值格按内容定基数时会被字段格挤成一条缝。
    let field_cell = cx
        .debug_bounds("mongo-query-builder-field:0")
        .expect("field cell renders");
    let value_cell = cx
        .debug_bounds("mongo-query-builder-value:0")
        .expect("value cell renders");
    assert!(
        field_cell.size.width >= gpui::px(140.) && value_cell.size.width >= gpui::px(120.),
        "field and value cells keep usable widths: {field_cell:?} {value_cell:?}"
    );
    // 字段格与值格都是代码编辑器——只有编辑器能挂补全。
    //
    // 两格一律不许越出格子宽度：它们曾因多行编辑器溢出而长出拖不动的滚动条。
    // 垂直位置不在这里量（那时文字还没排好），下面用光标行对齐格子中线来验。
    let field_body = cx
        .debug_bounds("mongo-query-builder-field-body:0")
        .expect("field editor body renders");
    assert!(
        field_body.size.width <= field_cell.size.width,
        "field editor keeps its cell's width: body={field_body:?} cell={field_cell:?}"
    );
    let value_body = cx
        .debug_bounds("mongo-query-builder-value-body:0")
        .expect("value editor body renders");
    assert!(
        value_body.size.width <= value_cell.size.width,
        "value editor keeps its cell's width: body={value_body:?} cell={value_cell:?}"
    );
    // 键盘输入落进格子本身的输入状态，且字段格真的挂上了补全 provider。
    //
    // 这里不验候选内容：候选来自字段目录，而目录要靠真实集合的抽样回填，无库的
    // 布局测试里它是空的。候选的筛选规则由 `mongo_completion_tests` 单测覆盖；
    // 这一步只保证接线没断——provider 装上、焦点可达、打字能进编辑器。
    let (row_field, row_value) = cx.update(|_, cx| {
        let builder = view
            .read(cx)
            .mongo_query_builders
            .get(&tab_id)
            .expect("builder state");
        let row = builder
            .condition(&[0])
            .and_then(|condition| condition.row.as_ref())
            .expect("first condition row");
        (row.field.clone(), row.value.clone())
    });
    cx.update(|_, cx| {
        assert!(
            row_field.read(cx).lsp().completion_provider.is_some(),
            "the field cell must have a field-path completion provider"
        );
        assert!(
            row_value.read(cx).lsp().completion_provider.is_some(),
            "the value cell must have a value completion provider"
        );
    });
    // 只有一个条件时条件树不该有可滚区间。
    //
    // 树把内边距放在内层就是为这条：滚动容器自己带内边距时，gpui 会把它同时
    // 算进内容与视口，`max_offset` 恒为两倍内边距，右侧于是挂着一根拖不动的
    // 滚动条。
    cx.update(|_, cx| {
        let builder = view
            .read(cx)
            .mongo_query_builders
            .get(&tab_id)
            .expect("builder state");
        assert_eq!(
            builder.scroll.max_offset(),
            gpui::point(gpui::px(0.), gpui::px(0.)),
            "one condition row fits the tree, so it must not scroll"
        );
    });
    cx.simulate_click(field_cell.center(), Modifiers::default());
    cx.simulate_input("status");
    cx.update(|_, cx| {
        assert_eq!(
            row_field.read(cx).value().to_string(),
            "status",
            "typing lands in the field cell's editor"
        );
    });
    // 文字落在格子的**中线**上。
    //
    // 编辑器不像单行 `Input` 会自己垂直居中：它从上内边距往下排，盒子贴着
    // 28px 的格子放就会低 3.5px。量的是光标那一行的中心——字形就在这一行里。
    let cursor_row = cx.update(|_, cx| {
        row_field
            .read(cx)
            .cursor_layout()
            .expect("the field editor lays its cursor out")
            .0
    });
    assert!(
        (cursor_row.center().y - field_cell.center().y).abs() <= gpui::px(1.5),
        "the field text sits on its cell's center line: cursor={cursor_row:?} cell={field_cell:?}"
    );
    // 新行默认空值：不写进 JSON，Filter 仍是空对象——加了一行还没填不该把结果查空。
    let filter_value = cx.update(|_, cx| {
        view.read(cx)
            .mongo_query_inputs
            .get(&tab_id)
            .expect("query inputs")
            .read(cx)
            .filter
            .read(cx)
            .value()
            .to_string()
    });
    assert_eq!(filter_value, "{}", "empty condition rows stay out of the filter");
    // 移除后只剩空 JSON，行也没了。
    let remove = cx
        .debug_bounds("mongo-query-builder-remove:0")
        .expect("remove button must render");
    cx.simulate_click(remove.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-builder-row:0").is_none(),
        "removing the row drops it from the builder"
    );
    // 组内子项数不设上限：连加七条都要落成行。
    //
    // 曾经卡在每组 6 个直接子项，第 7 条时「添加条件」无声置灰，条件树却还没滚动
    // ——用户看到的就是「加了 6 个条件就无法添加了」。上限只剩总数 24 条。
    for _ in 0..7 {
        let add = cx
            .debug_bounds("mongo-query-builder-add:root")
            .expect("add condition must render");
        cx.simulate_click(add.center(), Modifiers::default());
    }
    assert!(
        cx.debug_bounds("mongo-query-builder-row:6").is_some(),
        "a single group keeps accepting rows past six"
    );
    for _ in 0..7 {
        let remove = cx
            .debug_bounds("mongo-query-builder-remove:0")
            .expect("remove button must render");
        cx.simulate_click(remove.center(), Modifiers::default());
    }
    assert!(
        cx.debug_bounds("mongo-query-builder-row:0").is_none(),
        "clearing the seven probe rows leaves the builder empty"
    );
    // 先收起：下面要验的是「带着复杂表达式**重新展开**」时的拒绝路径。
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-builder").is_none(),
        "clicking the builder entry again collapses it"
    );

    // 添加子组：子组默认 AND，再添加第二条同字段条件；切为 OR 后不能被对象键覆盖。
    let filter_editor = cx.update(|_, cx| {
        view.read(cx)
            .mongo_query_inputs
            .get(&tab_id)
            .expect("query inputs")
            .read(cx)
            .filter
            .clone()
    });
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    let add_group = cx
        .debug_bounds("mongo-query-builder-add-group:root")
        .expect("add subgroup must render");
    // 工具条是单行流式布局，内容超宽会把右侧按钮推出卡片、甚至推到窗口外。
    // 窗口外的点击被 gpui 静默丢弃（表现为「点了没反应」），所以先卡住边界。
    let window_bounds = cx.update(|window, _| window.bounds());
    assert!(
        window_bounds.contains(&add_group.center()),
        "toolbar buttons must stay inside the window: {add_group:?} {window_bounds:?}"
    );
    cx.simulate_click(add_group.center(), Modifiers::default());
    assert!(cx.debug_bounds("mongo-query-builder-group:0").is_some());
    let nested_add = cx
        .debug_bounds("mongo-query-builder-add:0")
        .expect("subgroup condition button");
    cx.simulate_click(nested_add.center(), Modifiers::default());
    for (path, field, value) in [("0-0", "status", "\"paid\""), ("0-1", "status", "\"pending\"")] {
        let field_input = cx.update(|_, cx| {
            view.read(cx)
                .mongo_query_builders
                .get(&tab_id)
                .and_then(|builder| {
                    let path: Vec<usize> = path.split('-').map(|part| part.parse().expect("path")).collect();
                    builder.condition(&path).and_then(|condition| condition.row.as_ref()).map(|row| (row.field.clone(), row.value.clone()))
                })
                .expect("nested condition row")
        });
        cx.update(|window, cx| {
            field_input.0.update(cx, |input, cx| input.set_value(field, window, cx));
            field_input.1.update(cx, |input, cx| input.set_value(value, window, cx));
        });
    }
    // 分段控件里点 OR：不再有下拉菜单，交互是就地切换。
    let nested_or = cx
        .debug_bounds("mongo-query-builder-logic:0:or")
        .expect("subgroup OR segment");
    cx.simulate_click(nested_or.center(), Modifiers::default());
    cx.run_until_parked();
    let nested_value = cx
        .debug_bounds("mongo-query-builder-value:0-1")
        .expect("nested value input");
    cx.simulate_click(nested_value.center(), Modifiers::default());
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("\"archived\"");
    let nested_filter = cx.update(|_, cx| filter_editor.read(cx).value().to_string());
    assert_eq!(
        nested_filter,
        r#"{"$or":[{"status":"paid"},{"status":"archived"}]}"#,
        "OR preserves both conditions and nested editor changes reach Filter"
    );
    let remove_group = cx
        .debug_bounds("mongo-query-builder-remove-group:0")
        .expect("remove subgroup");
    cx.simulate_click(remove_group.center(), Modifiers::default());
    assert!(cx.debug_bounds("mongo-query-builder-group:0").is_none());

    // 根 `$or` 能从 Filter 反向展开：两个分支各成一组、同字段条件不互相覆盖。
    // 往返后文本必须原样（顺序、键名都不许被改写）。
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    assert!(cx.debug_bounds("mongo-query-builder").is_none());
    cx.update(|window, cx| {
        filter_editor.update(cx, |editor, cx| {
            editor.set_value(
                r#"{"$or":[{"status":"paid"},{"status":"pending"}]}"#,
                window,
                cx,
            )
        });
    });
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("mongo-query-builder-row:0-0").is_some()
            && cx.debug_bounds("mongo-query-builder-row:1-0").is_some(),
        "root OR conditions remain editable without losing duplicate fields"
    );
    let round_trip = cx.update(|_, cx| filter_editor.read(cx).value().to_string());
    assert_eq!(
        round_trip, r#"{"$or":[{"status":"paid"},{"status":"pending"}]}"#,
        "parsing the builder must not rewrite the filter text"
    );
    // 收起来，后面用回简单查询。
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-builder").is_none(),
        "collapsing after a round trip still works"
    );

    // 折叠态提示：附加条件在界面上不可见时，唯一提示就是折叠入口旁的计数。
    //
    // 这里不点 Find 造条件：没有真库，首屏那次查询会在驱动超时里挂着，Find 一直 disabled。
    // 改走分页命令——它把起点写进已生效查询（`applied_query.skip`），是生产路径，
    // 又不碰连接器。
    cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(AppCommand::SetMongoOffset { tab_id, offset: 40 }, cx);
        });
    });
    let applied = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                fluxdb_app::TabKind::MongoCollection(state) => Some(state.applied_query.clone()),
                _ => None,
            })
            .expect("mongo collection tab stays open")
    });
    assert_eq!(applied.skip_value(), 40, "offset is part of the applied query");
    let options = cx
        .debug_bounds("mongo-query-options")
        .expect("options toggle stays visible");
    cx.simulate_click(options.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-option-count").is_some(),
        "collapsed options must still advertise the active conditions"
    );
    assert!(
        cx.debug_bounds("mongo-query-field:projection").is_none(),
        "options collapse after the toggle"
    );

    // 首层开着、第二层收起时，低频条件同样看不见：第二层入口自己也要给计数。
    let options = cx
        .debug_bounds("mongo-query-options")
        .expect("options toggle stays visible");
    cx.simulate_click(options.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-field:projection").is_some()
            && cx.debug_bounds("mongo-query-field:skip").is_none(),
        "re-expanding options keeps 高级查询 collapsed"
    );
    assert!(
        cx.debug_bounds("mongo-query-advanced-count").is_some(),
        "collapsed 高级查询 must advertise its own hidden conditions"
    );
    let advanced = cx
        .debug_bounds("mongo-query-advanced")
        .expect("advanced toggle stays visible");
    cx.simulate_click(advanced.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-field:skip").is_some()
            && cx.debug_bounds("mongo-query-advanced-count").is_none(),
        "expanded 高级查询 shows its fields and drops the count"
    );

    // 聚焦态必须有可见反馈：输入框本体关掉了自带描边（`appearance(false)`），焦点态
    // 由外框补上。两种状态的描边取同一个颜色就等于没有焦点态，点进输入框后整块毫无变化。
    let colors = cx.update(|_, cx| crate::ui_colors_from_theme(gpui_component::ThemeMode::Light, cx));
    assert_ne!(
        crate::mongo_query_input_border(true, colors),
        crate::mongo_query_input_border(false, colors),
        "focused and unfocused filters must paint different frame borders"
    );

    // 再点一次收起：展开方式不记忆在标签页状态之外，点了就要有反应。
    let options = cx
        .debug_bounds("mongo-query-options")
        .expect("options toggle stays visible");
    cx.simulate_click(options.center(), Modifiers::default());
    assert!(
        cx.debug_bounds("mongo-query-field:projection").is_none(),
        "clicking options again collapses them"
    );

    // Filter 随内容增行（Compass 的回车加高），封顶 4 行，再多由编辑器自己滚。
    //
    // Filter 必须留在 `EditorState` 上——补全只挂在它上面；而 `auto_grow` 是
    // `TextareaState` 独有的、且是 `gpui-base` 的 crate 内 API。所以行数由视图数
    // 换行符推出、框高倒推。这里验的是真实布局高度，不是推导函数自说自话。
    assert_eq!(
        crate::mongo_query_filter_rows(&"{}".repeat(9)),
        1,
        "没有换行的长文本不折行，仍算一行"
    );
    assert_eq!(
        crate::mongo_query_filter_rows("a\nb\nc\nd\ne"),
        4,
        "行数封顶 4 行"
    );
    let filter_editor = cx.update(|_, cx| {
        view.read(cx)
            .mongo_query_inputs
            .get(&tab_id)
            .expect("query inputs")
            .read(cx)
            .filter
            .clone()
    });
    let height_at = |cx: &mut gpui::VisualTestContext, lines: usize| {
        let text = (0..lines)
            .map(|line| format!("// {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        cx.update(|window, cx| {
            filter_editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.debug_bounds("mongo-query-filter")
            .expect("filter row renders")
            .size
            .height
    };
    let one_row = height_at(cx, 1);
    let two_rows = height_at(cx, 2);
    let four_rows = height_at(cx, 4);
    let six_rows = height_at(cx, 6);
    assert!(
        two_rows > one_row + gpui::px(8.),
        "a second line grows the filter frame: one={one_row:?} two={two_rows:?}"
    );
    assert!(
        four_rows > two_rows + gpui::px(8.),
        "the frame keeps growing up to four lines: two={two_rows:?} four={four_rows:?}"
    );
    assert!(
        (six_rows - four_rows).abs() <= gpui::px(0.5),
        "past four lines the frame stops growing and the editor scrolls: \
         four={four_rows:?} six={six_rows:?}"
    );

    // 条件树封顶 240px：条件多了必须在树内滚动，不能把结果区一路挤下去。
    //
    // 光靠根组铺满 6 条条件只有约 198px，够不到封顶，所以让 Filter 解析出一棵更高的树：
    // 根 `$or` 下四个展开的子组、每组三条条件（约 450px）。走的是生产路径
    // （解析 Filter → 渲染 → 量边界），不是给测试造的旁路。
    //
    // 逐条点「添加」不行：加满第一个子组后，第二个子组的按钮已被滚出封顶视口，
    // 窗口外的点击被 gpui 静默丢弃。
    let filter_editor = cx.update(|_, cx| {
        view.read(cx)
            .mongo_query_inputs
            .get(&tab_id)
            .expect("query inputs")
            .read(cx)
            .filter
            .clone()
    });
    cx.update(|window, cx| {
        filter_editor.update(cx, |editor, cx| {
            editor.set_value(
                r#"{"$or":[
                    {"a1":1,"a2":2,"a3":3},
                    {"b1":1,"b2":2,"b3":3},
                    {"c1":1,"c2":2,"c3":3},
                    {"d1":1,"d2":2,"d3":3}
                ]}"#,
                window,
                cx,
            );
        });
    });
    let options = cx
        .debug_bounds("mongo-query-options")
        .expect("options toggle stays visible");
    cx.simulate_click(options.center(), Modifiers::default());
    let builder_toggle = cx
        .debug_bounds("mongo-query-builder-toggle")
        .expect("builder entry stays visible");
    cx.simulate_click(builder_toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let tree = cx
        .debug_bounds("mongo-query-builder-tree")
        .expect("conditions tree renders");
    // 内容远超上限，所以树必须**顶到** 240px：正好等于 240 才说明封顶生效且没被外层压扁。
    // 只断言「不超过」会漏掉「树被挤成几十像素」这种同样看不见条件的坏法。
    assert!(
        (tree.size.height - gpui::px(crate::MONGO_QUERY_BUILDER_TREE_MAX_HEIGHT)).abs()
            <= gpui::px(0.5),
        "the conditions tree fills its {}px cap and scrolls inside: {tree:?}",
        crate::MONGO_QUERY_BUILDER_TREE_MAX_HEIGHT
    );
    // 封顶后靠下的行落到可视区之外；滚轮必须能把它们滚进视口。滚动条与滚轮都来自
    // 原生 `overflow_y_scroll`，滚不动就说明树被压成 0 高或没有可滚区间
    // ——这正是「条件太多时没有滑块」的那个 bug。
    let last_row = "mongo-query-builder-row:3-2";
    let before = cx
        .debug_bounds(last_row)
        .expect("last row is laid out inside the scroll content")
        .origin
        .y;
    assert!(
        before + gpui::px(crate::MONGO_QUERY_BUILDER_CONTROL_HEIGHT)
            > tree.origin.y + tree.size.height,
        "the overflowing row starts outside the capped viewport: {before:?} {tree:?}"
    );
    let wheel_position = gpui::point(
        tree.origin.x + gpui::px(20.),
        tree.origin.y + tree.size.height - gpui::px(20.),
    );
    // 滚动条覆盖层的边界必须**不随内容移动**。
    //
    // 覆盖层是 `absolute().inset_0()`：挂在滚动容器自己身上时它就成了滚动内容的
    // 子节点，内容滚多远它跟多远，拇指相对覆盖层的位置不变——用户看到的就是
    // 「滚了但滚动条不动」。所以它得挂在外层。这里用滚动前后的边界相等来锁。
    let overlay_before = cx
        .debug_bounds("scrollbar-overlay")
        .expect("the tree's scrollbar overlay renders");
    for _ in 0..3 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: wheel_position,
            delta: gpui::ScrollDelta::Lines(gpui::point(0., -6.)),
            ..Default::default()
        });
    }
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let after = cx
        .debug_bounds(last_row)
        .expect("last row still renders after scrolling")
        .origin
        .y;
    assert!(
        after < before,
        "wheel scrolling must move the capped tree content: {before} -> {after}"
    );
    let overlay_after = cx
        .debug_bounds("scrollbar-overlay")
        .expect("the tree's scrollbar overlay renders");
    assert_eq!(
        overlay_before, overlay_after,
        "the scrollbar overlay is a viewport overlay and must not scroll with the content"
    );
    // 覆盖层要盖住整个可视区，且不越出树外框。
    let tree_after = cx
        .debug_bounds("mongo-query-builder-tree")
        .expect("conditions tree renders");
    assert!(
        (overlay_after.size.height - tree_after.size.height).abs() <= gpui::px(1.),
        "the overlay matches the tree viewport: overlay={overlay_after:?} tree={tree_after:?}"
    );
}
