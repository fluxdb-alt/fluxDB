// 连接级只读的判定边界。
//
// 不同数据库类型对“写”的判定方式不同（SQL 看语句、Redis 看命令白名单、Mongo 看
// runCommand 命令名），因此由实现方各自分类；这里只定义策略契约，不负责拦截。
//
// 拦截由应用层在共享执行入口统一完成：UI、AI、导入等路径都走同一组入口，
// 避免每个按钮各写一遍判断。

/// 一次待判定的执行意图。
#[derive(Clone, Copy, Debug)]
pub enum WriteIntent<'a> {
    /// SQL 文本，可能包含多条语句。
    Sql(&'a str),
    /// Redis 命令文本，可能包含多行（每行一条命令）。
    RedisCommand(&'a str),
    /// MongoDB `runCommand` 形式的命令 JSON。
    MongoCommand(&'a str),
    /// 无语句文本的结构性写入：表数据编辑、导入、建删对象等。
    Mutation,
}

/// 只读连接的写入判定策略。
///
/// 实现方只回答“该意图在只读连接上是否允许”，不执行任何数据库操作。
pub trait ReadOnlyPolicy: Send + Sync {
    /// 只读连接下该意图是否允许执行。
    ///
    /// 无法证明只读时必须返回 `false`（fail-closed），不能默认放行。
    fn allows(&self, intent: &WriteIntent<'_>) -> bool;
}
