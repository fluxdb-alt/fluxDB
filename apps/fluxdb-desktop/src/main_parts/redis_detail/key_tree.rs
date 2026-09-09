// Redis Key 前缀折叠树（多级）：把「平铺」的已过滤 key 行按分隔符递归折叠成多级 folder/leaf 树，
// 再按 tab 展开集合拍平成可见行。每一级分割都向下折叠一层 folder（如 `demo:orders:list`
// → `demo` → `orders` → `list`）；最后一层的 leaf 只显示最后一段（label），完整 key 仍保留给
// 详情/删除/刷新反查。本文件只保留纯函数建树 + 拍平能力，与具体 UI 渲染解耦，便于单测。
// 分隔符不写死：统一走 `RedisKeyDelimiter` 配置（默认 `:`），后续换分隔符只改一处。
// 参考 RedisInsight `constructKeysToTree` / dbx `redisKeyTree` 的建树思路，但不照搬其架构。

/// Redis Key 列表展示模式：平铺（默认）/ Folder。两种模式都保留在同一列表内切换。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisKeyListMode {
    Flat,
    Folder,
}

impl RedisKeyListMode {
    fn is_folder(self) -> bool {
        matches!(self, RedisKeyListMode::Folder)
    }
}

/// 树节点：`Folder` 携带完整前缀路径（prefix，作展开态 key）与本段名（segment，作展示）；
/// `Leaf` 携带完整 key（key，作详情/删除反查）与最后一段（segment，作展示）及原始行下标。
#[derive(Clone, Debug, PartialEq)]
enum RedisKeyListNode {
    Folder {
        prefix: String,
        segment: String,
        count: usize,
        children: Vec<RedisKeyListNode>,
    },
    Leaf {
        key: String,
        segment: String,
        source_row: usize,
    },
}

/// 拍平后的可见行类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisKeyRowKind {
    Folder,
    Leaf,
}

/// 拍平后的可见行：folder 携带完整前缀路径（prefix，展开态 key）+ 本段展示名（label）；
/// leaf 携带展示用的最后一段（label）+ 完整 key（key）+ 原始行下标。
#[derive(Clone, Debug, PartialEq)]
struct RedisKeyVisibleRow {
    kind: RedisKeyRowKind,
    depth: usize,
    prefix: String,
    count: usize,
    label: String,
    key: String,
    source_row: usize,
}

/// 前缀 trie 节点：`leaf` 为「该前缀恰好是完整键」的原始行下标（None 表示无完整键），
/// `children` 按下一段前缀分叉。用 BTreeMap 保证兄弟节点输出顺序稳定（按段名字典序）。
#[derive(Default, Debug)]
struct TrieNode {
    leaf: Option<usize>,
    children: BTreeMap<String, TrieNode>,
}

/// Redis Key 前缀折叠的分隔符配置（**统一入口**）：建树拆分与完整路径重建都走它，
/// 避免把分隔符散落写死在多处。默认 `:`，后续支持其他分隔符只改默认值/注入点即可。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RedisKeyDelimiter {
    /// 分隔字符。
    ch: char,
}

impl Default for RedisKeyDelimiter {
    fn default() -> Self {
        RedisKeyDelimiter { ch: ':' }
    }
}

impl RedisKeyDelimiter {
    /// 把 key 按分隔符切分成段。
    fn split<'a>(&self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        key.split(self.ch)
    }

    /// 用分隔符把父级完整路径与本段名拼成完整路径；父级为空直接返回本段名。
    fn join_path(&self, parent: &str, segment: &str) -> String {
        if parent.is_empty() {
            segment.to_string()
        } else {
            format!("{parent}{}{segment}", self.ch)
        }
    }
}

/// 从 (键名, 原始行下标) 列表按分隔符递归建多级前缀树：每个段是一层，叶子在最后一层。
fn redis_key_build_tree(entries: &[(String, usize)], delim: RedisKeyDelimiter) -> Vec<RedisKeyListNode> {
    let mut root = TrieNode::default();
    for (key, source_row) in entries {
        let mut cur = &mut root;
        for segment in delim.split(key) {
            cur = cur.children.entry(segment.to_string()).or_default();
        }
        cur.leaf = Some(*source_row);
    }
    redis_key_convert_nodes(delim, "", &root.children)
}

/// 把一层的所有子节点转换为「folder 先于 leaf」的节点列表；parent 为父级完整路径前缀。
/// 有子节点的节点生成 Folder（其 count 为子树叶子总数，prefix 为完整路径，segment 为本段展示名）；
/// 自身是完整键的节点生成 Leaf（key 为完整路径即完整键，segment 为本段展示名）。
fn redis_key_convert_nodes(
    delim: RedisKeyDelimiter,
    parent: &str,
    children: &BTreeMap<String, TrieNode>,
) -> Vec<RedisKeyListNode> {
    let mut folders = Vec::new();
    let mut leaves = Vec::new();
    for (segment, child) in children {
        let path = delim.join_path(parent, segment);
        if !child.children.is_empty() {
            let sub = redis_key_convert_nodes(delim, &path, &child.children);
            let count = sub.iter().map(redis_key_leaves_of).sum();
            folders.push(RedisKeyListNode::Folder {
                prefix: path.clone(),
                segment: segment.clone(),
                count,
                children: sub,
            });
        }
        if let Some(source_row) = child.leaf {
            leaves.push(RedisKeyListNode::Leaf {
                key: path.clone(),
                segment: segment.clone(),
                source_row,
            });
        }
    }
    // 同层 folder 永远排在 leaf 前面；BTreeMap 天然按段名字典序，分区后组内顺序不变。
    folders.extend(leaves);
    folders
}

/// 单个节点的叶子总数：`Leaf` 计 1，`Folder` 用其缓存的 count。
fn redis_key_leaves_of(node: &RedisKeyListNode) -> usize {
    match node {
        RedisKeyListNode::Leaf { .. } => 1,
        RedisKeyListNode::Folder { count, .. } => *count,
    }
}

/// 按展开集合把多级树递归拍平成可见行；`expanded` 是已展开的 folder 完整路径集合
/// （切回平铺不清空）。folder 的子项只有在其完整路径在展开集合里时才继续下钻。
fn redis_key_flatten(
    nodes: &[RedisKeyListNode],
    expanded: &BTreeSet<String>,
    depth: usize,
    out: &mut Vec<RedisKeyVisibleRow>,
) {
    for node in nodes {
        match node {
            RedisKeyListNode::Folder {
                prefix,
                segment,
                count,
                children,
            } => {
                out.push(RedisKeyVisibleRow {
                    kind: RedisKeyRowKind::Folder,
                    depth,
                    prefix: prefix.clone(),
                    count: *count,
                    label: segment.clone(),
                    key: String::new(),
                    source_row: 0,
                });
                if expanded.contains(prefix) {
                    redis_key_flatten(children, expanded, depth + 1, out);
                }
            }
            RedisKeyListNode::Leaf {
                key,
                segment,
                source_row,
            } => {
                out.push(RedisKeyVisibleRow {
                    kind: RedisKeyRowKind::Leaf,
                    depth,
                    prefix: String::new(),
                    count: 0,
                    label: segment.clone(),
                    key: key.clone(),
                    source_row: *source_row,
                });
            }
        }
    }
}

#[cfg(test)]
mod redis_key_tree_tests {
    use super::*;

    /// 便捷构建：键名列表转 (键名, 下标) 输入。
    fn entries(keys: &[&str]) -> Vec<(String, usize)> {
        keys.iter().enumerate().map(|(i, k)| (k.to_string(), i)).collect()
    }

    /// 默认分隔符 `:`。
    fn delim() -> RedisKeyDelimiter {
        RedisKeyDelimiter::default()
    }

    fn visible_keys(nodes: &[RedisKeyListNode], expanded: &BTreeSet<String>) -> Vec<String> {
        let mut rows = Vec::new();
        redis_key_flatten(nodes, expanded, 0, &mut rows);
        rows.into_iter()
            .map(|row| match row.kind {
                RedisKeyRowKind::Folder => format!("[{}]{}", row.label, row.count),
                RedisKeyRowKind::Leaf => row.label,
            })
            .collect()
    }

    #[test]
    fn flat_and_single_segment_keys_are_not_grouped() {
        let tree = redis_key_build_tree(&entries(&["alpha", "beta", "gamma"]), delim());
        // 无 `:` 分隔时不产生 folder，全部为顶层叶子且按名字排列。
        let rows = visible_keys(&tree, &BTreeSet::new());
        assert_eq!(rows, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn multi_level_tree_folds_every_segment_into_folder() {
        // demo:orders:list / demo:config → demo(folder) -> orders(folder) -> list(leaf)，config(leaf)。
        let keys = &["demo:orders:list", "demo:config"];
        let tree = redis_key_build_tree(&entries(keys), delim());
        let rows = visible_keys(&tree, &BTreeSet::new());
        assert_eq!(rows, vec!["[demo]2"]);
        // 只展开 demo：orders 仍是 folder（多级保留），config 是叶子。
        let expanded = BTreeSet::from(["demo".to_string()]);
        let rows = visible_keys(&tree, &expanded);
        assert_eq!(rows, vec!["[demo]2", "[orders]1", "config"]);
        // 再展开 demo:orders：list 出现。
        let expanded = BTreeSet::from(["demo".to_string(), "demo:orders".to_string()]);
        let rows = visible_keys(&tree, &expanded);
        assert_eq!(rows, vec!["[demo]2", "[orders]1", "list", "config"]);
    }

    #[test]
    fn folder_before_leaf_same_level() {
        // `app` 既是完整键又是文件夹：顶层 folder app 排在 leaf app 前。
        let keys = &["app", "app:a", "app:z"];
        let tree = redis_key_build_tree(&entries(keys), delim());
        let rows = visible_keys(&tree, &BTreeSet::new());
        assert_eq!(rows, vec!["[app]2", "app"]);
        // 展开 app：其下叶子 a / z 可见，顶层 leaf app 保留在最后。
        let expanded = BTreeSet::from(["app".to_string()]);
        let rows = visible_keys(&tree, &expanded);
        assert_eq!(rows, vec!["[app]2", "a", "z", "app"]);
    }

    #[test]
    fn sorting_folders_then_leaves_by_name() {
        // 顶层：folder(a/b) 先于 leaf(y/z)，各自按名字排序。
        let keys = &["b:1", "a:1", "z", "y"];
        let tree = redis_key_build_tree(&entries(keys), delim());
        let rows = visible_keys(&tree, &BTreeSet::new());
        assert_eq!(rows, vec!["[a]1", "[b]1", "y", "z"]);
    }

    #[test]
    fn custom_delimiter_is_supported() {
        // 分隔符是可配置的：换成 `/` 也能建多级树，证明不写死 `:`（改动只在一处 delimiter）。
        let keys = &["demo/orders/list", "demo/config", "token"];
        let dot = RedisKeyDelimiter { ch: '/' };
        let tree = redis_key_build_tree(&entries(keys), dot);
        let rows = visible_keys(&tree, &BTreeSet::new());
        assert_eq!(rows, vec!["[demo]2", "token"]);
        let expanded = BTreeSet::from(["demo".to_string(), "demo/orders".to_string()]);
        let rows = visible_keys(&tree, &expanded);
        assert_eq!(rows, vec!["[demo]2", "[orders]1", "list", "config", "token"]);
    }

    #[test]
    fn expand_state_is_independent_of_mode_switch() {
        // 展开集合在拍平之间保持，切回平铺（不解构树）时数据不丢失：此处验证重复拍平幂等。
        let keys = &["u:1", "u:2"];
        let tree = redis_key_build_tree(&entries(keys), delim());
        let expanded = BTreeSet::from(["u".to_string()]);
        let mut a = Vec::new();
        let mut b = Vec::new();
        redis_key_flatten(&tree, &expanded, 0, &mut a);
        redis_key_flatten(&tree, &expanded, 0, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn leaf_shows_last_segment_and_keeps_full_key() {
        // label 只展示最后一段，完整 key + source_row 保留，供详情/删除反查正确行。
        let keys = &["demo:orders:list", "demo:config", "token"];
        let tree = redis_key_build_tree(&entries(keys), delim());
        let expanded = BTreeSet::from(["demo".to_string(), "demo:orders".to_string()]);
        let mut rows = Vec::new();
        redis_key_flatten(&tree, &expanded, 0, &mut rows);
        let triples: Vec<(&str, &str, usize)> = rows
            .iter()
            .filter(|r| r.kind == RedisKeyRowKind::Leaf)
            .map(|r| (r.label.as_str(), r.key.as_str(), r.source_row))
            .collect();
        assert!(triples.contains(&("list", "demo:orders:list", 0)));
        assert!(triples.contains(&("config", "demo:config", 1)));
        assert!(triples.contains(&("token", "token", 2)));
    }
}
