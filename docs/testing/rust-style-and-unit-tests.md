# Rust 风格与单元测试

目标是降低修改与阅读成本，不是让函数更短、测试数量更多或抽象层更多。本文配合根目录 [AGENTS.md](../../AGENTS.md) 使用，不要求一次性重构历史实现；验证范围以根文件“测试与验收”为准。

## 1. 依据与适用边界

以下资料于 2026-10-17 查阅。Rust 官方资料提供语言惯例和工具建议；开源项目提供可参考的工程实践，不代表所有项目必须采用同一种目录、框架或函数大小。

| 资料 | 采用的原则 | 不照搬的部分 |
| --- | --- | --- |
| [Rust Style Guide 原则](https://doc.rust-lang.org/style-guide/principles.html) | 优先可读性、可扫描性，兼顾 diff 与工具一致性；格式交给 rustfmt | 不能从格式指南推导出函数行数、单次调用不可抽取等架构禁令 |
| [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) | 惯用命名、转换/所有权语义、清晰错误与契约、最小惊讶 | 主要面向库 API，不要求内部小函数全部公开或写长篇 rustdoc；敏感对象不盲目派生序列化/调试能力 |
| [The Rust Book：测试编写](https://doc.rust-lang.org/book/ch11-01-writing-tests.html) | 用断言检查结果，支持返回 `Result` 的测试；panic 测试明确期望 | 不把 `is_err()` 当作所有错误场景的充分断言 |
| [The Rust Book：测试组织](https://doc.rust-lang.org/book/ch11-03-test-organization.html) | 单测就近、`#[cfg(test)]`、可测私有实现；集成测试通过公开 API | 不为了集成测试导出内部状态，也不要求每个私有函数各有一个测试 |
| [Clippy 官方用法](https://doc.rust-lang.org/clippy/usage.html) | 默认 lint 作为检查起点，有理由时局部抑制 | 不整组启用 `restriction`；`pedantic` 属于有主观取舍的检查，不直接变成全仓强制规则 |
| [Cargo：编写测试](https://doc.crates.io/contrib/tests/writing.html) | 按功能组织场景，验证命令结果、输出及文件变化，人工审查快照变化 | Cargo 的 fixture、专用宏和支持 crate 有其规模与场景；本项目不因此另造夹具框架或照搬其 CLI “UI 测试”术语 |
| [Tokio：测试贡献规范](https://github.com/tokio-rs/tokio/blob/master/docs/contributing/pull-requests.md#tests)、[通道测试实例](https://github.com/tokio-rs/tokio/blob/master/tokio/tests/sync_mpsc.rs) | 行为变化附回归测试，按能力组织场景，显式验证发送、接收、关闭、背压等状态 | 不照搬 Tokio 的运行时、测试宏、feature 组合或工程专用格式命令 |

下文的函数抽取、目录与断言取舍是本仓库结合这些原则形成的约定，不声称 Rust 官方规定了固定大小或唯一写法。

## 2. 小函数：按职责抽取，不按行数拆碎

决定是否抽取时先问：**函数名是否表达一个独立契约，并让调用处更容易理解？**

| 值得抽取 | 通常保持就地 |
| --- | --- |
| 带边界规则的解析、校验、转换、风险判定 | 给一个普通字段赋值又包一层没有语义的函数 |
| 被真实调用方重复使用且语义相同的逻辑 | 仅为让调用处“每行一个步骤”拆出一串只用一次的片段 |
| 复杂分支中的独立算法或可命名的领域操作 | 一层调用只转发给另一层，参数、语义与边界均无变化 |
| 数据库事务、锁、资源关闭、IO/平台实现边界 | 为满足“函数必须小于 N 行”而拆散连续可读的流程 |
| 维护对象不变量、trait 能力边界或稳定公开 API | 为了更容易测试才增加生产包装、getter 或 public 接口 |

不能仅因为“只调用一次”就内联：授权校验、格式解析、状态迁移和资源生命周期即使只调用一次，也可能有清晰职责。访问器、构造函数和 trait 方法本来就可能只有一两行，不属于自动清理对象。

反过来，函数名只是 `prepare_step_1`、`build_inner`、`process_data`，需要点进去才能知道做了什么，通常没有减少理解成本。先考虑更好的局部变量、早返回、就地 `match`，再决定是否需要独立函数；不要再加一层“统一入口”来掩盖调用链碎片。

本仓库已有 `parse_profile` 将显式值校验与 `instance_profile` 的环境读取分开：前者有独立的格式契约，也能不修改全局环境地测试。这类拆分有实际价值，不因为函数较小或调用方少而机械合并。Controller 中真实的状态访问、存储装配和应用层能力边界也不按“只有一次转发”机械删除。

### 其他 Rust 风格取舍

- 维护状态或对象不变量的方法放 `impl`；与对象无关的纯运算放所属模块，不为了“面向对象”添加 `&self`。
- 优先复用标准转换和集合能力；具体类型足够时不增加泛型/trait，不为缩短函数、减少参数个数或方便测试引入 manager、builder、宏和转发层。
- `&str`、`&Path`、切片表达只读借用，`String`/`PathBuf` 等表达拥有权；不是禁止传具体类型，而是不要求不必要的拥有权或分配。
- 可恢复错误沿用项目错误体系与 `?`，保留错误类别、来源和上下文，不用 `.ok()`、`unwrap_or_default` 或忽略返回值掩盖应报告的失败。只有契约确实允许默认值时才使用兜底；生产代码不对外部输入/IO 直接 `unwrap`/`expect`，确有已证明的不变量时在局部说明，测试中的 `expect` 可用于清晰报告失败。
- 不为绕过借用检查滥用 `clone`、`Arc<Mutex<_>>` 或全局状态，也不为消除合理复制牺牲可读性；按契约选择 `Option`、`Result`、enum 与 newtype，不以魔法值或含糊布尔参数编码多种业务状态。
- 短表达式可以直接返回；普通循环通常优于需要解释才能理解的多层组合子。不要为消除几行重复引入宏。
- 格式交给 `cargo fmt --all`。Clippy 警告先判断是否真实改善正确性/可读性；局部有理由的例外应可解释，不用整文件 `allow` 逃避处理。

### trait 与能力边界

数据库、平台、存储有多种实现或需要替换外部依赖时，优先复用已有 trait；确有需要再定义职责单一的小接口。接口归使用能力的领域/应用层，具体实现留在实现层，不让抽象归属于偶然的首个调用页面。

编译期确定实现时优先泛型或 `impl Trait`，运行时需要异构实现时使用 `dyn Trait`。简单逻辑、单一实现和没有替换需求的代码保留具体类型，不为假想扩展引入接口。

### 命名与注释

- 名称表达对象、操作和必要约束，不按首次调用页面、功能或临时需求命名；确有业务/平台限制时保留限定，避免泛化名称掩盖真实限制。避免用 `handle`、`process`、`utils` 掩盖具体职责。
- 场景入口可用业务名，底层可复用逻辑按操作与对象命名；先搜索等价能力，不因调用场景不同复制实现或增加仅转发的包装。
- 转换区分 `as_`、`to_`、`into_`，常规访问器不无故加 `get_`；不为套规范批量修改历史 API。
- 注释说明契约、前置条件、边界、副作用与设计原因，不逐句翻译代码；场景示例明确标为示例，只有实现确实依赖某场景时才写“仅用于”。
- 提交前用另一个符合契约的调用方检查名称与注释是否仍准确；若不准确，判断是命名过窄还是实现专用再修正。同步修正本次暴露的误导描述，不顺手批量重命名。

## 3. 单测组织：一个行为，不是一种方法一个测试

### 放置位置

- 普通模块：在被测实现附近的 `#[cfg(test)] mod tests` 中测试；子模块可以访问父模块私有项，通常不需要改变生产可见性。
- `include!` 拼接的文件：先看实际作用域。多个文件都在根作用域定义 `mod tests` 会冲突，应采用 `instance_profile_tests` 等职责名，而不是把所有测试塞入 crate 入口。
- 测试增多：按被测职责拆分，例如解析/校验或状态迁移，而不是 `tests_1.rs`、`misc_tests.rs`。移动时确认路径、模块和测试发现结果，不顺手调整生产模块结构。
- `tests/`：通过公开 API 验证跨模块协作；每个顶层集成测试文件通常是独立测试 crate，不为每个小用例新建一个顶层文件。确有跨文件共享辅助代码时可放 `tests/common/mod.rs`，不默认建立全局工具层。
- bin 私有 View 的无头测试放在 crate 内部，以 `#[cfg(test)]` 接入并复用生产装配，不为集成测试导出私有 View。模块单测、集成测试和用户旅程要标清范围，不能靠文件名宣称 E2E。

### 用例形状

“准备 → 执行 → 断言”保持可见即可，不强制为三行代码写三段注释。一个行为可能需要多个断言，例如失败后既返回鉴权错误，又没有提交数据库写入；不强制一个测试只有一个断言。

- 新增前先查已有覆盖；已有测试足以检出本次回归时只执行/必要时更新，不为每个小改动增加一个同义用例。不追求测试数量或行覆盖率增长，也不枚举没有独立风险的输入笛卡尔积。
- 用例名描述条件和结果，例如 `empty_profile_uses_default_instance`，避免 `test_profile`、`works` 或工单号作为唯一语义。
- 简单同构输入输出使用本地 case 数组与循环，失败信息包含输入或 case 名；有独立准备/清理、不同状态机或错误语义时拆测试，便于筛选和诊断。
- 准备最小必要输入，关键值直接写在测试体中。辅助函数只复用真正重复且有复杂度的准备/清理，不隐藏关键动作或本用例断言。
- 相同复杂断言可提取语义明确的 `assert_*`，但不为一两个 `assert_eq!` 增加间接层；少量可读重复优于参数繁多的“万能断言”。
- 缺陷回归从最小复现出发。能够在修复前运行时先确认它确实失败；无法验证旧版本时写明，不声称已完成红绿验证。

控制存量规模时优先合并同契约的重复准备、同构等价输入，保留真正不同的边界、故障模式与历史缺陷。删除用例需说明其已失效、重复或由等价覆盖替代的依据，不以“数量多/运行慢”为理由删除独有安全、数据完整性或平台验证。慢/不稳定测试应排查根因、分层与隔离，不能直接 `ignore` 后不管。

### 断言内容

- 成功时核对关键返回值/状态；只在数量、存在性确为契约时单独检查长度或 `is_some()`。
- 失败时核对稳定错误类别/错误码及必要副作用；不只检查“任何错误都行”，也不锁死非契约的完整中文文案、时间戳、临时路径。
- 预期值独立列出，不调用同一生产算法生成期望值。集合只有在顺序不属于契约时才排序归一化。
- 优先 `assert_eq!` 获取左右值差异；`assert!` 与 `matches!` 用于布尔/模式语义。需要中间失败上下文时使用 `expect`；多步准备也可以让测试返回 `Result` 并用 `?`。
- panic 只有在明确的 panic 契约中才是成功结果，使用 `#[should_panic(expected = "...")]` 避免其他位置意外 panic 误判成功；更重要的类型/业务约束用正常断言表达。
- 不吞掉错误或 panic，不在依赖缺失时直接 `return` 假装通过；支持显式跳过的独立编排必须记录跳过状态及原因。

## 4. 示例：直接测试既有纯解析，不修改全局环境

下面示例用于放在 `crates/fluxdb-storage/src/parts/instance_profile.rs` 内，与现有 `parse_profile` 同作用域。它是写法示例，尚未加入生产源码；不创建额外夹具，也不修改 `FLUXDB_PROFILE`。实际修改该能力时再评估所需覆盖。

```rust
#[cfg(test)]
mod instance_profile_tests {
    use super::*;

    #[test]
    fn empty_profile_uses_default_instance() {
        let cases = [None, Some(OsStr::new("")), Some(OsStr::new("  "))];

        for raw in cases {
            let profile = parse_profile(raw).expect("empty profile is valid");
            assert_eq!(profile, None, "raw={raw:?}");
        }
    }

    #[test]
    fn profile_trims_surrounding_whitespace() {
        let profile = parse_profile(Some(OsStr::new(" dev-_1 "))).expect("ASCII profile is valid");

        assert_eq!(profile.as_deref(), Some("dev-_1"));
    }

    #[test]
    fn profile_rejects_path_separators() {
        for raw in ["../prod", r"dev\prod"] {
            let error =
                parse_profile(Some(OsStr::new(raw))).expect_err("path separators must be rejected");

            assert_eq!(error.kind, ErrorKind::Internal, "raw={raw:?}");
        }
    }
}
```

这里的 `ErrorKind::Internal` 取自当前实现，示例不改变错误体系。测试名体现行为；同一空值规则用表驱动，不同规则单独成例；直接验证解析结果，不为测试新增生产接口。

## 5. 隔离、异步与执行

### 单测与集成环境分开

纯逻辑单测应可独立、并行运行，不要求启动 Podman 或准备已有 dev 配置。优先测试显式输入，不在并行测试中修改环境变量、当前目录、全局时钟或其他共享状态；测试环境变量读取本身时用独立子进程隔离，而不是给并行测试加一层未覆盖其他调用方的锁。

依赖存储、文件或数据库的测试标明层级；使用已有 `FLUXDB_PROFILE=dev` 和按需 Podman，不因此自制一套 fake 存储或夹具系统。dev 是持久环境，不是全新沙箱；共享写入串行且仅操作本次创建的数据，仍需核对外部连接、显式路径和系统副作用。

异步测试沿用已有执行器，用通道、同步点或可观察状态确认进度；验证成功、取消、超时和收尾时不靠固定 sleep。等待有实际适用的上限，后台任务和线程均有退出路径。Tokio 示例中的调度断言值得借鉴，但不因此在本项目引入 Tokio 测试运行时、Loom 等额外依赖。

### 命令与结果

按根目录 [AGENTS.md 的测试与验收约定](../../AGENTS.md#测试与验收)选择最小充分范围。开发时跑目标用例，Rust 修改收尾补格式化、受影响包的 check/Clippy；不在每次编辑后重复执行所有命令。以下命令是按条件选用的示例，**不是必须依次跑完的清单**；`<crate>`、`<module_path>`、`<binary>` 是待替换占位符：

```sh
cargo fmt --all
cargo check --locked -p <crate>
cargo clippy --locked -p <crate> --all-targets

# 默认：首次使用/修改筛选条件时核对目标，再运行受影响模块
FLUXDB_PROFILE=dev cargo test --locked -p <crate> --lib <module_path> -- --list
FLUXDB_PROFILE=dev cargo test --locked -p <crate> --lib <module_path>

# 仅当共享行为或影响范围需要时，扩大到整个 crate 单测
FLUXDB_PROFILE=dev cargo test --locked -p <crate> --lib

# bin-only crate 的内部单测不能用 --lib
FLUXDB_PROFILE=dev cargo test --locked -p <crate> --bin <binary> <module_path>

# 修改了可执行的公开 rustdoc 示例时另行检查
FLUXDB_PROFILE=dev cargo test --locked -p <crate> --doc
```

上述环境变量前缀是 macOS/Linux shell 写法；Windows 在专用 PowerShell 终端先设置 `$env:FLUXDB_PROFILE = "dev"`，再运行不带前缀的 Cargo 命令。`--lib` 不包含集成测试或 doctest；涉及持久化写入的套件另外按隔离规则串行运行，而不是让所有纯逻辑测试默认串行。

筛选通常是子串匹配，需要单个用例时使用 `--list` 得到完整测试路径，再在 libtest 参数中加 `--exact`；不要因过滤器不准顺手跑整个 workspace。每次执行后检查实际用例数，零用例应修正筛选；确无现有覆盖则按风险评估是否补测试，不让绿色退出码掩盖缺口。

只有影响共享契约、跨模块依赖、关键基础设施或无法可靠圈定范围等情况才扩大回归。局部模块测试通过后不固定追加全 crate；全 crate 通过也不固定追加 workspace。用户旅程/跨层链路需要时再选 E2E，不要求把纯逻辑用例从 UI 重测一遍。已通过结果仅在相关代码/依赖/feature 及环境未受后续改动影响时复用。

### SSH 隧道端到端

`scripts/ssh-tunnel-e2e.sh [mongo|mssql|oracle|dm|all]` 用 Podman 在同一 pod 内启动数据库与 sshd，只发布 sshd 端口，因此数据库在宿主机不可达，只能经跳板机转发：
- `crates/fluxdb-connectors/tests/ssh_tunnel_live.rs` 覆盖档案 → 隧道 → 驱动 → 命令回执，并断言去掉隧道后直连失败、错误 SSH 密码不回退直连。
- `i18n_e2e_tests::mongo_connection_form_connects_through_ssh_tunnel` 在生产视图里点击选择类型、切 SSH 页签、填表并保存，验证 UI 表单与凭据保存链路。
- 脚本自行生成临时密钥和 `known_hosts`（不限定 `-t`，libssh2 可能协商 ECDSA），只清理本次创建的 pod；容器参数缺失时用例直接跳过。
- SQL Server / Oracle / 达梦官方镜像只有 x86_64；arm64 宿主且无 Rosetta 时无法起真库。在 x86_64 Linux 上用 `scripts/ssh-tunnel-e2e.sh mssql|oracle|dm|all` 验证：数据库与 sshd 同 pod、数据库不发布宿主端口，只有经跳板机才能连通；`tests/sqlserver_tunnel_live.rs` 及 Oracle/达梦既有 smoke 用例通过 `FLUXDB_*_SSH` 环境变量启用隧道；达梦用例里直接使用裸驱动的事务/字符集检查在隧道模式下会显式跳过（裸驱动只认原始 host），其余读写都走生产连接器。达梦官方不提供可 pull 镜像，需用 `FLUXDB_E2E_DM_TAR` 指定官方 tar（如 `https://download.dameng.com/eco/dm8/dm8_20241022_x86_rh6_64_single.tar`，镜像名 `dm8_single:dm8_20241022_rev244896_x86_rh6_64`）。宿主机只需 SSH 入站端口，容器互访不出宿主。Oracle 官方仓库在国内可能很慢，可先在本机拉 amd64 镜像，再 `podman save | zstd | ssh 'zstd -d | podman load'` 传入，并用 `FLUXDB_E2E_ORACLE_IMAGE` 指向本地镜像名。

Clippy 的 `--all-targets` 会检查测试等目标，但不执行用例；不默认加 `--all-features` 打开未经确认的平台/可选能力。优先处理本次新增或触及的警告，不为历史警告顺手大改，只在最小范围抑制有依据的 lint 并说明原因；不整组开启 `pedantic`/`restriction`，不批量加 `allow`。在未建立一致工具链和零警告基线前，不擅自将全仓警告升级为错误。

报告记录执行平台、命令、实际通过/失败/忽略/过滤数量。表驱动的多行输入不等于多个 `#[test]`；`--list`、`--no-run`、零用例与 `#[ignore]` 都不是通过证据。

## 6. 提交前检查

- 新 helper 是否有独立语义，还是只让调用链更深？合并后能否仍看清业务/资源边界？
- 测试是否按被测职责归属，是否为测试暴露了本应私有的接口？
- 测试是否有独立于生产算法的预期，并且真正会在目标缺陷出现时失败？
- 准备、动作、断言是否一眼可见，helper 是否隐藏了关键业务步骤或预期？
- 是否优先复用已有覆盖，只补本次风险缺口，而不是机械增加数量或重复输入组合？
- 本次运行范围是否匹配风险，是否无故从目标单测升级到全 crate、workspace 或 E2E？
- 是否验证用例实际被发现并执行，保留有效历史覆盖，避免借整理测试做无关重构？
