# TODO 清单

## 进程内存监控（临时诊断）

- **状态**：进行中
- **用途**：临时诊断工具，采样当前进程 RSS/虚拟内存/CPU 并写入日志，用于排查长时间运行后的内存增长点。
- **触发点**：日志系统初始化时启动低频采样线程（默认 60s 一条，`FLUXDB_MEMORY_INTERVAL_SECS` 可临时调高频率）。
- **清理**：排查完成后删除以下内容后再合入正式版本：
  - `apps/fluxdb-desktop/src/main_parts/logging.rs` 中的 `DEFAULT_MEMORY_SAMPLE_INTERVAL_SECS`、`MIN_MEMORY_SAMPLE_INTERVAL_SECS` 常量，`memory_sample_interval()`、`start_process_resource_monitor()` 及两处调用。
  - `apps/fluxdb-desktop/Cargo.toml` 与 `Cargo.lock` 中新增的 `sysinfo` 依赖。
  - `apps/fluxdb-desktop/src/main_parts/content_views.rs` 性能诊断开关描述文案中关于"进程内存每分钟写入日志"的表述。
- **关联**：现有"性能诊断"右上角 HUD（FPS/帧耗时/CPU/GPU/内存）只做实时展示，本监控补充其缺失的长时间趋势数据。
