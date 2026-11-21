/// 单表写入事务：在**同一条连接**上跨多个批次执行写入，最后提交或回滚。
///
/// 覆盖模式下「整表原子」靠它实现：清空目标表与全部 INSERT 同属一个事务，
/// 任何一批失败都会回滚到写入前状态。参数化执行默认每条语句新开连接，
/// 无法跨批保持事务，因此这里必须持有驱动连接句柄（含 SSH 隧道／建连计划）。
pub struct TableWriteSession {
    inner: TableWriteSessionInner,
    /// 先置 false 再收尾：提交失败时不再尝试回滚，避免掩盖真实错误。
    open: bool,
}

/// MySQL 家族会话：建连计划持有 SSH 隧道与本地转发，必须活到事务收尾。
struct MySqlWriteSession {
    _plan: MySqlConnectionPlan,
    connection: MySqlConnection,
}

enum TableWriteSessionInner {
    Sqlite {
        connection: sqlx::SqliteConnection,
    },
    MySql(Box<MySqlWriteSession>),
    Postgres(PgSession),
    SqlServer(Box<SqlServerClient>),
    Oracle(OracleSession),
    Dameng(DmSession),
}

/// 表写入专用共享 runtime：与查询路径一致，全进程复用一套线程池。
fn table_write_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("创建表写入 tokio runtime 失败")
    })
}

impl TableWriteSession {
    /// 建立新连接并开启事务。`database` / `schema` 为空时按连接档案回退。
    pub fn open(
        config: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<Self> {
        let runtime = table_write_runtime();
        let inner = match config.kind {
            DatabaseKind::Sqlite => {
                let mut connection = runtime.block_on(sqlite_connect(config))?;
                runtime.block_on(async {
                    sqlx::raw_sql("BEGIN IMMEDIATE")
                        .execute(&mut connection)
                        .await
                        .map_err(sqlite_error)?;
                    Ok::<_, Error>(())
                })?;
                TableWriteSessionInner::Sqlite { connection }
            }
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
                let plan = mysql_connection_plan(config)?;
                let mut connection = runtime.block_on(plan.connect())?;
                if let Some(database) = database.filter(|database| !database.is_empty()) {
                    runtime.block_on(async {
                        sqlx::raw_sql(&format!(
                            "USE {}",
                            fluxdb_core::MySqlDialect.quote_identifier(database)
                        ))
                        .execute(&mut connection)
                        .await
                        .map_err(mysql_error)?;
                        sqlx::raw_sql("START TRANSACTION")
                            .execute(&mut connection)
                            .await
                            .map_err(mysql_error)?;
                        Ok::<_, Error>(())
                    })?;
                } else {
                    runtime.block_on(async {
                        sqlx::raw_sql("START TRANSACTION")
                            .execute(&mut connection)
                            .await
                            .map_err(mysql_error)?;
                        Ok::<_, Error>(())
                    })?;
                }
                TableWriteSessionInner::MySql(Box::new(MySqlWriteSession {
                    _plan: plan,
                    connection,
                }))
            }
            DatabaseKind::Postgres | DatabaseKind::Kingbase => {
                let database = pg_request_database(config, database);
                let session = runtime.block_on(pg_connect(config, &database))?;
                runtime.block_on(async {
                    session.client.batch_execute("BEGIN").await.map_err(pg_error)
                })?;
                TableWriteSessionInner::Postgres(session)
            }
            DatabaseKind::SqlServer => {
                let database = sqlserver_request_database(config, database)?;
                let mut client = sqlserver_connect_database(config, Some(&database))?;
                sqlserver_runtime().block_on(sqlserver_exec_transaction(
                    &mut client,
                    "BEGIN TRANSACTION",
                ))?;
                TableWriteSessionInner::SqlServer(Box::new(client))
            }
            DatabaseKind::Oracle => {
                // OCI 在第一条 DML 时隐式开启事务，无需显式 BEGIN。
                TableWriteSessionInner::Oracle(oracle_connect_with_schema(config, schema)?)
            }
            DatabaseKind::Dameng => {
                let mut client = dm_connect(config)?;
                client.begin().map_err(|error| {
                    dm_error("开启达梦事务失败", error)
                })?;
                TableWriteSessionInner::Dameng(client)
            }
            DatabaseKind::Redis | DatabaseKind::MongoDb => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "当前连接类型不支持表级事务",
                ));
            }
        };
        Ok(Self { inner, open: true })
    }

    /// 在事务连接上执行一条参数化 DML，返回驱动报告的影响行数。
    pub fn execute(
        &mut self,
        statement: &str,
        parameters: &[fluxdb_core::SqlParameter],
    ) -> fluxdb_core::Result<u64> {
        if !self.open {
            return Err(Error::new(
                ErrorKind::Internal,
                "表写入事务已结束，不能再执行语句",
            ));
        }
        match &mut self.inner {
            TableWriteSessionInner::Sqlite { connection } => {
                table_write_runtime().block_on(async {
                    let mut query = sqlx::query(statement);
                    for parameter in parameters {
                        query = sqlite_bind_parameter(query, parameter)?;
                    }
                    query
                        .execute(&mut *connection)
                        .await
                        .map(|outcome| outcome.rows_affected())
                        .map_err(sqlite_error)
                })
            }
            TableWriteSessionInner::MySql(session) => {
                let connection = &mut session.connection;
                table_write_runtime().block_on(async {
                    let mut query = sqlx::query(statement);
                    for parameter in parameters {
                        query = mysql_bind_parameter(query, parameter)?;
                    }
                    query
                        .execute(&mut *connection)
                        .await
                        .map(|outcome| outcome.rows_affected())
                        .map_err(mysql_error)
                })
            }
            TableWriteSessionInner::Postgres(session) => {
                table_write_runtime().block_on(async {
                    // 与参数化查询入口一致：先 prepare 拿到占位符类型，再强制转换绑定。
                    let prepared = session.client.prepare(statement).await.map_err(pg_error)?;
                    let expected = prepared.params();
                    if expected.len() != parameters.len() {
                        return Err(Error::new(
                            ErrorKind::Query,
                            format!(
                                "参数个数不匹配：SQL 期望 {} 个，实际传入 {} 个",
                                expected.len(),
                                parameters.len()
                            ),
                        ));
                    }
                    let mut bound = Vec::with_capacity(parameters.len());
                    for (parameter, ty) in parameters.iter().zip(expected) {
                        bound.push(PgBound::coerce(parameter, ty).map_err(|reason| {
                            Error::new(
                                ErrorKind::Query,
                                format!("参数与占位符类型 {} 不兼容：{reason}", ty.name()),
                            )
                        })?);
                    }
                    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                        bound.iter().map(PgBound::as_tosql).collect();
                    session
                        .client
                        .execute_raw(&prepared, refs.iter().copied())
                        .await
                        .map_err(pg_error)
                })
            }
            TableWriteSessionInner::SqlServer(client) => {
                sqlserver_runtime().block_on(async {
                    let mut query = tiberius::Query::new(statement);
                    for parameter in parameters {
                        bind_sqlserver_parameter(&mut query, parameter);
                    }
                    query
                        .execute(client)
                        .await
                        .map(|result| result.total())
                        .map_err(sqlserver_driver_error)
                })
            }
            TableWriteSessionInner::Oracle(connection) => {
                let binds = parameters
                    .iter()
                    .map(oracle_bind_parameter)
                    .collect::<fluxdb_core::Result<Vec<_>>>()?;
                let outcome = oracle_execute_statement(
                    connection,
                    statement,
                    &binds,
                    Pagination::default(),
                    false,
                    "",
                    &|| false,
                )?;
                Ok(match outcome {
                    OracleStatementOutcome::Affected(affected, _) => affected,
                    OracleStatementOutcome::Rows(_, returned) => returned,
                    OracleStatementOutcome::Cancelled => 0,
                })
            }
            TableWriteSessionInner::Dameng(client) => {
                let params = dm_sql_parameters(parameters)?;
                let result = dm_execute(client, statement, &params)?;
                Ok(match result {
                    ExecuteResult::Affected(affected) | ExecuteResult::Call { affected, .. } => {
                        affected
                    }
                    ExecuteResult::Query { rows, .. } => rows.len() as u64,
                })
            }
        }
    }

    /// 提交事务；失败时返回错误且不再尝试回滚。
    pub fn commit(mut self) -> fluxdb_core::Result<()> {
        self.open = false;
        let runtime = table_write_runtime();
        match &mut self.inner {
            TableWriteSessionInner::Sqlite { connection } => runtime.block_on(async {
                sqlx::raw_sql("COMMIT")
                    .execute(&mut *connection)
                    .await
                    .map_err(sqlite_error)?;
                Ok(())
            }),
            TableWriteSessionInner::MySql(session) => runtime.block_on(async {
                let connection = &mut session.connection;
                sqlx::raw_sql("COMMIT")
                    .execute(&mut *connection)
                    .await
                    .map_err(mysql_error)?;
                Ok(())
            }),
            TableWriteSessionInner::Postgres(session) => runtime.block_on(async {
                session
                    .client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(pg_error)
            }),
            TableWriteSessionInner::SqlServer(client) => {
                // 连接随后随会话 Drop 关闭，不再走 sqlserver_close 的消费式收尾。
                sqlserver_runtime().block_on(async {
                    sqlserver_exec_transaction(client, "COMMIT TRANSACTION").await
                })
            }
            TableWriteSessionInner::Oracle(connection) => {
                connection
                    .commit()
                    .map_err(|error| oracle_error("Oracle 提交事务失败", error))
            }
            TableWriteSessionInner::Dameng(client) => {
                let committed = client
                    .commit()
                    .map_err(|error| dm_error("达梦提交事务失败", error));
                // 提交成功或失败都关闭连接，避免事务停留在不确定状态。
                client.close();
                committed
            }
        }
    }

    /// 回滚事务；连接随后丢弃。失败只记录日志，调用方已经处在错误路径上。
    fn rollback(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        let runtime = table_write_runtime();
        match &mut self.inner {
            TableWriteSessionInner::Sqlite { connection } => {
                let _ = runtime.block_on(async {
                    sqlx::raw_sql("ROLLBACK")
                        .execute(&mut *connection)
                        .await
                        .map_err(sqlite_error)
                });
            }
            TableWriteSessionInner::MySql(session) => {
                let connection = &mut session.connection;
                let _ = runtime.block_on(async {
                    sqlx::raw_sql("ROLLBACK")
                        .execute(&mut *connection)
                        .await
                        .map_err(mysql_error)
                });
            }
            TableWriteSessionInner::Postgres(session) => {
                let _ = runtime.block_on(async {
                    session
                        .client
                        .batch_execute("ROLLBACK")
                        .await
                        .map_err(pg_error)
                });
            }
            TableWriteSessionInner::SqlServer(client) => {
                let _ = sqlserver_runtime().block_on(async {
                    sqlserver_exec_transaction(client, "ROLLBACK TRANSACTION").await
                });
            }
            TableWriteSessionInner::Oracle(connection) => {
                if let Err(error) = connection.rollback() {
                    tracing::warn!(
                        target: "fluxdb_connectors",
                        message = %error,
                        "Oracle 表级事务回滚失败，连接将丢弃"
                    );
                }
            }
            TableWriteSessionInner::Dameng(client) => {
                if let Err(error) = client.rollback() {
                    tracing::warn!(
                        target: "fluxdb_connectors",
                        message = %error,
                        "达梦表级事务回滚失败，连接将丢弃"
                    );
                }
                client.close();
            }
        }
    }
}

impl Drop for TableWriteSession {
    fn drop(&mut self) {
        // 未显式提交的事务在这里兜底回滚：中途 `?` 提前返回也不会留下半张表。
        self.rollback();
    }
}

/// 达梦绑定参数转换；与参数化查询入口保持完全一致的类型映射。
fn dm_sql_parameters(
    parameters: &[fluxdb_core::SqlParameter],
) -> fluxdb_core::Result<Vec<dm_pure::Param>> {
    parameters
        .iter()
        .map(|parameter| {
            Ok(match parameter {
                fluxdb_core::SqlParameter::Null => dm_pure::Param::Null,
                fluxdb_core::SqlParameter::Bool(value) => dm_pure::Param::Bool(*value),
                fluxdb_core::SqlParameter::I64(value)
                | fluxdb_core::SqlParameter::U64(value)
                | fluxdb_core::SqlParameter::Decimal(value) => value
                    .parse::<i64>()
                    .map(dm_pure::Param::Int)
                    .unwrap_or_else(|_| dm_pure::Param::Str(value.clone())),
                fluxdb_core::SqlParameter::F64(value) => dm_pure::Param::Float(*value),
                fluxdb_core::SqlParameter::Text(value) => dm_pure::Param::Str(value.clone()),
                fluxdb_core::SqlParameter::Bytes(value) => dm_pure::Param::Bytes(value.clone()),
            })
        })
        .collect()
}
