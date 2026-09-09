#[derive(Clone, Debug, PartialEq)]
struct RedisKeyDetail {
    key: String,
    kind: String,
    value: String,
    ttl: String,
    /// 键内存占用（已换算为可读大小，如 `12.5 KB`），来自连接列表「大小」列；无值时空串。
    size: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisKeyDetailRefreshKind {
    Key,
    Hash,
    Set,
    ZSet,
    List,
    Stream,
}

trait RedisKeyDetailRefresh {
    fn refresh(self, tab_id: TabId, key: String, this: &mut NavicatMain, cx: &mut Context<NavicatMain>);
}

impl RedisKeyDetailRefresh for RedisKeyDetailRefreshKind {
    fn refresh(self, tab_id: TabId, key: String, this: &mut NavicatMain, cx: &mut Context<NavicatMain>) {
        match self {
            RedisKeyDetailRefreshKind::Key => {}
            RedisKeyDetailRefreshKind::Hash => this.rerun_redis_hash_field_search(tab_id, key, cx),
            RedisKeyDetailRefreshKind::Set => this.rerun_redis_set_member_search(tab_id, key, cx),
            RedisKeyDetailRefreshKind::ZSet => this.rerun_redis_zset_member_search(tab_id, key, cx),
            RedisKeyDetailRefreshKind::List => this.rerun_redis_list_item_search(tab_id, key, cx),
            RedisKeyDetailRefreshKind::Stream => this.rerun_redis_stream_entry_search(tab_id, key, cx),
        }
    }
}

impl RedisKeyDetail {
    fn refresh_kind(&self) -> RedisKeyDetailRefreshKind {
        if self.kind.eq_ignore_ascii_case("hash") {
            RedisKeyDetailRefreshKind::Hash
        } else if self.kind.eq_ignore_ascii_case("set") {
            RedisKeyDetailRefreshKind::Set
        } else if self.kind.eq_ignore_ascii_case("zset") {
            RedisKeyDetailRefreshKind::ZSet
        } else if self.kind.eq_ignore_ascii_case("list") {
            RedisKeyDetailRefreshKind::List
        } else if self.kind.eq_ignore_ascii_case("stream") {
            RedisKeyDetailRefreshKind::Stream
        } else {
            RedisKeyDetailRefreshKind::Key
        }
    }
}
