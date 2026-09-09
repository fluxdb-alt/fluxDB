// Redis Workbench 命令词典（命令树模型 + synopsis + 参数定位）。
//
// 这是 `redis_completion.rs` 的数据底座：命令不再只是扁平字符串列表，而是可表达
// `block / oneof / optional / multiple` 的命令树。每个参数节点都能：
//   - 渲染出 RedisInsight 风格的 synopsis（供补全项 detail 展示）；
//   - 支持「按已输入参数定位当前应补的 token」的参数上下文联动。
//
// 设计取舍（参考 RedisInsight 的 supported_commands.json，但按 gdb 轻量复刻）：
//   - 数据以紧凑字符串书写（`<key>` 必填值 / `[x]` 可选 / `x...` 可重复 / `A|B` 二选一），
//     在首次访问时统一解析为运行时命令树，避免大段手写结构体、降低维护成本。
//   - 词典按来源分层：基础命令 + 模块命令（JSON / FT / TS / GRAPH / BF / CF / CMS /
//     TOPK / TDIGEST / AI）。
//
// 本文件经 `include!` 汇入 crate-root 作用域（见 lib.rs），与 `redis_completion.rs`
// 共享命名空间；对外只暴露少量 `pub` 符号供补全引擎调用。

use std::sync::OnceLock;

/// 参数节点类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RedisArgType {
    /// 取值参数，如 `<key>`、`<value>`（无可补 token，仅参与 synopsis）。
    Value,
    /// 固定 token，如 `NX`、`VERBATIM`（可直接补全）。
    Token,
    /// 一组固定 token 任选其一，如 `NX | XX`（子项皆可补全）。
    OneOf,
    /// 嵌套参数块，如 `LOAD <count> <field...>`（内部子参数递归参与定位）。
    Block,
}

/// 命令树中的一个参数节点。
#[derive(Clone, Debug)]
pub struct RedisArg {
    rtype: RedisArgType,
    /// 展示文本：Value 形如 `<key>`；Token 为 token 本身；复合类型的展示名（可空）。
    display: String,
    /// 是否可选。
    optional: bool,
    /// 是否可重复（multiple）。
    multiple: bool,
    /// OneOf 的候选 / Block 的子参数。
    children: Vec<RedisArg>,
}

/// 树状命令的一级子命令（每个自带参数骨架）。
#[derive(Clone, Debug)]
pub struct RedisSubcommand {
    name: String,
    arguments: Vec<RedisArg>,
}

/// 一条命令的运行时定义。
#[derive(Clone, Debug)]
pub struct RedisCommandSpec {
    pub name: String,
    pub subcommands: Vec<RedisSubcommand>,
    pub arguments: Vec<RedisArg>,
}

/// 作者友好（紧凑字符串）的一条命令定义，首次访问时解析为 `RedisCommandSpec`。
struct RedisCommandAuthor {
    name: &'static str,
    /// (子命令名, 该子命令的参数骨架字符串列表)；为空表示无子命令。
    subcommands: &'static [(&'static str, &'static [&'static str])],
    /// 顶层参数骨架字符串列表（无子命令或子命令之后的参数位）。
    arguments: &'static [&'static str],
}

//
// 紧凑参数串语法：
//   <key>          必填取值参数
//   [count]        可选取值参数
//   <key>... 或 [key...]  可重复取值参数
//   [EX|PX|NX]     可选二选一 token 组（参与参数位 token 补全）
//   MAXLEN|MINID   必填二选一 token 组
//   [MATCH]        可选固定 token
//   VERBATIM       必填固定 token
//   <score member...>  空格分隔的复合块（Block）
//

/// 解析单个紧凑参数串为运行时 `RedisArg` 节点。
fn parse_redis_arg(raw: &str) -> RedisArg {
    // 剥离可选括号 `[...]`。
    let (rest, optional) = match raw.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        Some(inner) => (inner, true),
        None => (raw, false),
    };
    // 识别可重复 `...` 后缀。
    let (rest, multiple) = match rest.strip_suffix("...") {
        Some(inner) => (inner, true),
        None => (rest, false),
    };

    // 二选一 token 组：`A|B`，子项递归解析（子项一般是 token，也支持带值的块）。
    if rest.contains('|') {
        let children = rest
            .split('|')
            .map(|part| parse_redis_arg(part.trim()))
            .collect::<Vec<_>>();
        return RedisArg {
            rtype: RedisArgType::OneOf,
            display: String::new(),
            optional,
            multiple,
            children,
        };
    }

    // 复合块：空格分隔的若干子参数，如 `<field> <value>` / `[MATCH] <pattern>`。
    if rest.contains(char::is_whitespace) {
        let children = rest
            .split_whitespace()
            .map(parse_redis_arg)
            .collect::<Vec<_>>();
        return RedisArg {
            rtype: RedisArgType::Block,
            display: String::new(),
            optional,
            multiple,
            children,
        };
    }

    // 取值参数：`<x>`。
    if rest.starts_with('<') && rest.ends_with('>') {
        return RedisArg {
            rtype: RedisArgType::Value,
            display: rest.to_string(),
            optional,
            multiple,
            children: Vec::new(),
        };
    }

    // 其余视为固定 token。
    RedisArg {
        rtype: RedisArgType::Token,
        display: rest.to_string(),
        optional,
        multiple,
        children: Vec::new(),
    }
}

/// 渲染单个参数节点的 synopsis（含可选/可重复括号包裹）。
fn redis_arg_synopsis(arg: &RedisArg) -> String {
    let body = match arg.rtype {
        RedisArgType::Value | RedisArgType::Token => arg.display.clone(),
        RedisArgType::OneOf => arg
            .children
            .iter()
            .map(redis_arg_synopsis)
            .collect::<Vec<_>>()
            .join(" | "),
        RedisArgType::Block => arg
            .children
            .iter()
            .map(redis_arg_synopsis)
            .collect::<Vec<_>>()
            .join(" "),
    };
    let with_multi = if arg.multiple {
        format!("{} ...", body)
    } else {
        body
    };
    if arg.optional {
        format!("[{}]", with_multi)
    } else {
        with_multi
    }
}

/// 把 `&[&str]` 解析为参数节点列表。
fn parse_redis_args(args: &[&str]) -> Vec<RedisArg> {
    args.iter().map(|s| parse_redis_arg(s)).collect()
}

//
// 命令词典（作者友好表）。
// 基础命令直接沿用原有覆盖；模块命令按 Redis 官方 / RedisInsight 命令规格补齐。
//

const fn sub(
    name: &'static str,
    arguments: &'static [&'static str],
) -> (&'static str, &'static [&'static str]) {
    (name, arguments)
}

static REDIS_COMMANDS_AUTHOR: &[RedisCommandAuthor] = &[
    modcmd("JSON", &[
        sub("ARRAPPEND", &["<key>", "<path>", "[value ...]"]),
        sub("ARRINDEX", &["<key>", "<path>", "<value>"]),
        sub("ARRINSERT", &["<key>", "<path>", "<index>", "[value ...]"]),
        sub("ARRLEN", &["<key>", "[path]"]),
        sub("ARRPOP", &["<key>", "[path]", "[index]"]),
        sub("ARRTRIM", &["<key>", "<path>", "<start>", "<stop>"]),
        sub("CLEAR", &["<key>", "<path>"]),
        sub("DEBUG", &["<subcommand>", "[key]"]),
        sub("DEL", &["<key>", "[path]"]),
        sub("FORGET", &["<key>", "<path>"]),
        sub("GET", &["<key>", "[path]"]),
        sub("MGET", &["<key>", "[path]"]),
        sub("MSET", &["<key>", "<path>", "<value>", "[path value ...]"]),
        sub("NUMINCRBY", &["<key>", "<path>", "<increment>"]),
        sub("NUMMULTBY", &["<key>", "<path>", "<factor>"]),
        sub("OBJLEN", &["<key>", "[path]"]),
        sub("OBJKEYS", &["<key>", "[path]"]),
        sub("RESP", &["<key>", "<path>"]),
        sub("SET", &["<key>", "<path>", "<value>", "[NX|XX]", "[FORGET_DEF]"]),
        sub("STRAPPEND", &["<key>", "<path>", "<value>"]),
        sub("STRLEN", &["<key>", "[path]"]),
        sub("TOGGLE", &["<key>", "<path>"]),
        sub("TYPE", &["<key>", "[path]"]),
    ]),
    modcmd("FT", &[
        sub("AGGREGATE", &["<index>", "<query>", "[LOAD] <count> [field ...]", "[GROUPBY] <nargs> [property ...]", "[SORTBY] <nargs>", "[LIMIT] <offset> <num>", "[APPLY] <expression>", "[FILTER] <filter>", "[WITHCURSOR]", "[COUNT] <num>", "[PARAMS] <nargs> [name value ...]", "[DIALECT] <dialect>"]),
        sub("ALIASADD", &["<index>", "<alias>"]),
        sub("ALIASDEL", &["<alias>"]),
        sub("ALIASUPDATE", &["<index>", "<alias>"]),
        sub("ALTER", &["<index>", "[SCHEMA] <add> [field ...]"]),
        sub("CONFIG", &["<subcommand>", "[option]"]),
        sub("CREATE", &["<index>", "[ON] <type>", "[PREFIX] <count> [prefix ...]", "[LANGUAGE] <lang>", "[SCHEMA] [field ...]"]),
        sub("CURSOR", &["<subcommand>"]),
        sub("DICTADD", &["<dict>", "[term ...]"]),
        sub("DICTDEL", &["<dict>", "[term ...]"]),
        sub("DICTDUMP", &["<dict>"]),
        sub("DROPINDEX", &["<index>", "[DD]"]),
        sub("EXPLAIN", &["<index>", "<query>", "[DIALECT] <dialect>"]),
        sub("EXPLAINCLI", &["<index>", "<query>", "[DIALECT] <dialect>"]),
        sub("INFO", &["<index>"]),
        sub("PROFILE", &["<index>", "<query>"]),
        sub("SEARCH", &["<index>", "<query>", "[NOCONTENT]", "[VERBATIM]", "[NOSTOPWORDS]", "[WITHSCORES]", "[WITHPAYLOADS]", "[WITHSORTKEYS]", "[PARAMS] <nargs> [name value ...]", "[SORTBY] <field> [DESC|ASC]", "[LIMIT] <offset> <num>", "[RETURN] <count> [field ...]", "[FILTER] <filter>", "[GEOFILTER] <field> <lng> <lat> <radius> <unit>", "[INKEYS] <count> [key ...]", "[ANALYZE]", "[DIALECT] <dialect>"]),
        sub("SPELLCHECK", &["<index>", "<query>", "[TERMS] <include|exclude> <dict>", "[DISTANCE] <distance>", "[DIALECT] <dialect>"]),
        sub("SUGADD", &["<key>", "<string>", "<score>", "[INCR]", "[PAYLOAD] <payload>"]),
        sub("SUGDEL", &["<key>", "<string>"]),
        sub("SUGGET", &["<key>", "<prefix>", "[FUZZY]", "[WITHSCORES]", "[WITHPAYLOADS]", "[MAX] <num>"]),
        sub("SUGLEN", &["<key>"]),
        sub("SYNDUMP", &["<index>"]),
        sub("SYNUPDATE", &["<index>", "<synonym_group_id>", "[SKIPINITIALSCAN]", "[term ...]"]),
        sub("TAGVALS", &["<index>", "<field>"]),
    ]),
    modcmd("TS", &[
        sub("ADD", &["<key>", "<timestamp>", "<value>", "[RETENTION] <retentionperiod>", "[ENCODING] <COMPRESSED|UNCOMPRESSED>", "[CHUNK_SIZE] <size>", "[ON_DUPLICATE] <policy>", "[IGNORE] <basetime> <delta>", "[LABELS] <label> <value> [label value ...]"]),
        sub("ALTER", &["<key>", "[RETENTION] <retentionperiod>", "[CHUNK_SIZE] <size>", "[DUPLICATE_POLICY] <policy>", "[IGNORE] <basetime> <delta>", "[LABELS] <label> <value> [label value ...]"]),
        sub("CREATE", &["<key>", "[RETENTION] <retentionperiod>", "[ENCODING] <COMPRESSED|UNCOMPRESSED>", "[CHUNK_SIZE] <size>", "[DUPLICATE_POLICY] <policy>", "[IGNORE] <basetime> <delta>", "[LABELS] <label> <value> [label value ...]"]),
        sub("CREATERULE", &["<sourcekey>", "<destkey>", "<AGGREGATOR>", "[BUCKETDURATION] <bucketduration>"]),
        sub("DECRBY", &["<key>", "<value>", "[TIMESTAMP] <timestamp>", "[RETENTION] <retentionperiod>", "[CHUNK_SIZE] <size>", "[UNCOMPRESSED]", "[IGNORE] <basetime> <delta>", "[LABELS] <label> <value> [label value ...]"]),
        sub("DEL", &["<key>", "<fromtimestamp>", "<totimestamp>"]),
        sub("DELETERULE", &["<sourcekey>", "<destkey>"]),
        sub("GET", &["<key>", "[LATEST]"]),
        sub("INCRBY", &["<key>", "<value>", "[TIMESTAMP] <timestamp>", "[RETENTION] <retentionperiod>", "[CHUNK_SIZE] <size>", "[UNCOMPRESSED]", "[IGNORE] <basetime> <delta>", "[LABELS] <label> <value> [label value ...]"]),
        sub("INFO", &["<key>", "[DEBUG]"]),
        sub("MADD", &["<key> <timestamp> <value> [key timestamp value ...]"]),
        sub("MGET", &["<key> [key ...]", "[WITHLABELS]", "[SELECTED_LABELS] <label> [label ...]", "[FILTER] <filter> [filter ...]"]),
        sub("MRANGE", &["<fromtimestamp>", "<totimestamp>", "[LATEST]", "[FILTERBY_TS] <ts> [ts ...]", "[FILTERBY_VALUE] <min> <max>", "[WITHLABELS]", "[SELECTED_LABELS] <label> [label ...]", "[COUNT] <count>", "[ALIGN] <align>", "[AGGREGATION] <aggregator> <bucketduration>", "[FILTER] <filter> [filter ...]", "[GROUPBY] <label> <reducer>", "[REDUCE] <reducer>"]),
        sub("MREVRANGE", &["<fromtimestamp>", "<totimestamp>", "[LATEST]", "[FILTERBY_TS] <ts> [ts ...]", "[FILTERBY_VALUE] <min> <max>", "[WITHLABELS]", "[SELECTED_LABELS] <label> [label ...]", "[COUNT] <count>", "[ALIGN] <align>", "[AGGREGATION] <aggregator> <bucketduration>", "[FILTER] <filter> [filter ...]", "[GROUPBY] <label> <reducer>", "[REDUCE] <reducer>"]),
        sub("QUERYINDEX", &["<filter> [filter ...]"]),
        sub("RANGE", &["<key>", "<fromtimestamp>", "<totimestamp>", "[LATEST]", "[FILTERBY_TS] <ts> [ts ...]", "[FILTERBY_VALUE] <min> <max>", "[COUNT] <count>", "[ALIGN] <align>", "[AGGREGATION] <aggregator> <bucketduration>"]),
        sub("REVRANGE", &["<key>", "<fromtimestamp>", "<totimestamp>", "[LATEST]", "[FILTERBY_TS] <ts> [ts ...]", "[FILTERBY_VALUE] <min> <max>", "[COUNT] <count>", "[ALIGN] <align>", "[AGGREGATION] <aggregator> <bucketduration>"]),
    ]),
    modcmd("GRAPH", &[
        sub("CONFIG", &["GET", "[name ...]", "SET", "<name> <value>"]),
        sub("DELETE", &["<key>"]),
        sub("EXPLAIN", &["<key>", "<query>"]),
        sub("LIST", &[]),
        sub("PROFILE", &["<key>", "<query>"]),
        sub("QUERY", &["<key>", "<query>", "[TIMEOUT] <timeout>", "[COMPACT]"]),
        sub("RO_QUERY", &["<key>", "<query>", "[TIMEOUT] <timeout>", "[COMPACT]"]),
        sub("SLOWLOG", &["<key>", "[subcommand]"]),
    ]),
    modcmd("BF", &[
        sub("ADD", &["<key>", "<item>"]),
        sub("EXISTS", &["<key>", "<item>"]),
        sub("INFO", &["<key>"]),
        sub("INSERT", &["<key>", "[CAPACITY] <capacity>", "[ERROR] <error>", "[EXPANSION] <expansion>", "[NOCREATE]", "[NONSCALING]", "[ITEMS] <item> [item ...]"]),
        sub("LOADCHUNK", &["<key>", "<iterator>", "<data>"]),
        sub("MADD", &["<key>", "<item> [item ...]"]),
        sub("MEXISTS", &["<key>", "<item> [item ...]"]),
        sub("RESERVE", &["<key>", "<error_rate>", "<capacity>", "[EXPANSION] <expansion>", "[NONSCALING]"]),
        sub("SCANDUMP", &["<key>", "<iterator>"]),
    ]),
    modcmd("CF", &[
        sub("ADD", &["<key>", "<item>"]),
        sub("ADDNX", &["<key>", "<item>"]),
        sub("COUNT", &["<key>", "<item>"]),
        sub("DEL", &["<key>", "<item>"]),
        sub("EXISTS", &["<key>", "<item>"]),
        sub("INFO", &["<key>"]),
        sub("INSERT", &["<key>", "[CAPACITY] <capacity>", "[NOCREATE]", "[ITEMS] <item> [item ...]"]),
        sub("LOADCHUNK", &["<key>", "<iterator>", "<data>"]),
        sub("MEXISTS", &["<key>", "<item> [item ...]"]),
        sub("RESERVE", &["<key>", "<capacity>", "[BUCKETSIZE] <bucketsize>", "[MAXITERATIONS] <maxiterations>", "[EXPANSION] <expansion>"]),
        sub("SCANDUMP", &["<key>", "<iterator>"]),
    ]),
    modcmd("CMS", &[
        sub("INCRBY", &["<key>", "<item>", "<increment>", "[item increment ...]"]),
        sub("INFO", &["<key>"]),
        sub("INITBYDIM", &["<key>", "<width>", "<depth>"]),
        sub("INITBYPROB", &["<key>", "<error>", "<probability>"]),
        sub("MERGE", &["<destkey>", "<numkeys>", "<key> [key ...]", "[WEIGHTS] <weight> [weight ...]"]),
        sub("QUERY", &["<key>", "<item> [item ...]"]),
    ]),
    modcmd("TOPK", &[
        sub("ADD", &["<key>", "<item> [item ...]"]),
        sub("COUNT", &["<key>", "<item> [item ...]"]),
        sub("INCRBY", &["<key>", "<item> <increment> [item increment ...]"]),
        sub("INFO", &["<key>"]),
        sub("LIST", &["<key>", "[WITHCOUNT]"]),
        sub("QUERY", &["<key>", "<item> [item ...]"]),
        sub("RESERVE", &["<key>", "<topk>", "[INCRBY] <width> <depth> <decay>"]),
    ]),
    modcmd("TDIGEST", &[
        sub("ADD", &["<key>", "<value> [value ...]"]),
        sub("BYRANK", &["<key>", "<rank> [rank ...]"]),
        sub("BYREVRANK", &["<key>", "<rank> [rank ...]"]),
        sub("CDF", &["<key>", "<value> [value ...]"]),
        sub("CREATE", &["<key>", "[COMPRESSION] <compression>"]),
        sub("INFO", &["<key>"]),
        sub("MAX", &["<key>"]),
        sub("MERGE", &["<destkey>", "<numkeys>", "<key> [key ...]", "[COMPRESSION] <compression>", "[OVERRIDE]"]),
        sub("MIN", &["<key>"]),
        sub("QUANTILE", &["<key>", "<quantile> [quantile ...]"]),
        sub("RANK", &["<key>", "<value> [value ...]"]),
        sub("RESET", &["<key>"]),
        sub("REVRANK", &["<key>", "<value> [value ...]"]),
        sub("TRIMMED_MEAN", &["<key>", "<low_cut_quantile>", "<high_cut_quantile>"]),
    ]),
    modcmd("AI", &[
        sub("CONFIG", &["<subcommand>"]),
        sub("INFO", &["<key>"]),
        sub("MODELDEL", &["<key>"]),
        sub("MODELGET", &["<key>", "[BLOB]"]),
        sub("MODELPUT", &["<key>", "<backend>", "<device>", "[INPUTS] <input_count> <input> [input ...]", "[OUTPUTS] <output_count> <output> [output ...]", "[BATCHSIZE] <batchsize>", "[MINBATCHSIZE] <minbatchsize>", "[TAG] <tag>", "[BLOB] <blob>"]),
        sub("MODELRUN", &["<key>", "<input_tensor> [input_tensor ...]", "[OUT] <output_count> <output> [output ...]", "[TIMEOUT] <timeout>"]),
        sub("SCRIPTDEL", &["<key>"]),
        sub("SCRIPTGET", &["<key>"]),
        sub("SCRIPTPUT", &["<key>", "<script>", "[DEVICE] <device>", "[TAG] <tag>"]),
        sub("SCRIPTRUN", &["<key>", "<input_tensor> [input_tensor ...]", "[OUT] <output_count> <output> [output ...]", "[TIMEOUT] <timeout>"]),
        sub("TENSORGET", &["<key>", "[META]", "[VALUES]"]),
        sub("TENSORSET", &["<key>", "<type>", "<shape>", "[BLOB] <blob>", "<value>", "[VALUES] <value> [value ...]", "[SHAPE] <shape>", "[TYPE] <type>"]),
    ]),
    // 基础命令（沿用原有覆盖，含子命令树与参数骨架）。
    base("APPEND", &["<key>", "<value>"]),
    base("AUTH", &["<username>", "<password>"]),
    base("BGREWRITEAOF", &[]),
    base("BGSAVE", &["SCHEDULE"]),
    base("BITCOUNT", &["<key>", "[start]", "[end]"]),
    base("BITFIELD", &["<key>", "<operation>"]),
    base("BITOP", &["<operation>", "<destkey>", "<key>"]),
    base("BITPOS", &["<key>", "<bit>"]),
    base("BLMOVE", &["<source>", "<destination>", "<wherefrom>", "<whereto>", "<timeout>"]),
    base("BLPOP", &["<key>", "[key...]", "<timeout>"]),
    base("BRPOP", &["<key>", "<timeout>"]),
    base("BRPOPLPUSH", &["<source>", "<destination>", "<timeout>"]),
    base("BZPOPMAX", &["<key>", "<timeout>"]),
    base("BZPOPMIN", &["<key>", "<timeout>"]),
    modcmd("CLIENT", &[
        sub("CACHING", &[]),
        sub("GETNAME", &[]),
        sub("GETREDIR", &[]),
        sub("ID", &[]),
        sub("INFO", &[]),
        sub("KILL", &[]),
        sub("LIST", &[]),
        sub("NO-EVICT", &[]),
        sub("NO-TOUCH", &[]),
        sub("PAUSE", &[]),
        sub("REPLY", &[]),
        sub("SETNAME", &[]),
        sub("SETINFO", &[]),
        sub("TRACKING", &[]),
        sub("TRACKINGINFO", &[]),
        sub("UNBLOCK", &[]),
        sub("UNPAUSE", &[]),
    ]),
    modcmd("CLUSTER", &[
        sub("ADDSLOTS", &[]),
        sub("ADDSLOTSRANGE", &[]),
        sub("BUMPEPOCH", &[]),
        sub("COUNT-FAILURE-REPORTS", &[]),
        sub("COUNTKEYSINSLOT", &[]),
        sub("DELSLOTS", &[]),
        sub("DELSLOTSRANGE", &[]),
        sub("FAILOVER", &[]),
        sub("FLUSHSLOTS", &[]),
        sub("FORGET", &[]),
        sub("GETKEYSINSLOT", &[]),
        sub("INFO", &[]),
        sub("KEYSLOT", &[]),
        sub("LINKS", &[]),
        sub("MEET", &[]),
        sub("MYID", &[]),
        sub("MYSLOTS", &[]),
        sub("NODES", &[]),
        sub("REPLICAS", &[]),
        sub("REPLICATE", &[]),
        sub("RESET", &[]),
        sub("SAVECONFIG", &[]),
        sub("SET-CONFIG-EPOCH", &[]),
        sub("SETSLOT", &[]),
        sub("SHARDS", &[]),
        sub("SLOTS", &[]),
        sub("SLOTSEXPLAIN", &[]),
        sub("NUMKEYS", &[]),
    ]),
    modcmd("COMMAND", &[sub("COUNT", &[]), sub("DOCS", &[]), sub("GETKEYS", &[]), sub("INFO", &[]), sub("LIST", &[])]),
    modcmd("CONFIG", &[sub("GET", &[]), sub("RESETSTAT", &[]), sub("REWRITE", &[]), sub("SET", &[])]),
    base("COPY", &["<source>", "<destination>", "[DB]", "[REPLACE]"]),
    base("DBSIZE", &[]),
    base("DECR", &["<key>"]),
    base("DECRBY", &["<key>", "<decrement>"]),
    base("DEL", &["<key>", "[key...]"]),
    base("DISCARD", &[]),
    base("DUMP", &["<key>"]),
    base("ECHO", &["<message>"]),
    base("EVAL", &["<script>", "<numkeys>", "<key>", "[arg...]"]),
    base("EVALSHA", &["<sha1>", "<numkeys>", "<key>", "[arg...]"]),
    base("EXEC", &[]),
    base("EXISTS", &["<key>", "[key...]"]),
    base("EXPIRE", &["<key>", "<seconds>"]),
    base("EXPIREAT", &["<key>", "<timestamp>"]),
    base("EXPIRELT", &["<key>", "<milliseconds>"]),
    base("EXPIREATLT", &["<key>", "<timestamp>"]),
    base("EXPLAIN", &["<query>"]),
    base("EXPORT", &["<key>", "<file>"]),
    base("FAILOVER", &["[TO]", "<host>", "<port>", "[FORCE]", "[ABORT]"]),
    base("FLUSHALL", &["[ASYNC|SYNC]"]),
    base("FLUSHDB", &["[ASYNC|SYNC]"]),
    base("GEODIST", &["<key>", "<member1>", "<member2>", "[unit]"]),
    base("GEOHASH", &["<key>", "<member>"]),
    base("GEOPOS", &["<key>", "<member>"]),
    base("GEORADIUS", &["<key>", "<longitude>", "<latitude>", "<radius>", "<unit>"]),
    base("GEOSEARCH", &["<key>", "<member>", "<shape>"]),
    base("GET", &["<key>"]),
    base("GETBIT", &["<key>", "<offset>"]),
    base("GETDEL", &["<key>"]),
    base("GETEX", &["<key>", "<expiration>"]),
    base("GETRANGE", &["<key>", "<start>", "<end>"]),
    base("GETSET", &["<key>", "<value>"]),
    base("HDEL", &["<key>", "<field>", "[field...]"]),
    base("HEXISTS", &["<key>", "<field>"]),
    base("HEXPIRE", &["<key>", "<seconds>", "<field>"]),
    base("HGET", &["<key>", "<field>"]),
    base("HGETALL", &["<key>"]),
    base("HINCRBY", &["<key>", "<field>", "<increment>"]),
    base("HINCRBYFLOAT", &["<key>", "<field>", "<increment>"]),
    base("HKEYS", &["<key>"]),
    base("HLEN", &["<key>"]),
    base("HMGET", &["<key>", "<field>", "[field...]"]),
    base("HMSET", &["<key>", "<field>", "<value>", "[field value...]"]),
    base("HRANDFIELD", &["<key>", "[count]"]),
    base("HSCAN", &["<key>", "<cursor>", "[MATCH]", "[COUNT]", "[NOVALUES]"]),
    base("HSET", &["<key>", "<field>", "<value>", "[field value...]"]),
    base("HSETNX", &["<key>", "<field>", "<value>"]),
    base("HSTRLEN", &["<key>", "<field>"]),
    base("HVALS", &["<key>"]),
    base("INCR", &["<key>"]),
    base("INCRBY", &["<key>", "<increment>"]),
    base("INCRBYFLOAT", &["<key>", "<increment>"]),
    base("INFO", &["[section]"]),
    base("KEYS", &["<pattern>"]),
    base("LASTSAVE", &[]),
    modcmd("LATENCY", &[sub("DOCTOR", &[]), sub("GRAPH", &[]), sub("HISTORY", &[]), sub("LATEST", &[]), sub("RESET", &[])]),
    base("LINDEX", &["<key>", "<index>"]),
    base("LINSERT", &["<key>", "<where>", "<pivot>", "<element>"]),
    base("LLEN", &["<key>"]),
    base("LMOVE", &["<source>", "<destination>", "<wherefrom>", "<whereto>"]),
    base("LPOP", &["<key>", "[count]"]),
    base("LPOS", &["<key>", "<element>"]),
    base("LPUSH", &["<key>", "<element>", "[element...]"]),
    base("LPUSHX", &["<key>", "<element>", "[element...]"]),
    base("LRANGE", &["<key>", "<start>", "<stop>"]),
    base("LREM", &["<key>", "<count>", "<element>"]),
    base("LSET", &["<key>", "<index>", "<element>"]),
    base("LTRIM", &["<key>", "<start>", "<stop>"]),
    base("MGET", &["<key>", "[key...]"]),
    base("MIGRATE", &["<host>", "<port>", "<key>", "<destination-db>", "<timeout>"]),
    base("MONITOR", &[]),
    base("MOVE", &["<key>", "<db>"]),
    base("MSET", &["<key>", "<value>", "[key value...]"]),
    base("MSETNX", &["<key>", "<value>", "[key value...]"]),
    base("MULTI", &[]),
    modcmd("OBJECT", &[sub("ENCODING", &[]), sub("FREQ", &[]), sub("IDLETIME", &[]), sub("REFCOUNT", &[])]),
    base("PERSIST", &["<key>"]),
    base("PEXPIRE", &["<key>", "<milliseconds>"]),
    base("PEXPIREAT", &["<key>", "<milliseconds-timestamp>"]),
    base("PFADD", &["<key>", "<element>", "[element...]"]),
    base("PFCOUNT", &["<key>", "[key...]"]),
    base("PFMERGE", &["<destkey>", "<sourcekey>", "[sourcekey...]"]),
    base("PING", &["[message]"]),
    base("PSETEX", &["<key>", "<milliseconds>", "<value>"]),
    base("PSUBSCRIBE", &["<pattern>", "[pattern...]"]),
    base("PSYNC", &["<replicationid>", "<offset>"]),
    base("PTTL", &["<key>"]),
    base("PUBLISH", &["<channel>", "<message>"]),
    modcmd("PUBSUB", &[sub("CHANNELS", &[]), sub("NUMPAT", &[]), sub("NUMSUB", &[]), sub("SHARDCHANNELS", &[]), sub("SHARDNUMSUB", &[])]),
    base("PUNSUBSCRIBE", &["[pattern...]"]),
    base("QUIT", &[]),
    base("RANDOMKEY", &[]),
    base("READONLY", &[]),
    base("READWRITE", &[]),
    base("RENAME", &["<key>", "<newkey>"]),
    base("RENAMENX", &["<key>", "<newkey>"]),
    base("REPLCONF", &["<option>", "<value>"]),
    base("REPLICAOF", &["<host>", "<port>"]),
    base("RESET", &[]),
    base("RESTORE", &["<key>", "<ttl>", "<serialized-value>"]),
    base("ROLE", &[]),
    base("RPOP", &["<key>", "[count]"]),
    base("RPOPLPUSH", &["<source>", "<destination>"]),
    base("RPUSH", &["<key>", "<element>", "[element...]"]),
    base("RPUSHX", &["<key>", "<element>", "[element...]"]),
    base("SADD", &["<key>", "<member>", "[member...]"]),
    base("SAVE", &[]),
    base("SCAN", &["<cursor>", "[MATCH]", "[COUNT]", "[TYPE]"]),
    base("SCARD", &["<key>"]),
    modcmd("SCRIPT", &[sub("DEBUG", &[]), sub("EXISTS", &[]), sub("FLUSH", &[]), sub("KILL", &[]), sub("LOAD", &[])]),
    base("SDIFF", &["<key>", "[key...]"]),
    base("SDIFFSTORE", &["<destination>", "<key>", "[key...]"]),
    base("SELECT", &["<index>"]),
    base("SET", &["<key>", "<value>", "[EX <seconds> | PX <milliseconds> | EXAT <timestamp> | PXAT <milliseconds-timestamp> | KEEPTTL]", "[NX|XX]", "[GET]"]),
    base("SETBIT", &["<key>", "<offset>", "<value>"]),
    base("SETEX", &["<key>", "<seconds>", "<value>"]),
    base("SETNX", &["<key>", "<value>"]),
    base("SETRANGE", &["<key>", "<offset>", "<value>"]),
    base("SHUTDOWN", &["[NOSAVE|SAVE]"]),
    base("SINTER", &["<key>", "[key...]"]),
    base("SINTERCARD", &["<numkeys>", "<key>", "[LIMIT]"]),
    base("SINTERSTORE", &["<destination>", "<key>", "[key...]"]),
    base("SISMEMBER", &["<key>", "<member>"]),
    base("SLAVEOF", &["<host>", "<port>"]),
    modcmd("SLOWLOG", &[sub("GET", &[]), sub("HELP", &[]), sub("LEN", &[]), sub("RESET", &[])]),
    base("SMEMBERS", &["<key>"]),
    base("SMISMEMBER", &["<key>", "<member>", "[member...]"]),
    base("SMOVE", &["<source>", "<destination>", "<member>"]),
    base("SORT", &["<key>", "[BY]", "[LIMIT]", "[GET]", "[ASC|DESC]", "[ALPHA]", "[STORE]"]),
    base("SORT_RO", &["<key>", "[BY]", "[LIMIT]", "[GET]", "[ASC|DESC]", "[ALPHA]"]),
    base("SPOP", &["<key>", "[count]"]),
    base("SPUBLISH", &["<channel>", "<message>"]),
    base("SREM", &["<key>", "<member>", "[member...]"]),
    base("SSCAN", &["<key>", "<cursor>", "[MATCH]", "[COUNT]"]),
    base("STRLEN", &["<key>"]),
    base("SUBSCRIBE", &["<channel>", "[channel...]"]),
    base("SUBSTR", &["<key>", "<start>", "<end>"]),
    base("SUNION", &["<key>", "[key...]"]),
    base("SUNIONSTORE", &["<destination>", "<key>", "[key...]"]),
    base("SWAPDB", &["<index1>", "<index2>"]),
    base("SYNC", &[]),
    base("TIME", &[]),
    base("TOUCH", &["<key>", "[key...]"]),
    base("TTL", &["<key>"]),
    base("TYPE", &["<key>"]),
    base("UNLINK", &["<key>", "[key...]"]),
    base("UNSUBSCRIBE", &["[channel...]"]),
    base("UNWATCH", &[]),
    base("WATCH", &["<key>", "[key...]"]),
    base("WAIT", &["<numreplicas>", "<timeout>"]),
    base("XAUTOCLAIM", &["<key>", "<group>", "<consumer>", "<min-idle-time>", "<start>"]),
    base("XACK", &["<key>", "<group>", "<id>", "[id...]"]),
    base("XADD", &["<key>", "<id>", "<field>", "<value>", "[field value...]"]),
    base("XCLAIM", &["<key>", "<group>", "<consumer>", "<min-idle-time>", "<id>"]),
    base("XDEL", &["<key>", "<id>", "[id...]"]),
    modcmd("XGROUP", &[sub("CREATE", &[]), sub("CREATECONSUMER", &[]), sub("DELCONSUMER", &[]), sub("DESTROY", &[]), sub("HELP", &[]), sub("SETID", &[])]),
    modcmd("XINFO", &[sub("CONSUMERS", &[]), sub("GROUPS", &[]), sub("HELP", &[]), sub("STREAM", &[])]),
    base("XLEN", &["<key>"]),
    base("XPENDING", &["<key>", "<group>"]),
    base("XRANGE", &["<key>", "<start>", "<end>", "[COUNT]"]),
    base("XREAD", &["[COUNT]", "[BLOCK]", "STREAMS", "<key>", "<id>"]),
    base("XREADGROUP", &["GROUP", "<group>", "<consumer>", "[COUNT]", "[BLOCK]", "STREAMS", "<key>", "<id>"]),
    base("XREVRANGE", &["<key>", "<end>", "<start>", "[COUNT]"]),
    base("XSETID", &["<key>", "<last-id>"]),
    base("XTRIM", &["<key>", "MAXLEN|MINID", "<threshold>"]),
    base("ZADD", &["<key>", "<score>", "<member>", "[score member...]"]),
    base("ZCARD", &["<key>"]),
    base("ZCOUNT", &["<key>", "<min>", "<max>"]),
    base("ZDIFF", &["<numkeys>", "<key>", "[key...]"]),
    base("ZDIFFSTORE", &["<destination>", "<numkeys>", "<key>", "[key...]"]),
    base("ZINCRBY", &["<key>", "<increment>", "<member>"]),
    base("ZINTER", &["<numkeys>", "<key>", "[key...]"]),
    base("ZINTERCARD", &["<numkeys>", "<key>", "[key...]"]),
    base("ZINTERSTORE", &["<destination>", "<numkeys>", "<key>", "[key...]"]),
    base("ZLEXCOUNT", &["<key>", "<min>", "<max>"]),
    base("ZMPOP", &["<numkeys>", "<key>", "[key...]", "<MIN|MAX>"]),
    base("ZMSCORE", &["<key>", "<member>", "[member...]"]),
    base("ZPOPMAX", &["<key>", "[count]"]),
    base("ZPOPMIN", &["<key>", "[count]"]),
    base("ZRANDMEMBER", &["<key>", "[count]"]),
    base("ZRANGE", &["<key>", "<min>", "<max>", "[BYSCORE|BYLEX]", "[REV]", "[LIMIT]", "[WITHSCORES]"]),
    base("ZRANGEBYLEX", &["<key>", "<min>", "<max>", "[LIMIT]"]),
    base("ZRANGEBYSCORE", &["<key>", "<min>", "<max>", "[WITHSCORES]", "[LIMIT]"]),
    base("ZRANGESTORE", &["<dstkey>", "<src>", "<min>", "<max>"]),
    base("ZRANK", &["<key>", "<member>"]),
    base("ZREM", &["<key>", "<member>", "[member...]"]),
    base("ZREMRANGEBYLEX", &["<key>", "<min>", "<max>"]),
    base("ZREMRANGEBYRANK", &["<key>", "<start>", "<stop>"]),
    base("ZREMRANGEBYSCORE", &["<key>", "<min>", "<max>"]),
    base("ZREVRANGE", &["<key>", "<start>", "<stop>", "[WITHSCORES]"]),
    base("ZREVRANGEBYLEX", &["<key>", "<max>", "<min>", "[LIMIT]"]),
    base("ZREVRANGEBYSCORE", &["<key>", "<max>", "<min>", "[WITHSCORES]", "[LIMIT]"]),
    base("ZREVRANK", &["<key>", "<member>"]),
    base("ZSCAN", &["<key>", "<cursor>", "[MATCH]", "[COUNT]"]),
    base("ZSCORE", &["<key>", "<member>"]),
    base("ZUNION", &["<numkeys>", "<key>", "[key...]", "[WITHSCORES]"]),
    base("ZUNIONSTORE", &["<destination>", "<numkeys>", "<key>", "[key...]"]),
];

/// 构造「无子命令、只有顶层参数」的基础命令作者条目。
const fn base(name: &'static str, arguments: &'static [&'static str]) -> RedisCommandAuthor {
    RedisCommandAuthor { name, subcommands: &[], arguments }
}

/// 构造模块命令作者条目（只有子命令、无顶层参数）。
const fn modcmd(
    name: &'static str,
    subcommands: &'static [(&'static str, &'static [&'static str])],
) -> RedisCommandAuthor {
    RedisCommandAuthor { name, subcommands, arguments: &[] }
}

/// 懒加载解析后的命令词典（首次访问时把作者表解析为运行时命令树）。
static REDIS_COMMANDS: OnceLock<Vec<RedisCommandSpec>> = OnceLock::new();

fn redis_commands() -> &'static [RedisCommandSpec] {
    REDIS_COMMANDS.get_or_init(|| {
        REDIS_COMMANDS_AUTHOR
            .iter()
            .map(|author| RedisCommandSpec {
                name: author.name.to_string(),
                subcommands: author
                    .subcommands
                    .iter()
                    .map(|(name, args)| RedisSubcommand {
                        name: name.to_string(),
                        arguments: parse_redis_args(args),
                    })
                    .collect(),
                arguments: parse_redis_args(author.arguments),
            })
            .collect()
    })
}

/// 根据大写命令名查找命令定义（归一化查找入口）。
pub fn redis_command_spec(name: &str) -> Option<&'static RedisCommandSpec> {
    redis_commands().iter().find(|spec| spec.name == name)
}

/// 一条命令的完整 synopsis（命令名 + 参数骨架）。
pub fn redis_command_synopsis(spec: &RedisCommandSpec) -> String {
    let args = spec
        .arguments
        .iter()
        .map(redis_arg_synopsis)
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        spec.name.clone()
    } else {
        format!("{} {}", spec.name, args)
    }
}

/// 子命令的 synopsis（子命令名 + 其参数骨架）。
fn redis_subcommand_synopsis(spec: &RedisCommandSpec, sub: &RedisSubcommand) -> String {
    let args = sub
        .arguments
        .iter()
        .map(redis_arg_synopsis)
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        format!("{} {}", spec.name, sub.name)
    } else {
        format!("{} {} {}", spec.name, sub.name, args)
    }
}

/// 递归消费 tokens[ti..]：返回是否匹配并推进到的新 ti。
///
/// 用于参数定位：决定某条参数（及其嵌套子参数）是否吃下了当前输入。
fn redis_arg_steps(arg: &RedisArg, tokens: &[String], ti: usize) -> Option<usize> {
    match arg.rtype {
        RedisArgType::Value => (ti < tokens.len()).then_some(ti + 1),
        RedisArgType::Token => {
            (ti < tokens.len() && tokens[ti] == arg.display).then_some(ti + 1)
        }
        RedisArgType::OneOf => arg
            .children
            .iter()
            .find_map(|child| redis_arg_steps(child, tokens, ti)),
        RedisArgType::Block => {
            let mut cur = ti;
            for child in &arg.children {
                if let Some(nti) = redis_arg_steps(child, tokens, cur) {
                    cur = nti;
                } else if !child.optional {
                    // 块内必填子参数未被匹配：块整体暂时停住，返回已消费的位置。
                    break;
                }
            }
            Some(cur)
        }
    }
}

/// 从参数列表与「已输入的参数 tokens」返回当前候选前沿（一次可能容纳多个待补参数）。
///
/// 之所以返回「前沿」而非单个参数：命令尾部常有一串可选 token（如
/// `SET <key> <value> [EX ...] [NX|XX] [GET]`），用户正在输入任一可选 token 时，
/// 该 token 及其后的可选参数都可能是当前应补的目标。让调用方统一收集前沿里所有
/// 参数的 token 候选，再按用户前缀过滤，即可让 `SET k v N` 正确联想 `NX`。
///
/// 返回值借用自惰性构建且永驻的静态命令树，可安全地作为 `&'static` 生命周期传递。
fn redis_locate_args(args: &'static [RedisArg], tokens: &[String]) -> Vec<&'static RedisArg> {
    let mut ti = 0usize;
    // 最后一个被消费的 multiple 参数：若 tokens 溢出到它之后，仍停留在其上供重复。
    let mut last_multiple: Option<&'static RedisArg> = None;

    for (i, arg) in args.iter().enumerate() {
        if ti >= tokens.len() {
            // 所有已输入 token 消费完：从当前位置起到末尾的剩余参数都是候选前沿。
            return args[i..].iter().collect();
        }
        if let Some(nti) = redis_arg_steps(arg, tokens, ti) {
            ti = nti;
            if arg.multiple {
                last_multiple = Some(arg);
            }
            continue;
        }
        // 当前参数不匹配该 token。
        if arg.optional {
            // 可选且未触发：跳过，继续看后续参数。
            continue;
        }
        match arg.rtype {
            // 必填取值参数总会吃掉一个 token。
            RedisArgType::Value => {
                ti += 1;
                continue;
            }
            // 必填 token / oneof / block 当前未匹配：应停留在它上面补 token。
            _ => return vec![arg],
        }
    }

    // 参数遍历完仍有剩余 token：若存在可重复的参数则停留在其上，否则已配齐。
    if ti < tokens.len() {
        match last_multiple {
            Some(m) => vec![m],
            None => Vec::new(),
        }
    } else {
        // 恰好消费完：最后一个若是 multiple 则可继续补，否则无后续候选。
        match args.last() {
            Some(last) if last.multiple => vec![last],
            _ => Vec::new(),
        }
    }
}

/// 取一个参数节点里可补的 token 候选（不含取值参数）。
fn redis_arg_candidates(arg: &RedisArg) -> Vec<String> {
    match arg.rtype {
        RedisArgType::Token => vec![arg.display.clone()],
        RedisArgType::OneOf => arg
            .children
            .iter()
            .filter_map(|c| match c.rtype {
                RedisArgType::Token => Some(c.display.clone()),
                _ => None,
            })
            .collect(),
        RedisArgType::Block => arg
            .children
            .iter()
            .flat_map(redis_arg_candidates)
            .collect(),
        RedisArgType::Value => Vec::new(),
    }
}
