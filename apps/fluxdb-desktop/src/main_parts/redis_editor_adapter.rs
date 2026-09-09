// Redis 编辑器接入层：把 fluxdb-editor-core 的通用编辑器接到 Redis 业务上下文。
//
// 该层通过 provider/adapter 注入命令补全与执行能力，复用 fluxdb-app 的
// `redis_completion_result` 纯逻辑。本层不直接连接数据库，不做命令拼接。
pub(crate) mod redis_editor_adapter {
    include!("redis_editor_adapter/mod.rs");
}
