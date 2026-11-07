    /// 连接表单字段按“高频在首屏、低频在高级”分组。
    ///
    /// 守住三件事：MongoDB SRV 与 PostgreSQL 默认 schema 留在连接信息；
    /// MySQL 字符集/URL 参数只在高级页出现；SQLite 隐藏空的网络与安全页签。
    #[gpui::test]
    fn connection_tabs_group_common_fields_first(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, _view, _temp) = redis_e2e_view(cx);
        redis_e2e_click(cx, "home-new-connection");

        redis_e2e_click(cx, "new-connection-kind-mongodb");
        assert!(
            cx.debug_bounds("new-connection-toggle-mongo-srv").is_some(),
            "MongoDB SRV 属于地址语义，必须留在连接信息页"
        );

        redis_e2e_click(cx, "new-connection-kind-postgres");
        assert!(
            cx.debug_bounds("connection-pg-default-schema").is_some(),
            "PostgreSQL 默认 schema 属于默认作用域，必须留在连接信息页"
        );

        redis_e2e_click(cx, "new-connection-kind-mysql");
        assert!(
            cx.debug_bounds("connection-url-params").is_none(),
            "MySQL URL 参数不应挤占连接信息首屏"
        );
        assert!(
            cx.debug_bounds("connection-mysql-charset").is_none(),
            "MySQL 字符集不应挤占连接信息首屏"
        );
        redis_e2e_click(cx, "new-connection-tab-2");
        assert!(
            cx.debug_bounds("connection-url-params").is_some(),
            "MySQL URL 参数应位于高级页"
        );
        assert!(
            cx.debug_bounds("connection-mysql-charset").is_some(),
            "MySQL 字符集应位于高级页"
        );

        redis_e2e_click(cx, "new-connection-kind-sqlite");
        assert!(
            cx.debug_bounds("new-connection-tab-2").is_none(),
            "SQLite 没有网络与安全配置，不应显示第三个页签"
        );
        assert!(
            cx.debug_bounds("new-connection-tab-1").is_some(),
            "SQLite 仍应保留高级页（只读开关）"
        );
    }
