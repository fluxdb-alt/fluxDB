//! 通用语言注册表和语法层描述。
//!
//! 注册表只保存能力对象，不依赖 GPUI 或具体业务；宿主根据
//! `EditorProfile::language_id` 解析语言定义和语法 provider。

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::model::{Offset, Range};
use crate::syntax::{LanguageDefinition, SyntaxProvider};

/// 一个可嵌套的语法层范围。范围外的文本不会交给该语言 provider。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxLayer {
    pub language_id: String,
    pub range: Range,
    pub priority: u8,
}

impl SyntaxLayer {
    pub fn new(language_id: impl Into<String>, range: Range) -> Self {
        Self {
            language_id: language_id.into(),
            range,
            priority: 0,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
}

/// 语法注入描述；高优先级注入覆盖同一位置上的低优先级层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxInjection {
    pub language_id: String,
    pub range: Range,
    pub priority: u8,
}

impl SyntaxInjection {
    pub fn new(language_id: impl Into<String>, range: Range) -> Self {
        Self {
            language_id: language_id.into(),
            range,
            priority: 0,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
}

/// 语言注册信息。language 必须存在，syntax 可为空以支持纯文本语言。
/// 语法层树（DM-620）：多个可嵌套语法层的只读归一视图。
///
/// 由父语言的 `SyntaxProvider::injections` 构建，用于「按 offset 定位语言」
/// （高亮/诊断/hover 路由，DM-624）与「按编辑范围找相交层」（DM-622）。层区间
/// 由 provider 保证互不重叠；`from_injections` 只做归一排序。空注入时层树为空，
/// 单语言路径零开销（零全文扫描）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyntaxLayerTree {
    layers: Vec<SyntaxLayer>,
}

impl SyntaxLayerTree {
    /// 从注入描述归一化构建层树：按 range.start 升序；同一 range 冲突时保留高
    /// priority 层。
    pub fn from_injections<I>(injections: I) -> Self
    where
        I: IntoIterator<Item = SyntaxInjection>,
    {
        let mut layers: Vec<SyntaxLayer> = injections
            .into_iter()
            .map(|i| SyntaxLayer {
                language_id: i.language_id,
                range: i.range,
                priority: i.priority,
            })
            .collect();
        // 同一 range：高 priority 覆盖低 priority（同 priority 保后者）。
        layers.sort_by(|a, b| {
            a.range
                .start
                .cmp(&b.range.start)
                .then(b.range.end.cmp(&a.range.end))
                .then(b.priority.cmp(&a.priority))
        });
        // 去重：range 相同（并集区间重合开始）时只保留第一个（已按 priority 高者在前）。
        let mut out: Vec<SyntaxLayer> = Vec::with_capacity(layers.len());
        for layer in layers {
            if let Some(last) = out.last() {
                if last.range.start == layer.range.start && last.range.end == layer.range.end {
                    continue;
                }
            }
            out.push(layer);
        }
        Self { layers: out }
    }

    /// 层树是否为空（无注入）。
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// 层数。
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// 定位包含 `offset` 的语言层：最高 priority（并列取区间最小/最先）。无则 None。
    pub fn language_at(&self, offset: Offset) -> Option<&SyntaxLayer> {
        // 已按 start 升序 + 区间从大到小排序；取第一个 range 含 offset 的层即最优。
        let mut best: Option<&SyntaxLayer> = None;
        let mut best_priority = 0u8;
        for layer in &self.layers {
            if layer.range.contains(offset) {
                if best.is_none() || layer.priority > best_priority {
                    best = Some(layer);
                    best_priority = layer.priority;
                }
            }
        }
        best
    }

    /// 与 `range` 相交的所有层（DM-622：编辑只使相交的层失效/重解析）。
    pub fn layers_intersecting(&self, range: Range) -> Vec<&SyntaxLayer> {
        let mut out = Vec::new();
        for layer in &self.layers {
            let l_start = layer.range.start;
            let l_end = layer.range.end;
            // 区间相交：layer.end >= range.start 且 layer.start <= range.end。
            if l_end >= range.start && l_start <= range.end {
                out.push(layer);
            }
        }
        out
    }
}

/// 语言注册信息。language 必须存在，syntax 可为空以支持纯文本语言。
#[derive(Clone)]
pub struct LanguageRegistration {
    pub language: Arc<dyn LanguageDefinition>,
    pub syntax: Option<Arc<dyn SyntaxProvider>>,
}

/// 进程内语言注册表；clone 只复制共享句柄，适合注入多个编辑器实例。
#[derive(Clone, Default)]
pub struct LanguageRegistry {
    entries: Arc<RwLock<BTreeMap<String, LanguageRegistration>>>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册或替换 language_id，返回被替换的旧条目。
    pub fn register(
        &self,
        language_id: impl Into<String>,
        language: Arc<dyn LanguageDefinition>,
        syntax: Option<Arc<dyn SyntaxProvider>>,
    ) -> Option<LanguageRegistration> {
        let id = language_id.into();
        self.entries
            .write()
            .ok()?
            .insert(id, LanguageRegistration { language, syntax })
    }

    pub fn get(&self, language_id: &str) -> Option<LanguageRegistration> {
        self.entries.read().ok()?.get(language_id).cloned()
    }

    pub fn language(&self, language_id: &str) -> Option<Arc<dyn LanguageDefinition>> {
        self.get(language_id).map(|entry| entry.language)
    }

    pub fn syntax(&self, language_id: &str) -> Option<Arc<dyn SyntaxProvider>> {
        self.get(language_id).and_then(|entry| entry.syntax)
    }

    pub fn language_ids(&self) -> Vec<String> {
        self.entries
            .read()
            .map(|entries| entries.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;

    impl LanguageDefinition for Plain {
        fn language_id(&self) -> &str {
            "plain"
        }
    }

    #[test]
    fn registry_replaces_and_resolves_language() {
        let registry = LanguageRegistry::new();
        let first: Arc<dyn LanguageDefinition> = Arc::new(Plain);
        assert!(registry.register("plain", first, None).is_none());
        assert_eq!(registry.language_ids(), vec!["plain"]);
        assert_eq!(registry.language("plain").unwrap().language_id(), "plain");

        let replacement: Arc<dyn LanguageDefinition> = Arc::new(Plain);
        assert!(registry.register("plain", replacement, None).is_some());
    }

    #[test]
    fn syntax_layer_keeps_priority_and_range() {
        let layer = SyntaxLayer::new("sql", Range::new(4, 12)).with_priority(3);
        assert_eq!(layer.language_id, "sql");
        assert_eq!(layer.range, Range::new(4, 12));
        assert_eq!(layer.priority, 3);
    }

    /// DM-620：from_injections 归一排序；空注入 → 空层树。
    #[test]
    fn layer_tree_from_injections_normalizes() {
        let tree = SyntaxLayerTree::from_injections([
            SyntaxInjection::new("sub", Range::new(10, 20)),
            SyntaxInjection::new("head", Range::new(0, 5)),
        ]);
        assert_eq!(tree.len(), 2);
        // 已按 start 升序。
        assert_eq!(tree.layers[0].language_id, "head");
        assert_eq!(tree.layers[1].language_id, "sub");

        let empty = SyntaxLayerTree::from_injections([]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
    }

    /// DM-620：同一 range 冲突时高 priority 覆盖低 priority（去重）。
    #[test]
    fn layer_tree_priority_wins_on_same_range() {
        let tree = SyntaxLayerTree::from_injections([
            SyntaxInjection::new("low", Range::new(5, 15)).with_priority(1),
            SyntaxInjection::new("high", Range::new(5, 15)).with_priority(5),
        ]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.layers[0].language_id, "high");
    }

    /// DM-624：language_at 返回覆盖 offset 的最高 priority 层；越界返回 None。
    #[test]
    fn layer_tree_language_at_picks_covering_highest_priority() {
        let tree = SyntaxLayerTree::from_injections([
            SyntaxInjection::new("parent", Range::new(0, 100)).with_priority(1),
            SyntaxInjection::new("child", Range::new(10, 30)).with_priority(5),
        ]);
        // child 范围内 → child（更高 priority）。
        assert_eq!(tree.language_at(15usize).unwrap().language_id, "child");
        // parent 专属区 → parent。
        assert_eq!(tree.language_at(60usize).unwrap().language_id, "parent");
        // 越界 → None。
        assert!(tree.language_at(200usize).is_none());
    }

    /// DM-622：layers_intersecting 只返回与编辑范围相交的层。
    #[test]
    fn layer_tree_intersecting_reports_only_intersected() {
        let tree = SyntaxLayerTree::from_injections([
            SyntaxInjection::new("a", Range::new(0, 10)),
            SyntaxInjection::new("b", Range::new(20, 30)),
            SyntaxInjection::new("c", Range::new(40, 50)),
        ]);
        let hit = tree.layers_intersecting(Range::new(25, 45));
        let ids: Vec<&str> = hit.iter().map(|l| l.language_id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"]);
    }
}
