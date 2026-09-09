// RedisConnectionOverview::apply 的 CPU 增量推导逻辑单测。
// 覆盖首次采样（无基线）、正常增量（可换算出百分比）、以及 uptime 未递增（无法换算）三种场景。

#[test]
fn redis_overview_first_load_has_no_cpu_usage() {
    let mut overview = RedisConnectionOverview::default();
    overview.apply(ConnectionOverview {
        version: "7.2.4".to_string(),
        used_memory_bytes: 5 * 1024 * 1024,
        cpu: Some(fluxdb_core::CpuStats {
            sys_seconds: 10.0,
            user_seconds: 20.0,
            uptime_seconds: 10.0,
        }),
    });
    // 首次采样没有基线，CPU 使用率暂不可得；但版本与内存应已更新
    assert_eq!(overview.cpu_usage_percent, None);
    assert_eq!(overview.version, "7.2.4");
    assert_eq!(overview.used_memory_bytes, 5 * 1024 * 1024);
}

#[test]
fn redis_overview_second_load_computes_cpu_percent() {
    let mut overview = RedisConnectionOverview::default();
    // 第一次采样建立基线
    overview.apply(ConnectionOverview {
        version: "7.2.4".to_string(),
        used_memory_bytes: 0,
        cpu: Some(fluxdb_core::CpuStats {
            sys_seconds: 10.0,
            user_seconds: 20.0,
            uptime_seconds: 10.0,
        }),
    });
    // 第二次采样：间隔 10 秒，CPU 累计秒数多 5 秒 → 50%
    overview.apply(ConnectionOverview {
        version: "7.2.4".to_string(),
        used_memory_bytes: 0,
        cpu: Some(fluxdb_core::CpuStats {
            sys_seconds: 12.0,
            user_seconds: 23.0,
            uptime_seconds: 20.0,
        }),
    });
    let expected: f64 = ((5.0_f64 / 10.0_f64) * 100.0_f64).max(0.0);
    assert_eq!(overview.cpu_usage_percent, Some(expected));
}

#[test]
fn redis_overview_stagnant_uptime_keeps_cpu_none() {
    let mut overview = RedisConnectionOverview::default();
    overview.apply(ConnectionOverview {
        version: "7.2.4".to_string(),
        used_memory_bytes: 0,
        cpu: Some(fluxdb_core::CpuStats {
            sys_seconds: 10.0,
            user_seconds: 20.0,
            uptime_seconds: 10.0,
        }),
    });
    // uptime 未递增（并发采样时序异常），无法可靠换算，保留 None 等待下一次采样
    overview.apply(ConnectionOverview {
        version: "7.2.4".to_string(),
        used_memory_bytes: 0,
        cpu: Some(fluxdb_core::CpuStats {
            sys_seconds: 11.0,
            user_seconds: 21.0,
            uptime_seconds: 10.0,
        }),
    });
    assert_eq!(overview.cpu_usage_percent, None);
}
