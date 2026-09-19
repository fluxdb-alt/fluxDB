/// 只接受可静态分析的 SQL dump 子集；解析失败明确拒绝，不重写目标库名。
fn validate_script(
    path: &Path,
    kind: DatabaseKind,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<()> {
    let mut count = 0;
    for_each_statement(path, kind, cancel, &mut |statement| {
        validate_statement(statement.trim(), kind)?;
        count += 1;
        Ok(())
    })?;
    if count == 0 {
        return Err(task_error("SQL 文件没有可恢复的语句"));
    }
    Ok(())
}

/// 按 SQL 语句切分备份文件，逐条回调（回调内可校验/归类）。语句为去掉普通注释后的可执行文本。
/// 处理 `--`/`#` 注释、`/*!...*/` 可执行注释、反引号/单/双引号，按 `;` 切分；PG COPY 数据块原样跳过。
fn for_each_statement(
    path: &Path,
    kind: DatabaseKind,
    cancel: &AtomicBool,
    f: &mut dyn FnMut(&str) -> fluxdb_core::Result<()>,
) -> fluxdb_core::Result<()> {
    use std::io::BufRead;
    let mut input = std::io::BufReader::new(fs::File::open(path).map_err(io_error)?);
    let mut line = Vec::new();
    let mut statement = String::new();
    let mut quote = None;
    let mut block = false;
    let mut executable = false;
    let mut escaped = false;
    let mut copy_data = false;
    loop {
        canceled(cancel)?;
        line.clear();
        // 有界读取，过长语句不吞掉内存，也不冒险拆分后执行。
        let n = (&mut input)
            .take(16 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .map_err(io_error)?;
        if n == 0 {
            break;
        }
        if n > 16 * 1024 * 1024 {
            return Err(task_error("SQL 行超过 16 MiB，暂不支持自动恢复"));
        }
        let text =
            std::str::from_utf8(&line).map_err(|_| task_error("SQL 恢复只支持 UTF-8 文本"))?;
        if copy_data {
            // COPY 数据保持原样交给 psql；其中的分号、反斜杠和引号不是 SQL。
            if text.trim_end_matches(['\r', '\n']) == "\\." {
                copy_data = false;
            }
            continue;
        }
        if quote.is_none() && !block && text.trim_start().starts_with('\\') {
            let cmd = text.split_whitespace().next().unwrap_or_default();
            if kind == DatabaseKind::Postgres && matches!(cmd, "\\restrict" | "\\unrestrict") {
                continue;
            }
            return Err(task_error(
                "脚本含客户端元命令/COPY 数据块，当前自动恢复不支持",
            ));
        }
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            let next = chars.get(i + 1).copied();
            if block {
                if ch == '*' && next == Some('/') {
                    block = false;
                    executable = false;
                    statement.push(' ');
                    i += 2;
                    continue;
                }
                if executable {
                    statement.push(ch);
                }
                i += 1;
                continue;
            }
            if let Some(q) = quote {
                statement.push(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    if next == Some(q) {
                        statement.push(q);
                        i += 1;
                    } else {
                        quote = None;
                    }
                }
            } else if ch == '-' && next == Some('-') || ch == '#' && kind != DatabaseKind::Postgres
            {
                break;
            } else if ch == '/' && next == Some('*') {
                block = true;
                executable = chars.get(i + 2) == Some(&'!');
                i += if executable { 3 } else { 2 };
                if executable {
                    while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                        i += 1;
                    }
                }
                continue;
            } else if matches!(ch, '\'' | '"' | '`') {
                quote = Some(ch);
                statement.push(ch);
            } else if ch == ';' {
                if !statement.trim().is_empty() {
                    f(statement.trim())?;
                    copy_data = kind == DatabaseKind::Postgres
                        && statement
                            .trim_start()
                            .to_ascii_uppercase()
                            .starts_with("COPY ");
                    if copy_data && !chars[i + 1..].iter().all(|c| c.is_whitespace()) {
                        return Err(task_error("COPY 数据必须从下一行开始"));
                    }
                }
                statement.clear();
            } else {
                statement.push(ch);
            }
            i += 1;
        }
        statement.push('\n');
        if statement.len() > 16 * 1024 * 1024 {
            return Err(task_error("SQL 语句超过 16 MiB，暂不支持自动恢复"));
        }
    }
    if copy_data {
        return Err(task_error("COPY 数据块缺少结束标记，备份可能被截断"));
    }
    if quote.is_some() || block {
        return Err(task_error("SQL 备份被截断或包含不支持的引用语法"));
    }
    if !statement.trim().is_empty() {
        f(statement.trim())?;
    }
    Ok(())
}
fn validate_statement(sql: &str, kind: DatabaseKind) -> fluxdb_core::Result<()> {
    use sqlparser::{
        ast::{
            CopySource, CopyTarget, Expr, FunctionArg, FunctionArgExpr, FunctionArguments,
            Statement, Value, visit_expressions, visit_relations,
        },
        dialect::{MySqlDialect, PostgreSqlDialect},
        parser::Parser,
    };
    use std::ops::ControlFlow;
    let dialect: &dyn sqlparser::dialect::Dialect = if kind == DatabaseKind::Postgres {
        &PostgreSqlDialect {}
    } else {
        &MySqlDialect {}
    };
    // mysqldump 的 ALTER TABLE ... DISABLE/ENABLE KEYS 是临时禁用/启用索引，
    // 只影响目标表索引，安全；sqlparser 的 MySqlDialect 解析不了，单独放行。
    let upper = sql.trim().to_ascii_uppercase();
    if upper.starts_with("ALTER TABLE ")
        && (upper.trim_end().ends_with(" DISABLE KEYS")
            || upper.trim_end().ends_with(" ENABLE KEYS"))
    {
        return Ok(());
    }
    let statements = Parser::parse_sql(dialect, sql)
        .map_err(|_| task_error("SQL 含无法可靠解析的语句，自动恢复已停止；请人工检查脚本"))?;
    if statements.len() != 1 {
        return Err(task_error("恢复语句边界不明确"));
    }
    for statement in &statements {
        let rendered = statement.to_string().to_ascii_uppercase();
        let copy = matches!(statement, Statement::Copy {
            source: CopySource::Table { .. }, to: false, target: CopyTarget::Stdin,
            options, legacy_options, ..
        } if kind == DatabaseKind::Postgres && options.is_empty() && legacy_options.is_empty());
        let allowed = [
            "CREATE SCHEMA ",
            "CREATE TABLE ",
            "CREATE VIEW ",
            "CREATE SEQUENCE ",
            "CREATE INDEX ",
            "CREATE UNIQUE INDEX ",
            "ALTER TABLE ",
            "ALTER SEQUENCE ",
            "INSERT INTO ",
            "DROP TABLE IF EXISTS ",
            "DROP VIEW IF EXISTS ",
            "SET ",
            // mysqldump 标准锁表语句：只锁目标表，安全。
            "LOCK TABLE ",
            "LOCK TABLES ",
            "UNLOCK TABLES",
        ];
        let sequence = rendered.starts_with("SELECT PG_CATALOG.SETVAL(")
            || rendered.starts_with("SELECT PG_CATALOG.SET_CONFIG('SEARCH_PATH', '', FALSE)");
        if !allowed.iter().any(|p| rendered.starts_with(p)) && !sequence && !copy {
            return Err(task_error(
                "脚本包含当前不支持的对象、数据库切换或动态 SQL；请人工检查",
            ));
        }
        // SET 必须为会话设置；不允许修改实例状态或改变后续解析的字符串语义。
        if rendered.starts_with("SET ")
            && (rendered.contains("GLOBAL")
                || rendered.contains("GTID")
                || rendered.contains("PERSIST")
                || rendered.contains("ROLE")
                || rendered.contains("AUTHORIZATION"))
        {
            return Err(task_error("脚本包含实例级或身份设置，禁止自动恢复"));
        }
        if rendered.contains("DEFINER")
            || rendered.contains("INTO OUTFILE")
            || rendered.contains("INTO DUMPFILE")
            || rendered.contains("DIRECTORY")
            || rendered.contains("TABLESPACE")
            || rendered.contains(" AS SELECT ")
        {
            return Err(task_error("脚本包含无法安全限定范围的对象定义"));
        }
        if kind == DatabaseKind::TiDb
            && (rendered.contains("FOREIGN KEY") || rendered.starts_with("CREATE SEQUENCE"))
        {
            return Err(task_error(
                "此 TiDB 恢复包含需要版本相关检查的外键/序列，当前不支持自动恢复",
            ));
        }
        let relations = visit_relations(statement, |name| {
            if name.0.len() > if kind == DatabaseKind::Postgres { 2 } else { 1 } {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        if relations.is_break() {
            return Err(task_error(
                "脚本包含显式数据库限定名，不能保证只恢复到选定目标",
            ));
        }
        // 收集首个被拦的函数名，报错时点名，方便用户定位是哪张表/默认值触发。
        let blocked_function = std::cell::RefCell::new(None);
        let expressions = visit_expressions(statement, |expr| {
            if let Expr::Function(function) = expr {
                let name = function.name.to_string().to_ascii_lowercase();
                // MySQL 前缀索引 `KEY idx (col(255))` 会被解析成函数 `col(255)`。
                // 参数全为纯数字字面量且名字是单段标识符时按列前缀对待，不是真正的函数调用。
                // 双段名（如 otherdb.func(1)）不当作索引前缀，避免跨库函数被放行。
                let prefix_args = match &function.args {
                    FunctionArguments::List(list) => Some(&list.args),
                    _ => None,
                };
                let is_simple_name = function.name.0.len() == 1;
                let index_prefix = is_simple_name
                    && prefix_args.is_some_and(|args| {
                        !args.is_empty()
                            && args.iter().all(|a| {
                                matches!(
                                    a,
                                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Value(
                                        sqlparser::ast::ValueWithSpan {
                                            value: Value::Number(..),
                                            ..
                                        }
                                    )))
                                )
                            })
                    });
                // 白名单：只放行只读/会话内、无跨库读写的函数。MySQL 额外放行常用
                // 内建函数（uuid/随机/日期/字符串/数值聚合等）；Postgres 保持最小集。
                let mut allowed_functions: Vec<&str> =
                    vec!["nextval", "now", "current_timestamp"];
                if kind == DatabaseKind::MySql || kind == DatabaseKind::TiDb {
                    allowed_functions.extend([
                        "pg_catalog.setval",
                        "pg_catalog.set_config",
                        "uuid",
                        "uuid_short",
                        "current_date",
                        "current_time",
                        "curdate",
                        "curtime",
                        "sysdate",
                        "localtime",
                        "localtimestamp",
                        "unix_timestamp",
                        "from_unixtime",
                        "date_format",
                        "date",
                        "time",
                        "year",
                        "month",
                        "day",
                        "hour",
                        "minute",
                        "second",
                        "datediff",
                        "date_add",
                        "date_sub",
                        "last_day",
                        "dayofweek",
                        "dayofmonth",
                        "dayofyear",
                        "coalesce",
                        "ifnull",
                        "nullif",
                        "if",
                        "concat",
                        "concat_ws",
                        "greatest",
                        "least",
                        "length",
                        "char_length",
                        "character_length",
                        "octet_length",
                        "bit_length",
                        "upper",
                        "lower",
                        "ucase",
                        "lcase",
                        "trim",
                        "ltrim",
                        "rtrim",
                        "replace",
                        "substring",
                        "substr",
                        "left",
                        "right",
                        "mid",
                        "lpad",
                        "rpad",
                        "repeat",
                        "reverse",
                        "locate",
                        "instr",
                        "find_in_set",
                        "ascii",
                        "ord",
                        "hex",
                        "unhex",
                        "round",
                        "abs",
                        "floor",
                        "ceil",
                        "ceiling",
                        "mod",
                        "sign",
                        "sqrt",
                        "pow",
                        "power",
                        "exp",
                        "ln",
                        "log",
                        "log2",
                        "log10",
                        "pi",
                        "truncate",
                        "rand",
                        "cast",
                        "convert",
                        "version",
                        "database",
                        "schema",
                        "user",
                        "current_user",
                        "session_user",
                        "system_user",
                    ]);
                } else {
                    allowed_functions.extend([
                        "pg_catalog.setval",
                        "pg_catalog.set_config",
                        // PG 常用只读/无副作用函数：uuid 生成、当前值与序列。
                        "gen_random_uuid",
                        "uuid_generate_v4",
                        "uuid_generate_v1",
                        "uuid_nil",
                        "uuid_ns_dns",
                        "uuid_ns_url",
                        "uuid_ns_oid",
                        "uuid_ns_x500",
                        "current_date",
                        "current_time",
                        "localtime",
                        "localtimestamp",
                    ]);
                }
                if !index_prefix && !allowed_functions.contains(&name.as_str()) {
                    if blocked_function.borrow().is_none() {
                        *blocked_function.borrow_mut() = Some(name);
                    }
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        });
        if expressions.is_break() {
            return Err(task_error(format!(
                "脚本含暂不能确认作用范围的函数调用「{}」，无法保证只恢复到选定目标",
                blocked_function.borrow().clone().unwrap_or_default()
            )));
        }
    }
    Ok(())
}
#[cfg(test)]
mod script_tests {
    use super::*;
    #[test]
    fn rejects_cross_database_and_dynamic_statements() {
        for sql in [
            "USE production",
            "DROP DATABASE production",
            "INSERT INTO production.t VALUES (1)",
            "CALL p()",
            "SELECT dangerous()",
        ] {
            assert!(
                validate_statement(sql, DatabaseKind::MySql).is_err(),
                "{sql}"
            );
        }
        assert!(
            validate_statement(
                "CREATE TABLE t (id INT PRIMARY KEY, name TEXT)",
                DatabaseKind::MySql
            )
            .is_ok()
        );
        assert!(
            validate_statement(
                "INSERT INTO t VALUES (1, '中文; NULL')",
                DatabaseKind::MySql
            )
            .is_ok()
        );
    }
    /// MySQL 前缀索引 `col(255)` 会被解析成函数，不得误判为作用范围可疑的函数调用。
    #[test]
    fn accepts_mysql_prefix_index() {
        assert!(
            validate_statement(
                "CREATE TABLE t (id INT PRIMARY KEY, dfs_id VARCHAR(768), \
                 KEY idx_dfs_id (dfs_id(255)) USING BTREE)",
                DatabaseKind::MySql
            )
            .is_ok()
        );
    }
    /// mysqldump 标准语句：锁表、以及 sqlparser 解析不了的 DISABLE/ENABLE KEYS，须放行。
    #[test]
    fn accepts_mysqldump_standard_statements() {
        for sql in [
            "LOCK TABLES `t` WRITE",
            "UNLOCK TABLES",
            "ALTER TABLE `t` DISABLE KEYS",
            "ALTER TABLE `t` ENABLE KEYS",
        ] {
            assert!(
                validate_statement(sql, DatabaseKind::MySql).is_ok(),
                "{sql}"
            );
        }
    }
    /// PG 常用安全函数（uuid 生成、日期）应放行。
    #[test]
    fn accepts_common_safe_postgres_functions() {
        for sql in [
            "CREATE TABLE t (id uuid DEFAULT gen_random_uuid(), b timestamptz DEFAULT now())",
            "CREATE TABLE t (id uuid DEFAULT uuid_generate_v4())",
        ] {
            assert!(
                validate_statement(sql, DatabaseKind::Postgres).is_ok(),
                "{sql}"
            );
        }
    }
    /// 常用安全函数（uuid/日期/字符串/数值）应放行，未知函数仍拦。
    #[test]
    fn accepts_common_safe_mysql_functions() {
        for sql in [
            "CREATE TABLE t (id INT, k VARCHAR(36) DEFAULT (uuid()))",
            "CREATE TABLE t (a INT DEFAULT (rand()*100), b TIMESTAMP DEFAULT (current_timestamp))",
            "INSERT INTO t VALUES (uuid(), concat('a','b'))",
            "INSERT INTO t VALUES (greatest(1,2), coalesce(null, 0))",
            "INSERT INTO t VALUES (now(), date_format('2026-01-01','%Y'))",
        ] {
            assert!(
                validate_statement(sql, DatabaseKind::MySql).is_ok(),
                "{sql}"
            );
        }
        // 非白名单自定义函数（无参，走不到索引前缀分支）仍要拦。
        assert!(
            validate_statement(
                "INSERT INTO t VALUES (my_custom_fn())",
                DatabaseKind::MySql
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    fn check(text: &str) -> fluxdb_core::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "fluxdb-copy-{}-{}.sql",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, text).unwrap();
        let result = validate_script(&path, DatabaseKind::Postgres, &AtomicBool::new(false));
        fs::remove_file(path).unwrap();
        result
    }
    #[test]
    fn postgres_copy_data_is_not_sql_and_requires_terminator() {
        assert!(
            check(
                "CREATE TABLE public.t (v text);\nCOPY public.t (v) FROM stdin;\n中文;'\n\\N\n\\.\n"
            )
            .is_ok()
        );
        assert!(check("COPY public.t (v) FROM stdin;\nunfinished\n").is_err());
        assert!(check("COPY public.t (v) FROM stdin;\n\\.\nDROP DATABASE other;\n").is_err());
        assert!(check("COPY public.t FROM '/tmp/input';\n").is_err());
    }
}

#[cfg(test)]
mod pg_dump_fixture_test {
    use super::*;
    #[test]
    fn realistic_pg_dump_copy_fixture() {
        let dump = r"-- PostgreSQL database dump
SET statement_timeout = 0;
SET lock_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
CREATE TABLE public.notes (id integer NOT NULL, body text);
COPY public.notes (id, body) FROM stdin;
1	中文; 'quoted' \\path
2	\N
\.
ALTER TABLE ONLY public.notes ADD CONSTRAINT notes_pkey PRIMARY KEY (id);
-- PostgreSQL database dump complete
";
        let path =
            std::env::temp_dir().join(format!("fluxdb-real-pg-copy-{}.sql", std::process::id()));
        fs::write(&path, dump).unwrap();
        let result = validate_script(&path, DatabaseKind::Postgres, &AtomicBool::new(false));
        fs::remove_file(path).unwrap();
        assert!(result.is_ok(), "{result:?}");
    }
}
