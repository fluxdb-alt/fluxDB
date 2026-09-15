# PostgreSQL 手动验收 · 操作手册

日期：2026-09-12（P0 修复后）。配套既有 [集中人工验收清单](2026-09-12-postgresql-manual-acceptance.md)（按功能编号 A–G）；
本手册按**操作顺序**写，含准备数据、细化步骤、每步断言与预期，覆盖：
- 既有清单 A–G 的桌面交互项；
- P0 修复（commit `f4facb6`）新增的手动行为：**查询会话保持**、**建 schema 建到目标库**、
  **连接生命周期释放**、**进程内 SSH 隧道（非阻塞桥）**。

> 每个有 `⚠️` 的步骤：先在亮色主题做完，再切暗色主题重复一遍确认无泄漏/叠影。
>
> **测试断言**列是「看到什么算通过」；不满足即记失败，别跳过继续。

---

## 0. 环境准备（开测前）

测试库（PG 16.15，127.0.0.1:5432 / postgres / secret / postgres）。SSH 跳板机（127.0.0.1:2222 / `tunnel` / `sshpass123`）。

### 0.1 准备一个多 schema + 混合命名库

在 PG 里建两个测试库，供跨 schema、大写/中文名、SSH 场景复用：

```sql
-- 连 postgres 库执行
CREATE DATABASE fluxdb_manual;
\c fluxdb_manual
CREATE SCHEMA tenant_a;
CREATE SCHEMA tenant_b;

-- 两个 schema 各有同名表，验证跨 schema 不串
CREATE TABLE tenant_a.orders (id serial PRIMARY KEY, amount numeric(10,2), note text);
INSERT INTO tenant_a.orders (amount, note) VALUES (12.50, 'abc'), (0.05, 'xyz');
CREATE TABLE tenant_b.orders (id serial PRIMARY KEY, amount numeric(10,2), note text);
INSERT INTO tenant_b.orders (amount, note) VALUES (100.00, 'B库');

-- 大写 + 中文名对象（验证 unquoted 折叠 / quoted 保留）
CREATE TABLE "OrderItems" (id int PRIMARY KEY);
CREATE TABLE "订单" (id int PRIMARY KEY, 名称 text);
CREATE VIEW sales_v AS SELECT id, amount FROM tenant_a.orders;
```

### 0.2 准备 nextval/serial 依赖（表操作用）

`fluxdb_manual` 里 `tenant_a.orders` 已带 serial 主键——复制表/插行时观察序列是否独立。

---

## A. 启动 + 连接对话框（T19）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| A1 | `cargo run -p fluxdb-desktop` | 窗口进入事件循环，无 panic |
| A2 | 新建连接 → 类型下拉选 PostgreSQL | PG 卡片可选；基础/TLS/SSH/高级四页签对 PG 开放 |
| A3 | 填 host `127.0.0.1`、port `5432`、维护库 `postgres`、user `postgres`、密码 `secret` → 保存并连接 | 测试成功；测试期间「测试」按钮 disabled + loading |
| A4 | 侧栏出现 PG 连接，展开 auto 展开库 | 连上后自动显示 schema（public 等） |
| A5 | 编辑该连接 → TLS/SSH/超时回填 | 表单值 = 档案值 |
| A6 | 复制该连接 → 用新密码改副本 → 测试 | 副本用独立密码；原连接不受影响 |
| A7 | 删除该连接 | 弹确认；Keychain 条目清理；对象树不残留 |

> A7 的「释放」（P0-4）：删除/断开后，可用 `docker exec fluxdb-t09-pg psql -U postgres -c "SELECT count(*) FROM pg_stat_activity WHERE application_name='FluxDB';"` 对比——断开前有 N 个 FluxDB 连接，断开后应回落到 0（**不再挂到 30 分钟**）。

---

## B. 对象树 / schema（T20 + P0-2）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| B1 | 连接 `fluxdb_manual` → 展开 | 出现 `public` / `tenant_a` / `tenant_b` 三个 schema 独立成行 |
| B2 | 展开 `tenant_a`、`tenant_b` | 各自列 own `orders` 表；`tenant_a.orders` 与 `tenant_b.orders` 互不串列 |
| B3 | `tenant_a` 节点右键 → 新建 schema → 名 `手动_甲` | 弹框只问名称；成功后在 `fluxdb_manual` 下出现该 schema |
| B4 ⚠️ | （P0-2 验证）B3 建的 schema 刷新后仍在 `tenant_a` 所属**同一库**下 | 它出现在 `fluxdb_manual`，**不是** `postgres` 维护库；切回 postgres 库不应凭空多出 `手动_甲` |
| B5 | 数据库 `fluxdb_manual` 右键 → 新建数据库 → 名 `fluxdb_manual2` | 弹框含 ENCODING + Locale(LC) + Owner + 模板；成功后新库出现 |
| B6 | 数据库右键 → 删除 `fluxdb_manual2` | 有活动连接时服务端 RESTRICT 拒绝并提示；成功才清节点 |
| B7 | schema 行右键 → 新建查询 | 打开的是**该 schema 作用域**的查询标签；`current_schema()` 返回该 schema |

失败判定：B3 建到别的库 = P0-2 未生效；B7 打开查询后 `current_schema()` 不是选中 schema = schema 作用域丢了。

---

## C. 查询会话保持（T13 + P0-1）★核心回归

P0-1 之前：`BEGIN` 与 `COMMIT` 落在不同连接，事务必丢。**本组是本次修复的手测核心。**

在动手前先把 C.0 读懂——本组几乎每步都要「新建标签」「切换标签」，不懂这个概念后面全乱。

### C.0 先搞懂：查询标签 / 新建 / 执行 / 切换 / 看结果

**「查询标签」= 一个 SQL 查询编辑器页签（tab）。** 每点一次「新建查询」就多开一个标签；每个标签是一套独立的查询会话（独立数据库连接）。同一连接可同时开多个标签，标签之间**不**共享事务、**不**共享临时表、**不**共享 search_path——这正是本组要验证的东西。

**怎么新建一个查询标签**（任选其一，都可）：
- 顶部工具栏「新建查询」按钮（图标 `+`，文案 `SQL`）
- 快捷键：macOS `Cmd+Y` / 其它 `Ctrl+Y`
- 左侧连接 / 数据库 / schema 节点右键 → 菜单「新建查询」（在 schema 上右键新建的标签会自动切到该 schema 作用域）

**怎么切换标签**：窗口顶部一排标签（类似浏览器页签），点哪个进哪个，当前激活的高亮。**同一个标签里**你反复输入、反复执行的语句，全跑在这**同一个会话**上。

**怎么执行**（光标落在哪条语句就执行哪条）：
- 工具栏「执行」按钮（▶）
- 快捷键：macOS `Cmd+Enter` / 其它 `Ctrl+Enter`

**怎么看结果**：编辑器下方结果区分两块——上面的「语句状态 / summaries」（显示 `OK` / 影响行数 / 报错信息），下面的「结果网格」（SELECT 返回的数据行）。看 count 就看查出来的那个数字。

> 本组核心手法：**C2 输入、C3 执行必须在同一个标签**；**C4 必须另开一个新标签**；**C5 再切回 C2 那个标签**。全程不要只开一个标签，要来回开、来回切。

### C.0.1 回到一条语句执行状态行

编辑器里，每条刚执行过的语句，其首行会带状态底色（running/success/failure）。执行成功呈绿/常色，失败呈红。哪条红就修哪条，别全选整块执行。

---

| 步骤 | 具体操作（点哪 / 输啥） | 预期结果 |
|---|---|---|
| C1 | 顶部「新建查询」开第一个标签；确认该标签连接的是 `fluxdb_manual`（标签标题或顶部库名下拉显示）。输入 `SELECT 1;` → 执行 | summaries 显示 OK；结果网格返回 1 行，值为 `1`（说明这条连接能正常跑） |
| C2 | **同一个标签**输入下面整段 → 执行（光标放语句上即可，多语句会逐条跑）：`BEGIN; INSERT INTO tenant_a.orders(amount,note) VALUES (1.23,'会话保持');` | 无报错；summaries 显示 OK（BEGIN 和 INSERT 都成功） |
| C3 | **同一标签**输入 `SELECT count(*) FROM tenant_a.orders;` → 执行 | 返回行数 = C1 之前的行数 **+1**（本会话能看到自己还没提交的那行） |
| C4 | **另开一个新标签**（顶部新建查询 / 快捷键），切到它，输入相同 `SELECT count(*) FROM tenant_a.orders;` → 执行 | 返回**不含**刚插入的那行（数字回到 C1 前原值）——因为新标签是另一个会话，看不到 C2 未提交的事务 |
| C5 | **切回 C2 那个标签**，输入 `COMMIT;` → 执行 | **COMMIT 成功**。若报 `no transaction in progress`（没有进行中的事务）→ **P0-1 未生效，直接判失败** |
| C6 | 任意标签（比如 C4 那个）输入 `SELECT count(*) FROM tenant_a.orders;` → 执行 | 该行**已可见**（比原值 +1）——证明 C5 提交成功、对其它连接也生效 |
| C7 | **同一个标签**依次输入并执行：`CREATE TEMP TABLE tmp_t(x int);` → `INSERT INTO tmp_t VALUES (1);` → `SELECT count(*) FROM tmp_t;` | 三条全在同一会话：temp 表存活、count=1。随后**另开新标签**输入 `SELECT * FROM tmp_t;` → 报 `relation "tmp_t" does not exist`（临时表只有建它的那个会话独享） |
| C8 | C2 那个标签输入 `SET search_path TO tenant_b;` 执行 → 再输入 `SELECT current_schema();` 执行 | current_schema 返回 `tenant_b`——同一会话里改 search_path 后，之前未提交的事务仍然还在（可把 C5 的 COMMIT 挪到这步之后判断，验证「切 schema 事务仍在」） |
| C9 | 关闭 C2 那个标签（点标签上的 ×） | 弹出提示（如有未提交/脏内容），确认后无残留连接；（可选）`docker exec fluxdb-t09-pg psql -U postgres -c "SELECT count(*) FROM pg_stat_activity WHERE application_name='FluxDB';"` 对比关闭前后连接数回落 |

**失败判定**：C5 的 COMMIT 报 `no transaction in progress`、C3 看不到自己刚插的行、C7 临时表换标签后仍可见——**任一出现即会话机制坏了**，P0-1 回归失败。

---

## D. 查询取消 / 事务恢复（T13）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| D1 | 输入 `SELECT pg_sleep(120);` → 执行 → 立即点「停止」 | loading 有反馈；约 1 秒内停止（真实 CancelToken，非 30s 硬超时） |
| D2 | 停止后语句标记「已取消：结果待核实」 | 不谎报成功、不误报断连 |
| D3 | 输入 `BEGIN; SELECT 1/0;` → 执行 | 失败，会话进入 aborted，提示需回滚 |
| D4 | 继续输入 `SELECT 1;` → 执行 | 被跳过/标注「需 ROLLBACK」，不自动回滚 |
| D5 | 输入 `ROLLBACK;` → 执行 → 再 `SELECT 1;` | ROLLBACK 后恢复正常执行 |

---

## E. 补全 / 结果编辑 / 历史（T14/T15）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| E1 | 新建查询，输入 `SELECT * FROM tenant_` 触发补全 | 出 `tenant_a`、`tenant_b` schema 候选；选中后插入 `schema.` |
| E2 | 跨 schema：输入 `SELECT * FROM tenant_b.` | 出 `tenant_b` 内的表/视图（本手册数据为 `orders`），不混入 `tenant_a.orders` 或 `public."OrderItems"`；也**不出现小写关键字候选 `order`** |
| E3 | 输入 `SELECT * FROM Or` 并选中 `OrderItems` 补全项 | 插入结果为 `SELECT * FROM "OrderItems"`；执行成功，不折叠成小写 |
| E4 | 执行 `SELECT * FROM tenant_a.orders` → 结果表编辑改成某行 amount → 提交 | 单表简单 SELECT 可编辑并提交成功 |
| E5 | 执行 JOIN/聚合查询 → 试图编辑结果 | 只读，编辑被禁用 |
| E6 | 执行插入带 BEGIN 的 DML → 查历史 | 历史含该语句与「未提交/已提交/已回滚」状态；`SET PASSWORD`/`CREATE ROLE` 等敏感语句**不进历史** |
| E7 | 输入 `SELECT * FROM tenant_a.order`，待补全浮层出现后把选中项停在 `orders` | 候选中只有 `orders`（无 `order` 关键字、无 `public."OrderItems"`）；右侧详情显示 `tenant_a.orders` 的列清单（`id`/`amount`/`note`）。首次悬停会拉一次列，再次悬停/切走切回应立即出现（索引已写回） |

---

## F. 数据 / 类型 / 建表（T09–T12、T16–T18）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| F1 | 打开 `tenant_a.orders` 数据页 | numeric 显示 `12.50`/`0.05`（精确，非 12.5 丢位） |
| F2 | 加一个 bytea/jsonb 列：`ALTER TABLE tenant_a.orders ADD COLUMN j jsonb, ADD COLUMN b bytea;` → 刷新 | jsonb 以 Json 显示；bytea 显示 BinarySummary 摘要（非整块刷屏） |
| F3 | 打开 bytea 单元格详情 → 上传/下载/Hex | 完整字节读取 |
| F4 | data 页排序/过滤（`amount > 10`、LIKE `'%'`、BETWEEN） | 结果正确；空 IN / 非法列报错不静默 |
| F5 | 分页翻页、改排序 | 同值行分页稳定（主键 tie-breaker） |
| F6 | `tenant_a` 下新建表向导 | PG 类型清单（jsonb/uuid/timestamptz…）；identity 默认 `GENERATED BY DEFAULT AS IDENTITY`；schema 限定 |
| F7 | 设计表改列类型/加唯一索引 → 保存 | 只发差异动作，预览=执行；外部 DDL 后保存被拒 |
| F8 | 复制 `tenant_a.orders` → `orders_copy` | 副本序列独立：往副本插行不推进源序列，两表主键不冲突 |
| F9 | 重命名/清空/删除表 | schema 限定；清空默认 CONTINUE IDENTITY，勾选 RESTART 才重置 |

---

## G. 角色 / 权限（T26/T27 + P0-5）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| G1 | PG 连接右键「用户与权限」 | 打开「用户与角色」工作台（非 MySQL user@host）；顶部有「保存 / 刷新 / 删除」，左侧是角色列表 + 搜索筛选，右侧页签为「常规 / 高级 / 成员关系 / 权限 / SQL 预览」；无 host/plugin/资源限制字段 |
| G2 | 左侧列表上方点「+」→ 新建 `test_r1`：填角色名，保持「允许登录」开启，「密码操作」选「设置新密码」并输入/确认密码 → 顶部「保存」；再次点「+」新建 `test_g1`，关闭「允许登录」→「保存」 | 每次提示「已保存全部更改」；列表刷新后能看到两个角色，`test_r1` 次行为「可登录」，`test_g1` 次行为「不可登录」 |
| G3 | 选中 `test_r1` →「成员关系」；在「添加成员关系」下拉选 `test_g1` →「授予成员关系」→ 顶部「保存」 | 保存前列出待保存的 `+ GRANT test_g1 TO test_r1...`；保存并刷新后「所属角色」表显示 `test_g1`，PG16+ 可看到 ADMIN / INHERIT / SET |
| G4 | 选中 `test_r1` →「常规」：改密码 →「保存」；再改名 `test_r1b` →「保存」，最后改回 `test_r1` →「保存」 | 每次提示「已保存全部更改」；改名期间成员关系跟随同一角色；新密码可登录 `test_r1`；改回后列表仍显示 `test_r1` |
| G5 | 选中 `test_r1` →「常规」→ 关闭「允许登录」→「保存」 | 提示保存成功；刷新后角色次行为「不可登录」，开关仍反映 false |
| G6 | 选中 `test_r1` →「权限」，先等数据库下拉完成目标列表加载；数据库选 `fluxdb_manual`，种类选「表」，Schema 选 `tenant_a`，对象下拉选 `orders` | 首次进入显示「加载目标列表…」，加载后数据库/schema/对象可选；目标完整后自动读取权限；概览显示属主，ACL 为 NULL 时显示「默认权限：属主全权、其余按对象类型默认，不等同于空授权」 |
| G7 | 在权限表勾选 `SELECT` 的「直接授予」和「可再授权」→「保存」→ 刷新；再取消「可再授权」→「保存」 | 保存前列出 `GRANT ... WITH GRANT OPTION` / `REVOKE GRANT OPTION FOR ...`；刷新后 `SELECT` 先显示「直接授权」且「可再授权=是」，随后保留直接授权但「可再授权=否」 |
| G8 | 取消 `SELECT` 的「直接授予」→「保存」→ 刷新 | 仅移除直接授权；PUBLIC / 成员继承 / 属主等来源仍可能显示「当前有效=是」，来源标注为其他来源，不会误报成可直接撤销的直接授权 |
| G9 ⚠️ | （P0-5 验证）先用查询页执行 `CREATE OR REPLACE FUNCTION public.manual_g9(integer, text) RETURNS text LANGUAGE sql AS $$ SELECT 'g9'::text $$;`。回到「权限」，种类选「函数」，Schema 选 `public`，从对象下拉选择 `public.manual_g9(integer, text)`；确认函数签名**只能来自枚举下拉，没有自由输入签名框**，然后勾选 `EXECUTE` 的「直接授予」→「保存」→ 刷新，最后取消勾选并保存，用 `DROP FUNCTION public.manual_g9(integer, text);` 清理 | 正常签名能读取并完成直接授权/撤销；恶意片段（如 `int) TO postgres; ALTER ROLE ...`）无法作为签名输入或进入待保存计划 |

失败判定：G9 函数签名可自由输入，或恶意签名能进入提交计划 = P0-5 未生效（严重安全问题，立即停）。

---

## H. 备份 / 原生脚本（T24/T25，需主机有 psql/pg_dump）

> 后端子进程 argv 已用容器内工具验证；桌面真实 spawn 需主机带工具。

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| H1 | 设置 → 数据 → 配 pg_dump 路径（或用 PATH） | 工具检测存在 |
| H2 | PG 连接 → 备份 tab → 原生备份 `fluxdb_manual` | pg_dump 生成 .sql（plain+inserts）；进度/取消正常；密码不明文进进程列表 |
| H3 | 用 psql 把备份恢复到干净隔离库 | 表/行/视图/索引/序列/函数齐全；序列位置正确；新增 identity 行不冲突 |
| H4 | 执行含 `COPY ... FROM STDIN` 的 SQL 文件 | 自动识别原生模式走 psql；COPY 数据正确落入；分号不误拆 |
| H5 | 原生脚本中途失败 | ON_ERROR_STOP 停止并提示；勾「继续错误」则继续 |
| H6 | 主机无 psql/pg_dump | 报「启动 psql/pg_dump 失败」清晰错误 |

---

## I. SSH 隧道（T05 + P0-6）★需跳板机

跳板机：`127.0.0.1:2222` / `tunnel` / `sshpass123`。PG 目标：`127.0.0.1:5432`。

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| I1 | 复制 PG 连接，传输层改 SSH：跳板 127.0.0.1:2222、user `tunnel`、密码认证 → 测试 | **测试成功**呈 SSH 隧道拨号成立（P0-6 前会卡到建连超时——进程内桥死锁） |
| I2 | 该 SSH 连接正常展开库、打开数据页、执行查询 | 隧道桥下数据双向流通正常 |
| I3 | 断开该连接 | `pg_stat_activity` 该连接回落；本地不再有残留隧道连接 |
| I4 | SSH 连接下执行原生备份 / 原生脚本 | 走 `ssh -L` 子进程隧道 + PGHOSTADDR；工具可用 |
| I5 | 误填跳板密码（如 `wrong`）→ 测试 | 报 SSH 认证失败，可读中文错误 |

失败判定：I1 测试超时（卡 5s+）或 I2 数据页无响应 = P0-6（SSH 桥）仍未修复。

---

## J. MySQL 回归（跨全部 P0）

| 步骤 | 操作 | 测试断言 |
|---|---|---|
| J1 | 新建/编辑 MySQL 连接测试保存 | 默认值（3306/root/utf8mb4/preferred）不变 |
| J2 | MySQL 建表/建库/查询/补全 | 反引号、AUTO_INCREMENT 行为与接 PG 前一致 |
| J3 | MySQL 对象树叶节点 | 仍 连接→库→分组，无 schema 行 |
| J4 | （间接）确认无 PG 会话释放误伤 MySQL | MySQL 连接断开后其池/连接正常回收 |

---

## 结果记录

每项把 `[ ]` 改 `[x]` 并附一句结果（如 `A4 ✓ 自动展开`）。**失败项必须记录错误原文**，特别是：
- C5 `no transaction in progress` → 会话机制失效（P0-1）
- B4 schema 建到 postgres 维护库 → P0-2 失效
- I1 测试超时 → 进程内 SSH 桥死锁（P0-6）
- G9 函数签名可自由输入，或恶意签名能进入提交计划 → SQL 注入（P0-5，严重）

全部通过后，据此勾选 T19/T20/T21/T22/T24/T25/T27，并登记到 T28 §5 验收表。

## 冒烟命令速查

```bash
# 桌面
cargo run -p fluxdb-desktop

# PG 真库单测（无需桌面）
FLUXDB_PG_SMOKE=127.0.0.1:5432:postgres:secret:postgres \
  cargo test -p fluxdb-connectors pg_live_ -- --test-threads=1

# SSH 隧道双向（回归 P0-6，旧实现失败/新实现通过）
FLUXDB_SSH_SMOKE=127.0.0.1:2222:tunnel:sshpass123 \
  cargo test -p fluxdb-connectors ssh_tunnel_relays -- --nocapture

# MySQL 回归
FLUXDB_MYSQL_SMOKE=127.0.0.1:53306:root:root:app \
  cargo test -p fluxdb-connectors mysql_live_ -- --test-threads=1
```
