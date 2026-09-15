# FluxDB Windows / Linux 适配方案

日期：2026-09-14。仓库基线：`d40b29c`。

本文基于当前源码、`Cargo.lock`、发布脚本，以及本机 Cargo registry 中对应版本依赖的源码和构建说明编写。本次只做方案，不修改程序；尚未在 Windows / Linux 上完成编译和实机验证。下文区分已确认的问题、推荐决策和待验证项。

## 1. 结论与支持范围

继续使用 **Rust + GPUI + gpui-component**，复用现有 core / app / connectors / storage 分层，不需要为跨平台更换 UI 框架或重写数据库功能。

当前依赖实际为 `gpui-pre 0.3.3`、`gpui-pre-platform 0.3.3`、`gpui-component 0.6.0`，不是旧版 `gpui 0.2.x`。该组合已有 Windows 和 Linux 平台实现，但“库有平台后端”不等于“FluxDB 安装后即可正常使用”。工作重点是平台行为、凭据与目录、原生库、外部工具、安装和验收。

建议首版支持矩阵如下，系统下限是产品建议，最终须经 P0 验证后确定：

| 平台 | 首版目标 | 发布格式 | 验收要求 |
| --- | --- | --- | --- |
| macOS | 保持现有 Intel / Apple Silicon | 现有 DMG | 不回归已有启动、凭据和数据 |
| Windows | Windows 11 x64，`x86_64-pc-windows-msvc` | Inno Setup EXE；另提供 ZIP 便于诊断 | 原生 Windows 使用，不依赖 WSL、Rust 或开发工具 |
| Linux | Ubuntu 22.04 / 24.04 x64，`x86_64-unknown-linux-gnu` | 优先 DEB；后续 AppImage | X11 与 Wayland 分别验收 |

Windows ARM64、Linux ARM64、其他发行版、Flatpak / Snap、Windows 10 暂不承诺支持。Windows 10 如有明确需求，单独验证系统 API、GPU 和 ConPTY 下限。Linux 不承诺覆盖全部显卡、远程桌面及无图形环境。

首版应支持已实现的数据库连接、查询、编辑、持久化、导入导出和 SSH 能力。原生命令行备份/终端允许依赖用户安装的客户端，但必须说明所需工具、检测状态及配置方式。Linux 托盘建议作为后续增强，不能因此影响关闭窗口和退出。

## 2. 现状与确定问题

| 位置 | 已确认现状 | 影响与处理优先级 |
| --- | --- | --- |
| `apps/fluxdb-desktop/Cargo.toml` | 使用 `gpui-pre` / `gpui-pre-platform`，不是原包名 `gpui` | 保持整套版本一致，P0 审计实际 feature |
| `crates/fluxdb-storage/src/lib.rs::default_root` | 固定 `$HOME/Library/Application Support/fluxdb`，失败回退相对目录 `.fluxdb` | Windows 通常无 HOME；Linux 路径不规范，P0 |
| 同文件凭据函数 | 非 macOS 读取返回 `None`、写入返回 `Ok(())`、删除为空操作 | 密码未保存却报告成功，重启无法恢复，P0 |
| `main_parts/logging.rs::app_data_dir` | 重复拼接 macOS 路径；无 HOME 回退当前目录 | 配置与日志可能落在不同且不可写位置，P0 |
| `main_parts/app_boot_helpers.rs::app_assets_base_path` | 仅识别 macOS bundle；否则用编译时 `CARGO_MANIFEST_DIR/assets` | Windows/Linux 安装包会引用构建机路径，P0 |
| `main_parts/tray_icon.rs::install_close_to_tray` | 关闭回调始终返回 `false`，只有 macOS 调用 `cx.hide()` | Windows/Linux 关闭按钮可能无效；托盘失败也继续阻止关闭，P0 |
| `tray-icon 0.23.1` | Linux 需要 GTK 事件循环，项目只处理菜单消息，没有对应 GTK 初始化/循环 | 仅装库不能解决托盘运行问题，P0 决策 |
| `main_parts/app_boot.rs` | titlebar 配置包含 macOS 红绿灯位置 | Windows/Linux 窗口装饰、拖动与按钮须适配，P1 |
| 编辑器、终端、表格与 JSON 渲染 | 多处写死 `Menlo`，部分字号/缩进依赖字体假设 | 缺字、对齐、光标和选择区域可能异常，P1 |
| `main_parts/shortcuts.rs` 与编辑器注册 | 应用快捷键已有平台分支，但编辑器折叠等仍直接绑定 `cmd-*` | 复用现有映射并补全遗漏，P1 |
| `main_parts/terminal_component/transport.rs` | 已用 portable-pty；工具路径解析仅检查精确文件名，额外目录只处理 Homebrew | Windows `.exe`、带空格路径及 PATH 行为需验证，P1 |
| `main_parts/menus_dialogs/database_backup.rs` | 存在直接执行数据库 CLI、`ssh` / `sshpass` 的逻辑 | Windows 不应要求 sshpass；需复用 connector 隧道并归位执行职责，P1 |
| `crates/fluxdb-connectors/src/parts/transport/ssh.rs` | known_hosts 默认路径使用 HOME | Windows 用户目录解析错误，不能退化为跳过主机密钥验证，P1 |
| `data_editor_model/export.rs::default_data_export_directory` | 读 `HOME/Downloads`，失败回退 `current_dir()`，再回退临时目录 | Windows 无 HOME 时回退到安装目录/`System32` 等不可写位置，导出静默失败，P1 |
| 导出写入逻辑（同文件 CSV/JSON/XML 分支） | 一律写 UTF-8 且不带 BOM | Windows 中文 Excel 打开 CSV 乱码；需 BOM 选项与换行策略，P1 |
| `logging.rs` | 只有写入，没有大小上限、轮转或保留期 | 长期运行日志无限增长，且日志可能含 SQL 与连接信息，P1 |
| storage SQLite 连接参数 | 未显式设置 `busy_timeout`，也未显式声明 journal 模式 | 多实例或备份并发时出现 `database is locked`，P1 |
| 应用入口 | 无单实例守卫 | Windows 双击快捷方式易开多实例并争抢同一 SQLite，P1 |
| 敏感文件权限 | 配置、SQLite、导出与临时文件未设权限（无 `0o600`/`0o700` 调用） | 多用户机器上凭据库与导出数据可被同机其他账号读取，P1 |
| `.github/workflows/release.yml` | 仅启用 macOS 发布；Windows job 注释，Linux 缺失 | 需原生构建、测试和包验证，P2 |
| `scripts/windows-installer.iss` | 已有按用户安装脚本，但仅复制 EXE 和 assets | 没有证明动态库、资源定位、升级与卸载可用，P2 |

已有 `gpui_platform::application()`、macOS 专属 cocoa 条件依赖、跨平台快捷键定义、portable-pty、共享 SSH 隧道和工具路径设置均应复用。

本表只列主干问题。其余容易被“编译通过、窗口能开”掩盖的跨平台缺陷，见第 12 节补充清单，实施时与本表同等对待。

## 3. 依赖与原生库策略

### 3.1 GPUI / gpui-component

当前已核对的依赖关系：

```text
fluxdb-desktop
├─ gpui-pre 0.3.3
├─ gpui-pre-platform 0.3.3
│  ├─ macOS: gpui-pre-macos
│  ├─ Windows: gpui-pre-windows（Direct3D 11 等 Windows API）
│  └─ Linux: gpui-pre-linux → gpui-pre-wgpu、X11 / Wayland
├─ gpui-component 0.6.0 → gpui-base 0.6.0
│  └─ gpui-pre-platform（启用 x11 / wayland 等 feature）
└─ gpui-fps 0.6.0 → gpui-pre（profiler）
```

`gpui-pre-platform` 自身默认 feature 为空，但当前 `gpui-base 0.6.0` 的非 WASM 依赖显式启用了 `x11`、`wayland`；Cargo feature 合并后，当前项目并非完全未启用 Linux 图形后端。实施时仍建议在 desktop 的 Linux target 依赖中显式声明这两项，避免平台能力依赖组件的间接配置。

处理原则：

1. 保留 `Cargo.lock`，构建使用 `--locked`；新增依赖后有意识更新锁文件并评审差异。固定验证通过的 Rust 工具链，不长期依赖浮动 stable。
2. 用目标平台 `cargo tree -e features` 验证最终 feature，用 `cargo tree -d` 检查重复版本；特别防止同时引入不兼容的 GPUI 类型版本。`windows` 等封装库存在多个版本不一定是错误，按类型交互和链接结果判断。
3. 继续使用 gpui-component 通用控件与已有 AppIcon；平台化不能变成自绘按钮、菜单、弹框的理由。优先检查其 TitleBar 等现成能力，再确定窗口布局。
4. 先验证当前版本，不顺带升级整套 UI。遇到上游问题保留最小复现，评估兼容版本修复；只有确认阻塞时再做范围明确的 patch/fork，并记录退出条件。
5. `gpui-fps` 会启用 profiler；评估改为诊断 feature，默认发布包关闭。它不是跨平台启动的必要功能，不为此替换 UI 技术栈。

### 3.2 依赖清单与决策

| 库/能力 | 当前情况 | 跨平台方案 |
| --- | --- | --- |
| `ssh2 0.9.6` → `libssh2-sys 0.3.2` | 包含 C 编译、zlib；Unix 使用 OpenSSL，Windows 默认可走系统加密后端 | 保留；优先源码编译 libssh2，固定构建来源，Windows 不主动启用 openssl-on-win32 |
| `openssl-sys 0.9.117` | 锁文件中确实存在，不能因数据库使用 rustls 就认为没有 OpenSSL | Linux DEB 优先使用基线系统 OpenSSL 并声明依赖；扫描完整 ELF/DLL/dylib 闭包 |
| `sqlx 0.8.6`、`tokio-postgres 0.7.18`、rustls | MySQL/SQLite 使用 SQLx；PG 使用 tokio-postgres + rustls | 普通连接无需安装 MySQL 或 PostgreSQL 客户端库；继续保留 TLS 校验 |
| `rustls 0.23.41`、`ring 0.17.14` | 当前 ring 路线仍含原生编译 | 配齐目标 C 工具链，核验最终 crypto provider，不能按“全部纯 Rust”打包 |
| `rusqlite 0.32`、SQLx SQLite | storage 已启用 bundled；锁文件中 `libsqlite3-sys` 为 0.30.1 | 保持统一链接版本；检查 workspace 与单 crate 构建的 feature，避免 `links=sqlite3` 冲突；普通使用不依赖 sqlite3 CLI |
| `tree-sitter 0.26.13`、JSON 0.24.8、sequel 0.3.11 | 解析器含 C/C++ 构建 | 保留锁定版本，配齐编译器，验证高亮、解析 ABI 与输入编辑；不要求用户安装 Node/tree-sitter CLI |
| `portable-pty 0.9` | 已有 PTY 接入 | 保留；Windows 验证 ConPTY、Unicode、Ctrl-C、resize 和进程回收，Linux 验证 PTY 与终端环境 |
| `tray-icon 0.23.1` → muda / GTK / appindicator | 即使 `default-features=false`，Linux GTK 依赖仍存在；关闭的是 libxdo 等默认能力 | 首版从 Linux 依赖图排除托盘 crate，不能只跳过函数调用；Windows/macOS 保留 |
| `cocoa 0.26` | 已只在 macOS 编译 | 保持 target 条件依赖，将 Dock 代码放入对应平台职责文件 |
| 目录和系统凭据 | storage 未使用跨平台实现 | 目录优先评估 `dirs`/`directories`，凭据优先评估 `keyring` 并明确各平台 feature；选型验证后锁版本 |

GPUI 也提供凭据 API，但把 `gpui::App` 引入 storage 会破坏当前分层。推荐 storage 使用独立系统凭据后端；除非已有不依赖 GPUI 类型的可复用实现，否则不为减少一个依赖而把持久化搬进 UI。

**构建依赖与运行依赖必须分开记录。** 用户不应安装 Rust、Visual Studio、编译器或开发头文件来运行应用。

### 3.3 Windows 构建与运行库

- 使用原生 Windows MSVC runner，安装 VS C++ Build Tools 和 Windows SDK。当前 `gpui-pre-windows` 的构建脚本包含 release HLSL 预编译路径，会查找 `fxc.exe`；不能只做 debug / cargo check 验证。
- 明确固定 C 工具链与库来源，避免本机 vcpkg、OpenSSL 等环境偶然改变 libssh2 的链接方式。不要在未需要时同时引入 MSYS2/MinGW 工具链。
- 运行时核对 Direct3D 11、字体、IME、系统 API 与 VC/UCRT 依赖。用 `dumpbin /DEPENDENTS` 递归检查 EXE 和实际携带 DLL；必要时用加载跟踪检查延迟加载项。
- 首选常规 MSVC runtime 策略，若产物需要 VC Runtime，就在安装器中按许可携带/安装官方运行库，或采用经验证的 app-local 方案。若改静态 CRT，必须验证所有原生依赖 CRT 一致性，不能直接全局加参数了事。
- 只分发确有需要且允许再分发的第三方 DLL，不复制 Windows 系统 DLL。ZIP 同样要说明或满足运行库前置条件。
- 应用增加 Windows 图标 `.ico`、版本资源和合适的 GUI subsystem；隐藏控制台后仍保留文件日志和启动失败可见反馈。

### 3.4 Linux 构建与运行库

- 首选在 Ubuntu 22.04 基线构建，再测试 24.04；先确认固定 Rust/GPUI 依赖可在该基线构建。若做不到，明确提高系统下限，不在新系统构建后宣称兼容旧 glibc。
- 初始构建环境准备 C/C++ 编译器、pkg-config、按需使用的 clang/CMake，以及 OpenSSL、fontconfig、FreeType、xkbcommon、X11/XCB、Wayland 开发包；精确 apt 包名由 P0 的目标依赖和实际构建日志固化。
- 运行环境核对 X11/Wayland、字体栈、GPU loader/驱动、D-Bus、`xdg-desktop-portal` 及对应桌面 portal 后端。当前 Linux renderer 使用 wgpu；实际后端、驱动版本与软件渲染能力必须实测，不能承诺无 GPU 必然可用。
- 只有开启 Linux 托盘后才额外引入 GTK3 / appindicator 并处理 GTK 事件循环。不要为未发布的托盘功能保留这组运行库。
- DEB 使用 `dpkg-shlibdeps` 生成动态库依赖，并补充动态加载库、portal 和凭据服务等无法自动发现的要求；在干净系统安装依赖闭包。用 `readelf -d`、`ldd` 检查实际产物及 GLIBC/GLIBCXX 版本要求。
- AppImage 在 DEB 稳定后再做，整理 `$ORIGIN`/RPATH、可分发库和 AppRun 路径。不要把 glibc、显卡驱动和宿主桌面服务当作普通库全部打包；AppImage 也不能消除系统兼容边界。
- 对随包分发的 C 库、Rust crates、字体和图标生成许可证清单/SBOM，记录版本与安全更新责任；静态链接并不免除这些义务。

## 4. 平台适配设计

### 4.1 目录与资源

目录策略统一由 storage 定义，启动和日志消费解析结果，不再各自读取 HOME。建议首版保持单个持久化根目录，避免为了规范化同时重构数据库布局：

| 平台 | 持久化根目录 | 默认日志 |
| --- | --- | --- |
| macOS | 保持 `~/Library/Application Support/fluxdb` | 保持根目录 `logs/` |
| Windows | 系统 Known Folder 对应的 `%LOCALAPPDATA%/FluxDB` | 根目录 `logs/` |
| Linux | `$XDG_DATA_HOME/fluxdb`，缺省 `~/.local/share/fluxdb` | `$XDG_STATE_HOME/fluxdb/logs`，缺省 `~/.local/state/fluxdb/logs` |

`config.toml` 首版继续随持久化根目录保存；未来独立迁移到 XDG_CONFIG_HOME 不属于本次必要工作。补全索引暂保持现有布局，避免跨平台适配连带改动缓存协议。

目录创建失败要明确报错，不再静默回退安装目录/当前工作目录。日志目录不可写可以继续启动，但应提供可见诊断。系统目录使用 PathBuf/OsString；覆盖中文、空格、非 ASCII 和不同盘符，避免自行拼接斜杠。SSH 用户目录解析位于 connector 的相应边界，不让 connector 反向依赖 storage。

若非 macOS 曾运行开发版并产生旧目录，迁移必须备份、只在新库为空时执行且可重试。SQLite 当前开启 WAL，不能仅复制主数据库文件；迁移需关闭连接并正确 checkpoint，或用 SQLite backup 能力保证一致快照。失败保留旧数据和明确错误，禁止覆盖已有新库。

资源加载按安装布局解析：macOS 保留 bundle `Contents/Resources/assets`；Windows 使用 EXE 同级 `assets`；Linux DEB 使用安装前缀下 `share/fluxdb/assets`，AppImage 使用自身 AppDir 布局。开发模式才使用 `CARGO_MANIFEST_DIR`。从快捷方式、任意工作目录启动均须成功；发布模式缺资源要明确报错，不能继续依赖构建机目录。

### 4.2 安全凭据存储

在 storage 内抽取小范围凭据接口与平台后端，继续复用 `credential_ref`、SecretRef 和已有密码槽位遍历，不改变连接档案语义：

- macOS：保留现有 service `com.fluxdb.connection` 和 account 命名。首版可保留既有后端；如改原生 API/keyring，需兼容旧条目并验证原密码能读取。
- Windows：使用 Windows Credential Manager 后端。
- Linux：使用 Secret Service 后端，验证 GNOME Keyring 以及提供该接口的 KWallet 环境；“安装了 KDE”不代表凭据接口必定可用。不得用仅会话内核 keyring 冒充重启可恢复的持久化。
- 区分未找到、锁定、服务不可用、权限拒绝与实际写入失败。非 macOS 空实现必须移除；失败不能返回成功提示。
- 凭据后端不可用时，用户可明确选择仅本次会话输入密码，界面显示未持久保存。不能自动写入配置、日志、历史记录或导出文件。
- 保存连接时先确认必需凭据写入成功，再提交对应引用；失败保留原有效配置。系统凭据与 SQLite 不共享事务，需记录可恢复状态/清理新孤立条目，不能误删旧凭据或其他连接的共享条目。
- 系统凭据调用可能阻塞或弹授权，放后台执行并显示 loading/error；单元测试注入内存后端，不访问真实用户密码库。

### 4.3 窗口、托盘、字体与输入

将窗口装饰、托盘和资源定位等放到 desktop 的 `main_parts/platform/`，仅按职责拆分必要文件，不新增庞大平台框架。`main.rs` 保持启动 glue。

Windows/Linux 默认关闭窗口走正常关闭流程，先处理未保存编辑、未提交事务和后台任务，再关闭/退出。仅在用户启用“关闭到托盘”、托盘创建成功且恢复窗口经验证后才拦截关闭。首次 Linux 发行版关闭托盘，同时移除其 Linux 构建依赖；GNOME 无状态托盘扩展时也必须正常工作。

Windows/macOS 托盘分别验证事件循环、打开、退出和恢复焦点。后续 Linux 托盘若保留 tray-icon，需要在创建托盘的同一线程运行 GTK 循环，通过消息通道与 GPUI 交互，并验证退出回收；单纯轮询 MenuEvent 不是完整实现。

字体统一采用平台默认 UI 字体和已验证的等宽字体解析：macOS 保持 Menlo，Windows 评估 Consolas，Linux 检测已安装等宽字体。若需要稳定排版，选许可允许的等宽字体内嵌，并保留 CJK fallback；不得打包 Apple 专有字体。光标/缩进/选择几何从实际字体度量计算，清理 JSON 中固定字符宽度假设。

复用 `shortcuts.rs` 统一主修饰键与显示文案，补全编辑器和 SQL 操作中的裸 `cmd-*`；Windows/Linux 使用 Ctrl，但不能占用终端 Ctrl-C。覆盖 AltGr、Home/End、删除、折叠、格式化、执行、复制粘贴以及自定义快捷键冲突。

输入验收必须覆盖中文 IME 组合输入、候选窗定位、焦点切换、Unicode/emoji、剪贴板多行和 CRLF。分别测试系统 100%/150%/200% 缩放、多屏移动及明暗主题。弹框和菜单保留 Esc、点击外部关闭、内部事件阻止穿透和 loading 状态。

### 4.4 外部工具与 SSH

普通数据库操作通过内嵌 driver 完成；`pg_dump`、`psql`、`mysqldump`、`sqlite3`、`redis-cli` 等外部工具仅供已实现的对应功能使用。工具缺失不能阻止主窗口或普通查询启动。

复用 Settings 中已有工具路径项。工具发现遵循“用户显式路径 → 进程 PATH”，平台补充目录仅在有验证依据时加入；Windows 处理 `.exe` 和可执行文件解析规则，不把任意 `.bat/.cmd` 当普通 EXE 强行执行。始终用参数数组传递路径与参数，不拼 shell 命令；用户路径包含空格、中文、括号仍可使用。

将本次涉及的备份/SQL 文件执行中的外部调用从 UI 移到 app 编排及 connector 执行边界，复用现有 invocation 模型。UI 通过 AppCommand 启动，AppState/AppEvent 表达进度、取消和结果，不新增 UI 直接拼 SQL、开数据库或保存密码。

`database_backup.rs::pg_start_ssh_tunnel` 目前依赖 `sshpass` 密码认证，应复用 `connectors/parts/transport/ssh.rs` 的 libssh2 转发能力，通过 connector 暴露生命周期受控的操作接口，不能把其内部类型全部改成 pub。CLI 连接本地转发端口，operation 持有隧道直到结束/取消。继承证书、known_hosts、密码/私钥口令及超时规则，不因换平台跳过主机密钥检查；不要求 Windows 用户安装 sshpass。

非交互工具执行与 PTY 分开处理：备份使用后台进程，终端继续 portable-pty。取消后 kill 并 wait 回收进程和隧道，Windows 必须验证子进程树是否残留。连接参数、环境变量和 stderr 均需脱敏；使用客户端支持的安全凭据传递方式，必要临时凭据文件设权限并清理。

首版不捆绑所有数据库客户端，提供配置、版本检测和可见错误。`pg_dump` 保留与服务端版本兼容校验；Windows 的 redis-cli 来源需单独验证，不能默认系统已带或要求 WSL 才能运行应用。

PostgreSQL 客户端已按 DBeaver 方案实现自动发现与按需下载（`fluxdb-app/parts/pg_client_tools.rs`，详见 PostgreSQL 详细设计 §11.3）：发现顺序为「设置的客户端目录 → 应用托管下载目录 → 系统标准安装路径 → PATH」，Windows 展开 `%ProgramFiles%\PostgreSQL\<版本>\bin`、Linux 展开 `/usr/lib/postgresql/<版本>/bin`、macOS 展开 Postgres.app 与 /Library/PostgreSQL 及 libpq。Windows/macOS 支持应用内下载官方二进制包（安装到设置的客户端目录，未设置时到 `<应用数据目录>/clients/postgresql/<版本>`），Linux 只给包管理器安装引导。仍待平台验证：Windows 上下载解压后的 DLL 依赖是否齐全、带空格/中文路径的实际执行、以及是否需要读注册表补充发现（当前按标准安装目录展开，未读注册表）。

## 5. 实施顺序与交付物

| 阶段 | 工作与修改位置 | 完成条件 | 参考工作量 |
| --- | --- | --- | --- |
| P0：依赖验证 | Cargo/工具链；Windows 与 Linux 原生 debug/release 构建；验证 GPUI 窗口、Input、Table、portal、图形后端；确认 Linux 托盘裁剪方案 | 两个平台能在图形会话显示和操作窗口；产出构建依赖、运行库和阻塞清单 | 2–4 人日 |
| P1a：可运行基础 | storage 目录/凭据；desktop 资源、窗口关闭、日志 | 安装目录外启动成功；保存连接后重启恢复；密码库失败正确反馈 | 3–5 人日 |
| P1b：功能一致性 | 字体/快捷键/IME；外部工具；SSH；相关 UI 调用归位 | 查询编辑、文件操作、TLS/SSH、备份和终端代表性流程可用 | 4–7 人日 |
| P2：构建发布 | Windows 打包脚本与既有 Inno Setup；Linux DEB；CI、运行库扫描、图标、许可证 | 干净环境安装/升级/卸载通过，候选包可复现 | 3–5 人日 |
| P3：稳定性 | 三平台回归、Linux X11/Wayland、GPU/DPI/IME、修复与文档 | 验收矩阵通过，明确已知限制后发布 | 3–5 人日 |

第 12 节补充项按此归属：单实例与 SQLite 并发、文件权限、默认导出目录与文件名净化归入 P1a；编码、终端子进程、网络差异、时区归入 P1b；日志轮转与崩溃可诊断性归入 P1b 与 P3；分发信任与 Linux 桌面集成归入 P2。这些项目未包含在下列人日估算中，预计额外 3–5 人日。

合计约 **15–26 人日**，假设不遇到严重 GPUI 上游缺陷。它是排期参考，不是交付承诺；P0 结束后根据真实阻塞重新估算。AppImage、Linux 托盘和新增 CPU 架构另算。

每阶段保留 macOS 可运行。涉及超过 1200 行的现有文件，先机械拆出本次职责，再做行为修改，分开提交/说明；优先复用现有 `parts`，不得继续扩张入口文件或把整个 UI 重构纳入适配。

## 6. CI 与发布设计

新增 PR CI 与 tag release 两条路径，不直接把注释的 Windows job 打开就发布。

PR CI 在 macOS、Windows MSVC、Linux GNU 原生 runner 上运行；三个 OS 都检查相应 target，至少一个任务执行格式检查。缓存按 OS、target、工具链和锁文件区分。

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --locked -p fluxdb-core -p fluxdb-editor-core -p fluxdb-editor-language
cargo test --locked -p fluxdb-storage -p fluxdb-app -p fluxdb-connectors
cargo build --locked --release -p fluxdb-desktop
```

上述命令在各自原生环境执行；实施时区分不依赖服务的单测与需要数据库、SSH、密码服务的集成测试。对后者显式启动测试服务并执行对应测试，不能让“没有服务而跳过”被算成通过。UI Rust 修改本地执行 `cargo fmt` / `cargo check`，并确认 `cargo run -p fluxdb-desktop` 能启动。

GUI 冒烟任务需要真实/虚拟图形会话。Linux Xvfb 只覆盖 X11，Wayland 另用 compositor 会话；GPU、portal 和密码服务要单独准备。Windows GUI 测试需要交互桌面；普通 hosted runner 的进程存活不能替代显示/输入验收。CI 自动化与发布前干净 VM/实机验收结合。

发布先生成候选 artifacts，再执行依赖/安装检查，最后统一发布 Release，避免部分平台失败却已对外宣称全平台支持。保留现有 macOS DMG 签名流程；Windows 对 EXE/安装器加入可配置 Authenticode 签名，缺签名时明确候选包状态，不能伪装已签名。

建议文件：

- `.github/workflows/ci.yml`：原生构建与测试矩阵。
- `.github/workflows/release.yml`：扩展三平台候选构建和统一发布阶段。
- `scripts/package-windows.ps1`：build、资源、依赖 staging、Inno Setup、校验。
- `scripts/windows-installer.iss`：补全运行库策略、图标、版本、快捷方式和升级验证。
- `scripts/package-linux.sh`：DEB 目录布局、desktop 文件、图标、依赖和检查；AppImage 后续扩展。
- `docs/`：三平台构建说明、依赖清单、验收记录和已知限制。
- `README.md` / `README.en.md`：仅在验收通过后更新支持徽标和下载说明。

Windows 沿用按用户安装，不要求管理员运行数据库客户端；Linux 安装代码在系统位置、数据在用户位置。升级保留配置、SQLite 数据与凭据；卸载默认保留用户数据，任何删除用户数据的选项必须显式选择。包名包含版本、OS 和架构，并附校验和。

## 7. 验收矩阵与发布门槛

| 领域 | 必测情形 | 通过标准 |
| --- | --- | --- |
| 安装与资源 | 无 Rust/SDK/数据库 CLI 的干净系统；快捷方式、任意工作目录启动；中文用户名/安装路径 | 主窗口、图标、主题、高亮资源完整；没有构建机路径依赖 |
| 存储 | 首启、保存、重启、升级、旧目录迁移、不可写目录 | 配置/连接/历史不丢失；失败可见且不覆盖原数据 |
| 凭据 | DB/SSH/私钥口令/代理密码，新增修改删除；锁库、无服务、拒绝授权 | 成功重启恢复，失败不报保存成功；磁盘配置和日志无明文 |
| 数据库 | MySQL/TiDB、PostgreSQL、SQLite、Redis 等当前已实现路径 | 连接测试、对象加载、查询、编辑、分页、取消及事务行为与 macOS 一致 |
| TLS / SSH | CA 文件含空格/中文；证书/主机名失败；密码/私钥；未知/变更 host key | 正常链路可用，错误链路正确拒绝；无平台专属绕过 |
| 输入和布局 | 中文 IME、emoji、CRLF、复制粘贴、编辑器折叠、Ctrl-C；多 DPI / 多屏 / 明暗主题 | 光标与候选窗位置正确；快捷键、表格和字体度量无明显异常 |
| 窗口 | 关闭、取消关闭、退出、最小化/恢复；无托盘/托盘失败 | 可以退出；未保存编辑/事务有确认；无不可恢复隐藏窗口 |
| 文件与 CLI | 打开/保存对话框、拖放已有入口、工具缺失、显式路径、版本不符、备份恢复、取消 | 有 loading 与可操作错误；无残留进程、隧道或敏感临时文件 |
| Linux 桌面 | Ubuntu 22.04/24.04 的 X11/Wayland，GNOME；KDE 做扩展验证 | 图形、IME、剪贴板、portal 可用；不依赖托盘扩展 |
| GPU | 集显/独显代表设备、虚拟机/远程桌面代表环境 | 支持范围有记录；初始化失败可诊断，不以“编译通过”替代 |
| 生命周期 | 安装、覆盖升级、卸载重装，连续查询和关闭 | 用户数据可恢复，无严重资源泄漏、死锁或后台孤儿进程 |

测试证据记录 OS/桌面/显卡、Rust/依赖版本、commit、包 SHA256、用例结果和失败日志。macOS 至少回归启动、凭据读取、连接查询、备份、关闭到托盘与 DMG 安装。

**发布阻塞项：** 安装后缺 DLL/SO、资源丢失、启动或正常退出失败、密码假保存、配置/数据库损坏、普通查询不可用、关键 IME 输入不可用、TLS/SSH 校验退化。Linux 无托盘、可选 CLI 未安装可作为明确限制，但入口必须给出解释或配置方式。

## 8. 第一轮执行任务

- [ ] 固定工具链与现有锁文件，在 Windows/Linux 获取完整依赖树和 release 构建结果。
- [ ] 验证 Windows SDK/fxc、Linux 图形后端、字体与 portal；记录系统下限。
- [ ] 将 Linux 托盘依赖与引用条件化，修复统一返回 false 的关闭回调。
- [ ] 抽取 storage 目录与凭据职责，修复非 macOS 密码空实现，加入错误路径测试。
- [ ] 修复安装后资源路径及日志目录，验证无源码目录启动。
- [ ] 补全平台字体、窗口装饰、快捷键和中文输入验证。
- [ ] 复用 SSH 隧道，整理 CLI 执行边界与 Windows 进程行为。
- [ ] 决定单实例方案并落地，配套 SQLite 并发参数与陈旧锁处理。
- [ ] 收敛敏感文件权限，修正默认导出目录与文件名净化。
- [ ] 确定导出编码策略与外部 CLI 输出解码策略，补日志轮转与脱敏。
- [ ] 完成 Windows EXE / Linux DEB、运行库依赖和干净系统安装验证。
- [ ] 完成回归矩阵（含第 12.11 节新增必测项），发布候选版，再更新项目支持说明。

## 9. 核对资料

仓库证据以上文列出的 Cargo 文件、源码符号、release workflow 和安装脚本为准。依赖事实来自锁定版本的 Cargo manifest、`gpui-pre-windows/build.rs`、`gpui-pre-linux/src/linux.rs`、`libssh2-sys/build.rs` 及 tray-icon README；外链用于后续查阅，不表示本文已验证未来版本：

- [gpui-pre-platform 0.3.3](https://docs.rs/crate/gpui-pre-platform/0.3.3/source/Cargo.toml)
- [gpui-pre-windows 0.3.3 构建脚本](https://docs.rs/crate/gpui-pre-windows/0.3.3/source/build.rs)
- [gpui-pre-linux 0.3.3](https://docs.rs/crate/gpui-pre-linux/0.3.3/source/Cargo.toml)
- [gpui-base 0.6.0 feature 来源](https://docs.rs/crate/gpui-base/0.6.0/source/Cargo.toml)
- [gpui-component 0.6.0](https://docs.rs/crate/gpui-component/0.6.0/source/Cargo.toml)
- [tray-icon 0.23.1 平台要求](https://docs.rs/crate/tray-icon/0.23.1/source/README.md)
- [libssh2-sys 0.3.2 构建逻辑](https://docs.rs/crate/libssh2-sys/0.3.2/source/build.rs)

## 10. AI 实施约定

本节将方案转为可逐步执行、验收和交接的任务。写入方案不代表现在开始改代码，也不代表授权发布；后续明确要求按方案实施时，按以下默认决策推进，无需对普通文件拆分、接口命名等细节反复确认。第 5 节人日仅表示工作量，不能换算成 AI 运行时长；目标机器可用性、CI 与人工图形验收仍决定交付时间。

### 10.1 固定默认决策，限制无关改动

- 首版按第 1 节矩阵：Windows 11 x64、Ubuntu 22.04/24.04 x64；EXE/ZIP 和 DEB；Linux 暂无托盘。
- 保持 GPUI / gpui-component 路线、连接配置语义和 macOS 数据目录。AppImage、ARM64、换 UI 框架、通用插件系统、全面模块化和整库依赖升级均不在首版范围。
- `dirs`/`directories`、keyring 的具体版本和 feature 是 P0 待验证决策，不凭名称猜 API。阅读真实 manifest/源码后，选满足需求的最小组合并记录原因。
- 修改必须先追踪实际调用链；源码与本文不一致时以当前代码为依据，更新偏差记录。不得根据旧行号盲改，也不把文档中的建议文件名当作必须创建的抽象层。
- 只给复杂逻辑、错误恢复和平台边界补必要测试，不为简单样式或一行映射堆叠测试。遵守仓库 AGENTS.md，不使用 OpenSpec / Superpowers。
- 发现依赖不兼容时，先给出错误证据与最小修复；不能为通过构建删除数据库功能、关闭 TLS/host key 校验、吞错误或给非目标平台补假成功实现。

### 10.2 开始前核对执行环境

第一个实施任务先记录 `git status`、当前 commit、已有用户改动、Rust 版本和可访问的构建机器。不得覆盖用户未提交改动。建议使用独立 `codex/` 分支；不因适配顺手合并其他业务修改。

| 环境 | 必须具备 | 缺失时如何推进 |
| --- | --- | --- |
| macOS | 当前构建环境和图形会话 | 先保留现状与日志，不能宣称无回归 |
| Windows | 原生 MSVC 构建机；另有 Windows 11 交互桌面用于验收 | 可先写构建流程和平台代码；记录“待原生构建/待图形验收” |
| Linux | 基线构建环境；X11/Wayland 会话；D-Bus/portal | 容器可验构建和部分测试，不能替代桌面、IME 或 GPU 验收 |
| 数据库/SSH | 隔离测试实例、测试账号、证书与 host key 场景 | 先执行不依赖服务的测试；集成用例保持未验证 |
| 凭据服务 | 独立测试用户/唯一测试 service-account 命名 | 单测用注入后端；系统集成测试不读写真实连接凭据 |

当前用户没有 Windows/Linux 本机，默认采用第 11 节的 GitHub Actions 与按需远程桌面方案，不以购买实体机器为实施前提。如果 AI 只有 Mac 权限，可以完成代码与 CI 配置，但不能把 Mac 上的交叉 `cargo check` 等同于 Windows/Linux 可用。CI 文件写好不等于 CI 跑过。缺少环境时先完成独立工作，再一次性说明缺哪个环境、用于哪些测试以及如何接续；不凭空填写运行结果。

### 10.3 按任务推进，不一次性重构全部平台

每个任务形成一组可审查 diff 和证据。顺序上的依赖用于阻止过早报完成；缺少机器时允许先完成独立编码，但保留验收未完成状态。

| ID | 前置 | 实施内容与范围 | 必须交付的证据 |
| --- | --- | --- | --- |
| AI-00 | 无 | 复查基线、依赖、原生环境，建立最小 CI 构建任务；P0 所需的托盘条件编译可先做 | 三平台环境记录；原始构建命令/错误；待定库版本和 feature 决策 |
| AI-01 | AI-00 依赖结论 | storage 目录解析、desktop 日志/资源定位 | 中文/空格路径、缺 HOME、不可写目录、发布布局无源码启动；macOS 旧目录不变 |
| AI-02 | AI-01 目录语义 | storage 凭据后端、保存错误传播、会话密码状态 | 读写删、锁库、服务缺失、多槽位部分失败；无假成功或明文落盘 |
| AI-03 | AI-00 | 窗口关闭/托盘条件化、字体、快捷键、IME 与主题 | 三平台关闭和恢复记录；X11/Wayland、DPI、中文输入结果 |
| AI-04 | AI-02；AI-00 原生库结论 | app/connector 工具执行、SSH 隧道复用、PTY 行为 | 直接连接/SSH/CLI 对照；工具缺失与路径；取消后进程/隧道回收 |
| AI-05 | AI-01～04 的发布阻塞项解决 | 原生 release 打包、库扫描、安装升级 | 包校验和、依赖报告、干净环境安装记录，macOS 回归 |
| AI-06 | AI-05 | 完成第 7 节验收、更新 README/已知限制，整理候选发布材料 | 每个平台的验收结论；仍失败/未验证项目；可发布或不可发布的依据 |

第 12 节补充项挂到对应任务：12.1 单实例与并发、12.2 权限、12.4 路径与文件名归 AI-01；12.3 编码、12.5 终端子进程、12.6 网络差异、12.7 时区归 AI-04；12.8 日志与崩溃归 AI-03 与 AI-06；12.9 分发信任归 AI-05。任务完成判定包含 12.11 的对应必测项。

P0 可以使用最小窗口复现隔离 GPUI 问题，但最终验收必须启动真实 FluxDB，不能用 demo 替代主程序。完成 AI-06 不自动执行正式 tag、公开发布或生产数据迁移，遵循实际实施会话中的授权。

### 10.4 必须补齐的失败用例

正常路径已有代码可复用，AI 实施尤其需要验证以下边界，防止用“表面正常”替代正确性：

1. **凭据与配置不是同一个事务。** 更新多个密码槽位时中途失败，以及密码写完但 SQLite 提交失败，都要保留原引用与原密码可用。尤其不能先原地覆盖旧凭据，再声称配置回滚即可恢复；应选择与现有所有权模型兼容的暂存/版本化或可靠恢复策略，测试后再定实现。读取后端报错不能被当作密码不存在并自动清空配置。
2. **目录迁移必须区分确实有数据与理论可能。** AI-00 先盘点旧版可能生成的数据位置；无旧数据不新增复杂迁移框架。需要迁移时验证 WAL、目标目录已有数据、磁盘空间不足和中途失败；全过程使用测试副本。
3. **平台文件语义。** Windows 文件占用时覆盖/rename/delete 与 Unix 不同；核对本次涉及的保存和临时文件逻辑。覆盖盘符、UNC（若当前功能支持）、CRLF、文件对话框取消、空格/中文路径，不能全靠替换 `/` 为 `\\`。
4. **启动失败必须有证据。** 资源缺失、日志不可写、图形初始化失败分别测试；不能新增捕获所有 panic 后返回成功的逻辑。Windows 无控制台的候选包也要能定位错误。
5. **关闭与取消。** 未提交事务、未保存编辑、备份进行中、托盘失败分别验证；取消关闭后原任务/编辑保持有效，确认退出后子进程、PTY 与隧道确实回收。

### 10.5 验收与跨会话交接

实施时新建一个简洁记录文件 `docs/2026-09-14-windows-linux-implementation-status.md`，每个任务结束更新一次，不新增任务管理系统。不要现在预填测试成功。

```text
任务 ID / 状态：未开始、进行中、待验证、通过、阻塞
代码基线：commit；未提交 diff 的范围（如有）
目标环境：OS/版本、架构、桌面会话、工具链
改动：解决什么问题；主要文件；实际采用的依赖版本/feature
验证：完整命令、退出码、测试数量及跳过数量、日志路径或 CI 链接
GUI/安装：操作步骤、观察结果；安装包路径与 SHA256（如有）
已知问题：失败与未执行项目分别列出，注明是否阻塞发布
下一步：下一个任务 ID、确切阻塞点和所需环境
```

状态规则：代码写完但目标平台未运行是“待验证”；环境缺失且无法继续该任务是“阻塞”；只有对应验收证据齐全才是“通过”。跳过的测试不计通过。后续改动涉及已验收路径时，重新执行受影响测试；不重复与改动无关的全量测试。

每个任务结束自查 diff：是否引入无关重构、大面积 pub、散落平台判断、重复路径/工具解析、未说明的降级、明文敏感日志或锁文件无关升级。复查可以由当前 AI 完成，不默认启动多代理。

### 10.6 可直接交给 AI 的首轮指令

> 阅读仓库 AGENTS.md 和 docs/2026-09-14-windows-linux-support-plan.md，按第 10 节先执行 AI-00，不直接展开全部适配。先核对当前 commit、用户改动和可用平台环境，阅读锁定依赖源码，建立原生构建验证路径并记录真实结果。允许为定位阻塞做最小构建修复，但不升级整套 GPUI、不重写 UI、不删除已有数据库能力。运行必要检查，将依赖决策、失败日志和下一任务写入 implementation-status 文档。缺失目标机器时完成不依赖该机器的工作，明确留下待验证项；不要宣布全平台支持已完成。

后续可用“继续 AI-01，读取最新状态记录并完成该任务的验证”推进；需要持续实施时可明确要求按依赖顺序执行 AI-01～AI-06。无论一次还是多次会话，都保留相同验收门槛。

## 11. 无 Windows/Linux 本机时的验证方案

### 11.1 默认执行路线

当前只有 macOS 本机。先由 AI 建立 **GitHub Actions 原生构建、自动测试与候选包验证**，再按未覆盖项决定是否使用短期云桌面。无需先购买实体机器；也不因为本机缺少目标系统就只写代码、不运行目标平台检查。

AI-00 先检查仓库 Actions 是否可用、可用 runner 镜像和额度，以及当前会话是否具备推送验证分支、触发流程和读取日志的授权。编写配置与实际运行分别记录；账号权限或额度不足时明确阻塞原因。不得假设 Actions 免费或有无限算力，也不自动购买云资源。需要付费环境时先列明用途、规格、时长和费用估算，按用户授权开通，结束后回收。

### 11.2 AI 可自动执行的验证

| 验证层 | 执行环境与方法 | 验收证据 | 不能据此得出的结论 |
| --- | --- | --- | --- |
| 原生编译与链接 | Windows MSVC runner、Ubuntu 基线 runner；check/test/release build | OS/镜像版本、工具链、完整日志与退出码 | 编译成功不等于窗口/输入正常 |
| 配置与平台逻辑 | 两平台临时目录；路径、存储、错误恢复、模拟凭据后端测试 | 测试报告，中文/空格路径与错误用例结果 | 模拟后端不能证明系统密码库可用 |
| 数据库/TLS/SSH/CLI | 临时数据库与 SSH 服务；客户端测试进程在目标 OS 执行 | 连接、查询、证书/host key 拒绝、备份恢复、取消回收记录 | Linux 跑过的客户端流程不能算 Windows 已通过 |
| 原生库与资源 | 递归检查 EXE/DLL、ELF/SO，检查包内资源、版本与架构 | 依赖清单、缺失项、包 SHA256 | 静态依赖扫描不能覆盖全部动态加载问题 |
| 安装/升级/卸载 | 临时 VM 或隔离测试用户；EXE 静默安装、DEB 安装、测试数据升级保留 | 安装日志、文件/数据断言、卸载结果 | 静默安装不验证向导显示；构建 runner 不是干净终端用户系统 |
| Linux 图形冒烟 | Xvfb 覆盖 X11；单独 Wayland compositor，会话 D-Bus/portal，按需软件渲染 | 真实 FluxDB 窗口截图、输入/查询结果、关闭日志 | Xvfb 不覆盖 Wayland；软件渲染不代表真实 GPU |
| 系统凭据集成 | Windows 临时账号；Linux 独立 D-Bus/Secret Service 会话 | 原生后端写读删、应用重启后读取、锁库/无服务结果 | 应用重启不等于 OS 重启；授权弹窗与跨登录持久性需另验 |

数据库测试环境按平台分别设计：GitHub Actions 的 service containers 不能直接照搬到 Windows job。Windows 采用 runner 上安装并启动的测试服务，或受控、可达的临时远端服务；Windows 客户端必须真实参与测试。测试实例不使用生产数据或生产凭据，输出与 artifacts 不包含秘密。

安装验证至少分为两层：构建 runner 快速检查包结构和静默安装；干净 VM 检查未安装 SDK/开发库时是否可启动。若暂时只有构建 runner，后者标记“待验证”，不能用预装运行库的环境证明安装包依赖完整。

系统凭据测试使用唯一 service/account 与清理步骤。OS 重启、注销再登录场景需要可恢复的 VM；普通一次性 CI job 中只重启进程的结果，必须标注为“应用重启验证”。

### 11.3 远程桌面与后续用户测试

以下项目不能默认由普通 hosted CI 完整覆盖：Windows 中文 IME/候选窗、原生文件对话框、托盘与窗口恢复、Linux 桌面凭据授权、多 DPI、真实 GPU 和多屏行为。

优先按剩余风险安排短期 Windows 11 VM 和 Linux 桌面 VM。租用前确认系统镜像：Windows Server 的 runner 或云桌面不能替代 Windows 11 验收；普通无桌面 Linux 云主机也不能直接验证完整桌面交互。

只有 AI 实际具备远程桌面/UI 自动化工具，并能读取截图和操作结果时，才安排 AI 执行点击、输入、对话框和托盘用例。仅有 SSH/命令行权限时，完成可脚本化部分，交互用例保留待验证；不要声称仅开启 RDP 就具备 AI 操作能力。

远程会话需要记录显示协议、虚拟/真实 GPU 与缩放设置。云 VM 无法覆盖的多屏、驱动或本地输入体验，可在候选版阶段通过明确范围的用户测试补充；软件渲染截图不能证明真实显卡性能。所有证据仍沿用第 10.5 节记录格式。

### 11.4 实施与验证闭环

1. **AI-00：** 建立三平台 CI，先运行固定基线，保存原有错误；缺权限时交付可审查配置和触发方式。
2. **AI-01～04：** 每组修改运行本地必要检查，再触发受影响的目标平台任务；AI 读取失败日志、定位根因、修复并复跑。不删除测试或放宽成功判定来消除失败。
3. **AI-05：** 构建候选 EXE/ZIP/DEB，上传日志、测试结果、依赖清单、校验和及可用截图；暂不公开发布。配置任务超时、取消旧提交的重复运行和适当 artifact 保留期，控制消耗。
4. **AI-06：** 对照验收矩阵生成“通过/失败/未验证”清单，按需要补干净 VM 和远程桌面测试。测试结果关联确切 commit 与候选包，不用旧包的通过记录覆盖新改动。

无实体机器不改变第 7 节发布门槛：CI 全绿但关键桌面流程未验时，可以交付内部候选包及剩余事项，不能宣布正式支持已完成。非核心硬件组合未覆盖时明确支持边界；安装启动、退出、凭据、数据完整性与关键输入等必测项目未完成，仍阻塞正式支持声明。

## 12. 补充风险清单（第 2～7 节未覆盖项）

本节补充第一版方案遗漏的问题。共同特征是：它们不会让编译失败，也不会让主窗口打不开，因此最容易被“Windows 能启动了”这类结论掩盖，却直接影响数据安全、正确性和可用性。每项给出问题、所在边界和首版处理立场。归属阶段沿用第 5 节划分，验收项并入第 7 节矩阵执行。

### 12.1 进程实例与数据库并发

现状是无单实例守卫，且 storage 的 SQLite 未显式设置 `busy_timeout`。macOS 下 Dock 单图标掩盖了多实例；Windows 桌面/开始菜单/任务栏快捷方式被反复双击是常态，Linux 同理。两个进程同时写同一个 WAL 库，会出现写入失败、配置回退或连接配置被后写进程覆盖。

处理立场：首版必须二选一，不能不做决定。

- 方案 A（推荐）：加单实例守卫。Windows 用命名互斥量或 Known Folder 下的锁文件，Linux 用 `$XDG_RUNTIME_DIR` 锁文件加 `flock`。第二个实例激活已有窗口或明确退出并提示，不能静默失败。
- 方案 B：明确允许多实例，则必须为 SQLite 设置合理 `busy_timeout` 与重试，并验证并发写配置、并发写查询历史、备份期间写配置三种场景不丢数据。

无论哪种方案，锁文件都要处理进程被强杀后的陈旧锁：不能出现“上次崩溃后再也启动不了”。

### 12.2 敏感文件权限与临时凭据

代码中没有任何 `set_permissions` / `0o600` 调用。macOS 单用户场景问题不明显，Linux 多用户机器与 Windows 共享/域账号场景下，凭据库、连接配置、导出数据和备份文件的可读范围必须收敛。

- Unix：持久化根目录 `0o700`，SQLite 与配置文件 `0o600`。创建后立即设置，不依赖 umask。
- Windows：`%LOCALAPPDATA%` 默认继承用户 ACL，但必须实测验证，尤其是漫游配置文件和被重定向的用户目录。不要写入 `%PROGRAMDATA%` 或安装目录。
- 外部 CLI 的临时凭据文件（如 `PGPASSFILE`、`.my.cnf` 形态）：Unix 先创建再 `0o600` 再写入，Windows 无 `chmod` 语义，优先改用环境变量或 stdin 传递，避免落盘；确有落盘时放进程专属临时目录并在取消/失败/退出路径都清理。
- 导出与备份产物默认路径不放临时目录，权限跟随用户选择的目录，但要在 UI 明示文件含明文数据。

### 12.3 字符编码与换行

三处独立风险，现在都按 UTF-8 假设处理：

1. **导出编码。** CSV/JSON/XML 均写 UTF-8 无 BOM。Windows 简体中文环境的 Excel 按 ANSI（CP936）打开无 BOM 的 CSV，中文全部乱码。首版至少为 CSV 提供 BOM 选项，默认值单独决策并在 UI 标注；同时明确导出换行使用 `\n` 还是 `\r\n`。
2. **外部 CLI 输出解码。** Windows 中文系统上 `pg_dump` / `mysqldump` / `redis-cli` 的 stderr 通常是 CP936 而非 UTF-8，直接按 UTF-8 解码会得到乱码错误信息，既误导用户，也会让基于关键字的脱敏与错误分类失效。处理方式：优先通过环境变量把子进程输出固定为 UTF-8，做不到时按当前 ANSI code page 解码，并把实际采用的策略写进日志。
3. **输入文件识别。** SQL 文件执行与导入路径要处理 UTF-8 BOM、CRLF 和非 UTF-8 编码文件：BOM 不能被当作 SQL 语句首字符，非法编码要给可读错误而不是 `from_utf8_lossy` 后执行出一条错误 SQL。

### 12.4 文件名与路径语义

- **默认目录。** `default_data_export_directory` 的 `HOME/Downloads` 在 Windows 不成立，回退 `current_dir()` 可能是安装目录或 `System32`。改为平台 Known Folder / XDG user dirs 解析，取不到时回退到用户可写目录，而不是进程工作目录。
- **文件名合法性。** 导出文件名由对象名加时间戳拼接，表名里出现 `\ / : * ? " < > |` 时 Windows 直接创建失败，`CON`、`NUL`、`PRN`、`LPT1` 等保留名同样失败，尾部空格和点会被静默截断。写入前统一做文件名净化，并保证净化后仍唯一、仍可辨识来源。
- **路径长度。** Windows 默认 `MAX_PATH` 260。深层用户目录加长表名加时间戳容易越界，表现为莫名其妙的写入失败。要么限制生成长度，要么验证长路径支持并在清单中声明前置条件。
- **大小写敏感性。** macOS APFS 与 Windows 默认不区分大小写，Linux 区分。资源文件、图标、SQL 文件和 `known_hosts` 路径的拼写错误只会在 Linux 暴露，因此 Linux 构建必须跑一次完整资源加载，不能只靠 macOS 验证。

### 12.5 终端、PTY 与子进程

第 4.4 节已覆盖工具发现与进程回收，补充以下平台细节：

- `resolve_program_path` 目前只补 macOS Homebrew 目录。Linux 与 Windows 各自的补充策略要单独给出依据，不能照抄；没有依据就只用显式路径加进程 PATH。
- **Windows GUI 进程的 PATH 来自启动时的环境块。** 用户在应用运行期间新装 `psql`，不重启应用就检测不到。工具检测失败的提示里要包含“安装后需重启 FluxDB”。
- **ConPTY 下限。** 需要 Windows 10 1809 及以上；首版目标为 Windows 11，但要在启动或终端初始化失败时给出明确原因，而不是空白终端。
- **Ctrl-C 语义不同。** Windows 上向 PTY 写 `0x03` 与发送控制台控制事件效果不同；取消操作必须终止整个子进程树（推荐 Job Object），否则 `pg_dump` 等子进程会残留。Linux 用进程组信号。
- **终端 locale。** Linux 下不注入 `LANG`/`LC_ALL` 时，`redis-cli` 等工具的中文输出会乱码；Windows 下需要把控制台 code page 固定为 UTF-8。二者都属于 spawn 时的环境准备，不是 UI 层问题。

### 12.6 网络连接的平台差异

- **`localhost` 解析顺序。** Windows 通常优先解析到 `::1`，而 PostgreSQL / MySQL 默认可能只监听 `127.0.0.1`，导致 macOS 能连、Windows 报连接拒绝。要么按需回退到 IPv4，要么在错误提示中给出可操作建议，不能只抛底层错误。
- **Unix domain socket。** PostgreSQL 的 socket 路径连接方式在 Windows 不存在。若连接表单允许填写 socket 路径，Windows 上应禁用该入口或明确提示不支持，而不是尝试连接后报一个无意义错误。
- **系统代理不自动生效。** 现有代理能力是应用内配置（SOCKS5 / HTTP CONNECT）。Windows 的系统代理与 PAC、Linux 的桌面代理设置都不会自动被采用，这是有意选择，需要在设置界面和文档中写明。
- **休眠唤醒与网络切换。** Windows 笔记本合盖唤醒后 TCP 连接常已失效。要验证连接失效的表现是明确报错并可重连，而不是界面卡在 loading 或使用一个已死的连接执行写操作。

### 12.7 时间与本地化

`Local::now()` 被用于导出文件名、备份文件名和日志时间。Windows 与 Linux 的时区来源不同，精简容器或缺少 tzdata 的 Linux 环境下本地时区解析可能退化为 UTC。需要确认：时区解析失败不导致 panic；文件名时间戳与日志时间口径一致；跨时区用户看到的查询历史时间可解释。首版不做完整 i18n，但至少跑一次 `zh-CN` locale 下的构建与运行验证。

### 12.8 日志、崩溃与可诊断性

- **日志轮转缺失。** 加单文件大小上限、文件数量或保留天数。Windows 上被占用的日志文件删除/改名语义与 Unix 不同，轮转实现要按 Windows 语义验证，失败不能影响应用运行。
- **日志脱敏。** 走统一脱敏函数，覆盖连接串、密码、token、SSH 口令与外部 CLI 的命令行参数。跨平台适配阶段新增的诊断日志最容易漏掉这一步。
- **崩溃可见性。** Windows GUI subsystem 没有 stderr，panic 信息必须落盘，并保证 panic hook 在日志系统初始化失败时仍有兜底输出。约定崩溃转储的获取方式（Windows WER、Linux core dump），写进文档而不是让用户自己摸索。
- **“打开日志目录”入口**在三平台分别使用对应的系统打开方式，且在无桌面会话时给出路径文本而不是报错。

### 12.9 分发信任与升级

- **Windows SmartScreen。** 未签名或新签名的 EXE 首次运行会有警告，OV 证书需要信誉积累周期。这不是缺陷但必须提前决策并写入下载页说明，否则会被当成“安装包有问题”。
- **杀毒误报。** 未签名 + PTY + 网络访问的组合容易被 Defender 或国产安全软件拦截。准备误报申诉渠道，并在候选包阶段实测一次。
- **Linux 桌面集成。** DEB 需要 desktop 文件、hicolor 多尺寸图标、安装后刷新桌面数据库；`postinst`/`postrm` 必须幂等，卸载默认保留用户数据。还要验证应用正在运行时执行 apt 升级的行为。
- **自动更新。** 当前没有实现，首版明确不做，并说明升级方式是下载新包覆盖安装。特别提示 Windows 无法替换正在运行的 EXE，安装器需处理“请先退出应用”。

### 12.10 首版明确不做（防止范围蔓延）

自动更新、文件关联与自定义 URL scheme、系统通知、跨实例协作、屏幕阅读器与高对比度等无障碍适配、Windows 10 / ARM64 / 其他发行版。写入本节的目的是：这些项目在跨平台适配中经常被“顺手加上”，但每一项都会带来独立的验收负担，首版一律按不支持处理并在文档声明。

### 12.11 新增必测项

并入第 7 节验收矩阵执行，通过标准与该节一致。

| 领域 | 必测情形 | 通过标准 |
| --- | --- | --- |
| 实例与并发 | 重复启动应用；两实例同时修改连接配置；备份期间写配置；强杀后重启 | 按选定方案表现一致；无配置丢失或库损坏；陈旧锁不阻塞启动 |
| 文件权限 | Unix 检查根目录与库文件权限位；Windows 检查用户 ACL；临时凭据文件全生命周期 | 非属主不可读；取消与失败路径均完成清理 |
| 编码 | 中文数据导出 CSV 用 Excel 打开；Windows 中文系统触发 CLI 报错；导入 BOM/CRLF/非 UTF-8 SQL 文件 | 无乱码或有明确编码选项；错误信息可读且已脱敏 |
| 文件名与路径 | 表名含非法字符与保留名；超长路径；默认导出目录；Linux 资源大小写 | 导出成功或给出可操作错误；不回退到不可写目录 |
| 终端子进程 | Windows 取消备份后检查进程树；运行中安装 CLI 后的检测提示；中文输出终端 | 无残留子进程；提示可操作；输出不乱码 |
| 网络差异 | Windows 连 `localhost` 的 IPv4/IPv6 库；socket 路径入口；休眠唤醒后操作 | 连接成功或错误可操作；不支持项明确禁用；失效连接不静默执行写操作 |
| 日志与崩溃 | 长时间运行后的日志体积；强制 panic；日志目录被占用 | 体积受控；崩溃信息可取；日志无明文敏感信息 |
| 分发 | 未签名包首次运行；安全软件扫描；DEB 安装卸载重装；运行中升级 | 行为与文档声明一致；卸载保留用户数据 |

第 7 节的发布阻塞项相应扩充：**敏感文件权限不达标、密码或连接信息明文进日志、多实例导致配置或数据丢失、导出文件因文件名或目录问题静默失败**，同样阻塞发布。日志轮转缺失、CSV BOM 策略、SmartScreen 警告可作为已知限制发布，但必须在文档中写明。
