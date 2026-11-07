## 新功能

- 新增人大金仓 KingbaseES 数据源（PG 兼容模式）：复用 PostgreSQL 连接器、wire protocol、`pg_catalog` 与双引号方言，支持连接测试、库 / Schema 与对象浏览、SQL 查询与补全、数据查看与编辑、表结构查看与设计，以及用户与权限入口。
- 连接表单复用 PostgreSQL 档案，默认 `127.0.0.1:54321` / `system` / `test`，支持 TLS、SSH 隧道、代理、Schema 作用域、超时与 `postgres://` 连接串导入；侧边栏、设置页与连接类型清单加入 KingbaseES。
- 只读模式、AI 页面上下文与受控执行、筛选排序、导入导出、建表与 ER 图等能力按 PostgreSQL 家族同步接入。

## 改进

- PostgreSQL 相关分支统一通过 `is_postgres_family()` 判断，新增 PG 系数据库时不再逐处补 `DatabaseKind::Postgres`，避免方言、补全、过滤或 AI 策略漏接。
- 设置页内置连接器清单与连接类型栏底部计数更新为 11 种。

## 修复

- 修复 KingbaseES 打开“表属性 → DDL”时 panic：Kingbase 的 `pg_sequence` 把 `seqstart` / `seqincrement` / `seqmin` / `seqmax` 声明为 `int2`（值实际是 int8），按 `i64` 解码失败；查询改为显式 `::bigint`，PostgreSQL 行为不变。
- 修复连接类型栏底部仍显示“10 种内置驱动”的计数错误。

## 已知边界

- 仅支持 PG 兼容模式，Oracle 兼容模式不在支持范围内。
- 暂不支持原生备份 / 恢复；需要时请在命令行使用 `sys_dump` / `ksql`。备份入口会明确提示“该数据库尚不支持备份与恢复”。
- 不提供 `psql` / `ksql` 等外部工具入口。
- 用户与权限页沿用 PostgreSQL 实现，尚未在 KingbaseES 上单独回归。
- 表属性 DDL 覆盖序列、约束、索引、触发器、分区与视图；如遇未覆盖的 Kingbase 专有语法，请附失败语句反馈。
