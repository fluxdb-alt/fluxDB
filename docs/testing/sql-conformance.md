# SQL 引擎一致性测试

`crates/fluxdb-connectors/tests/sql_conformance.rs` 提供共享场景，`crates/fluxdb-connectors/tests/sql_conformance/adapters.rs` 放各引擎的连接配置与 fixture。两者用同一套断言验证各 SQL 连接器。目标是发现“连接和查询能跑通”之外的行为差异，不替代已有单测、历史缺陷回归和各引擎专项 live 测试。

## 覆盖范围

当前共享场景：

- 连接、结果列顺序、整数 / 文本 / DECIMAL / NULL / Unicode 往返。
- 分页、升序降序、等值筛选和 `IS NULL` 筛选。
- 对象树、补全列、主键索引、DDL 与表设计元数据。
- 增删改提交，以及同一批次失败后整体回滚。
- 只读入口拒绝写入且不留下副作用。
- 执行前取消不发送写入，也不报告成功。

SQLite 使用临时文件库，默认运行。PostgreSQL、MySQL、MariaDB、TiDB、SQL Server、DM8、Kingbase 和 Oracle 只在设置对应环境变量后运行，测试只创建并清理自己的唯一表或临时库。

```sh
FLUXDB_PROFILE=dev cargo test --locked -p fluxdb-connectors --test sql_conformance

# 本地 Podman / Docker
./scripts/sql-conformance.sh sqlite
./scripts/sql-conformance.sh postgres
./scripts/sql-conformance.sh mysql
./scripts/sql-conformance.sh mariadb
./scripts/sql-conformance.sh tidb
./scripts/sql-conformance.sh mssql
./scripts/sql-conformance.sh dm
FLUXDB_CONFORMANCE_KB_IMAGE=docker.io/huas/kingbase:v008r006c009b0014_single_arm_v1 \
  ./scripts/sql-conformance.sh kingbase
./scripts/sql-conformance.sh oracle
./scripts/sql-conformance.sh all

# 远程主机；脚本启动容器并建立 SSH 隧道
FLUXDB_CONFORMANCE_SSH=root@example.com \
  ./scripts/sql-conformance.sh postgres

FLUXDB_PROFILE=dev \
FLUXDB_PG_SMOKE=127.0.0.1:5432:postgres:postgres \
cargo test --locked -p fluxdb-connectors --test sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_MYSQL_TARGET=127.0.0.1:3306:root:mysql \
cargo test --locked -p fluxdb-connectors --test sql_conformance mysql_sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_MARIADB_SMOKE=127.0.0.1:3306:root:mariadb \
cargo test --locked -p fluxdb-connectors --test sql_conformance mariadb_sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_TIDB_TARGET=127.0.0.1:4000:root: \
cargo test --locked -p fluxdb-connectors --test sql_conformance tidb_sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_MSSQL_SMOKE=127.0.0.1:1433:sa:password:master \
cargo test --locked -p fluxdb-connectors --test sql_conformance sqlserver_sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_DM_SMOKE=127.0.0.1:5236:SYSDBA:SYSDBA001 \
cargo test --locked -p fluxdb-connectors --test sql_conformance dameng_sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_ORACLE_SMOKE=127.0.0.1:1521:FREEPDB1:fluxdb:fluxdb \
cargo test --locked -p fluxdb-connectors --test sql_conformance oracle_sql_conformance -- --ignored --nocapture

FLUXDB_PROFILE=dev \
FLUXDB_KB_SMOKE=127.0.0.1:54321:system:kingbase:test \
cargo test --locked -p fluxdb-connectors --test sql_conformance kingbase_sql_conformance -- --ignored --nocapture
```

## 新增数据库

1. 在 `tests/sql_conformance/adapters.rs` 新增目标构造器，提供连接配置、表路径、对象树父节点、建表/临时库和清理 SQL、标准查询及只读写入 SQL。
2. 使用唯一表名，只清理本次创建的对象；真实库测试必须由显式环境变量启用，并标记 `#[ignore]`。
3. 新增一个入口测试调用 `run_suite`。先跑 SQLite 验证共享断言，再跑目标真库。
4. 如果场景失败，优先修正连接器；不要为新引擎复制一套更宽松的断言，也不要默认增加跳过分支。

共享 fixture 固定为：

| 列 | 值 |
| --- | --- |
| `id` | `1` / `2` / `3` |
| `label` | `alpha` / Unicode 与引号 / `nulls` |
| `amount` | `12.5` / `-1.25` / `NULL` |
| `note` | `NULL` / `kept` / `NULL` |

列名和顺序也是契约。新引擎若必须使用大写列名，应在建表时引用小写标识符，而不是放宽共享断言。
