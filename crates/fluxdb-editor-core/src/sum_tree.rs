//! 通用持久 SumTree。
//!
//! 文本、显示映射和高亮共享同一棵分块树实现。叶子保存 item，节点保存摘要；
//! 替换只复制 dirty 路径，带 offset 的 item 还可以通过节点 lazy delta 延迟平移。

use std::sync::Arc;

const FANOUT: usize = 32;

pub(crate) trait Summary: Clone + Default + PartialEq + Eq + std::fmt::Debug {
    fn add(&self, other: &Self) -> Self;
}

/// 区间型摘要：子树的 `[start, end)` 包络。供按偏移定位/相交路径替换使用
/// （DM-202：fold add/remove/edit 只替换相交 transform path）。
pub(crate) trait IntervalSummary: Summary {
    fn start(&self) -> usize;
    fn end(&self) -> usize;
}

pub(crate) trait SumTreeItem: Clone {
    type Summary: Summary;

    fn summary(&self) -> Self::Summary;
}

/// 为需要 offset 平移的索引项提供统一的 lazy/edit 行为。
pub(crate) trait OffsetItem: SumTreeItem {
    fn range(summary: &Self::Summary) -> Option<(usize, usize)>;
    fn shift_summary(summary: &Self::Summary, delta: isize) -> Self::Summary;
    fn apply_delta(&mut self, delta: isize);
    fn apply_edit(&mut self, old_start: usize, old_end: usize, byte_delta: isize) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSummary {
    pub bytes: usize,
    pub lines: usize,
    pub utf16: usize,
}

impl Summary for TextSummary {
    fn add(&self, other: &Self) -> Self {
        Self {
            bytes: self.bytes.saturating_add(other.bytes),
            lines: self.lines.saturating_add(other.lines),
            utf16: self.utf16.saturating_add(other.utf16),
        }
    }
}

impl SumTreeItem for TextSummary {
    type Summary = Self;

    fn summary(&self) -> Self::Summary {
        *self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SumTree<I: SumTreeItem> {
    root: Arc<Node<I>>,
    leaf_count: usize,
}

impl<I: SumTreeItem> PartialEq for SumTree<I> {
    fn eq(&self, other: &Self) -> bool {
        self.leaf_count == other.leaf_count && self.total() == other.total()
    }
}

impl<I: SumTreeItem> Eq for SumTree<I> {}

#[derive(Clone, Debug)]
enum Node<I: SumTreeItem> {
    Leaf {
        items: Arc<Vec<I>>,
        total: I::Summary,
        lazy: isize,
    },
    Branch {
        children: Arc<Vec<Arc<Node<I>>>>,
        total: I::Summary,
        leaf_count: usize,
        lazy: isize,
    },
}

impl<I: SumTreeItem> Default for SumTree<I> {
    fn default() -> Self {
        Self {
            root: make_leaf(&[]),
            leaf_count: 0,
        }
    }
}

impl<I: SumTreeItem> SumTree<I> {
    pub(crate) fn from_items(items: &[I]) -> Self {
        let root = build_node(items);
        Self {
            root,
            leaf_count: items.len(),
        }
    }

    pub(crate) fn replace_leaves(&self, start: usize, end: usize, inserted: &[I]) -> Self {
        let start = start.min(self.leaf_count);
        let end = end.max(start).min(self.leaf_count);
        let nodes = replace_node(&self.root, start, end, inserted);
        let root = pack_root(nodes);
        Self {
            leaf_count: root.leaf_count(),
            root,
        }
    }

    pub(crate) fn total(&self) -> I::Summary {
        self.root.total().clone()
    }

    pub(crate) fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    pub(crate) fn summary_before_leaf(&self, leaf: usize) -> I::Summary {
        summary_before_node(&self.root, leaf.min(self.leaf_count))
    }

    pub(crate) fn get_ref(&self, index: usize) -> Option<&I> {
        (index < self.leaf_count)
            .then(|| get_leaf_ref(&self.root, index))
            .flatten()
    }

    #[cfg(test)]
    pub(crate) fn shared_trailing_leaves(&self, other: &Self) -> usize {
        let a = leaf_ptrs(&self.root);
        let b = leaf_ptrs(&other.root);
        let n = a.len().min(b.len());
        let mut count = 0;
        for i in 0..n {
            if a[a.len() - 1 - i] != b[b.len() - 1 - i] {
                break;
            }
            count += 1;
        }
        count
    }
}

impl<I: SumTreeItem> SumTree<I>
where
    I::Summary: IntervalSummary,
{
    /// 按序展开所有叶子 item（O(n)，用于迁移/遍历/验证）。
    pub(crate) fn to_vec(&self) -> Vec<I> {
        let mut out = Vec::with_capacity(self.leaf_count);
        collect_leaves(&self.root, &mut out);
        out
    }

    /// 读取第 `index` 个叶子 item（叶子序，0-based）。
    pub(crate) fn get(&self, index: usize) -> Option<I> {
        if index >= self.leaf_count {
            return None;
        }
        get_leaf_item(&self.root, index)
    }

    /// 第一个 `start >= target` 的叶子下标（叶子按 start 升序）。所有 item 的
    /// `start < target` 时返回 `leaf_count`。
    pub(crate) fn lower_bound_start(&self, target: usize) -> usize {
        lower_bound_start_node(&self.root, target)
    }

    /// 与 `other` 共享的**尾部连续叶子节点**个数。逐个叶子节点比较其 item 容器的
    /// `Arc` 指针（`Arc::ptr_eq`），从末尾往头数，遇不同即停。
    ///
    /// 每个叶子节点含 ≤ [`FANOUT`] 条 item，因此共享 1 个叶子节点即代表该批量
    /// item 所属子树从未被触碰。用于证明局部编辑只替换相交路径、未触碰的后缀
    /// 子树全程 `Arc` 共享（DM-202）。
    /// 与区间 `range=[s,e)` 相交（含相触，`item.end >= s && item.start <= e`）的
    /// 叶子下标范围 `(first, last_exclusive)`。仅访问相交子树路径及足可剪枝的
    /// 邻居（非全量遍历）——这是「只替换相交 transform path」的定位基础。无相交
    /// 返回 `None`（此时用 [`SumTree::lower_bound_start`] 定插入点）。
    pub(crate) fn intersecting_leaf_span(&self, range: (usize, usize)) -> Option<(usize, usize)> {
        let (qs, qe) = range;
        let mut first = usize::MAX; // 以 MAX 作哨兵：first 走 min 累加，MAX 即「未命中」
        let mut last = 0usize; // last 走 max 累加，从 0 起；命中即为最大命中叶子下标
        intersecting_span_node(&self.root, qs, qe, 0, &mut first, &mut last);
        if first == usize::MAX {
            return None;
        }
        Some((first, last.saturating_add(1)))
    }
}

impl SumTree<TextSummary> {
    pub(crate) fn from_summaries(summaries: &[TextSummary]) -> Self {
        Self::from_items(summaries)
    }

    pub(crate) fn locate_byte(&self, offset: usize) -> (usize, usize) {
        if self.leaf_count == 0 {
            return (0, 0);
        }
        locate_node(&self.root, offset.min(self.total().bytes), 0, |summary| {
            summary.bytes
        })
    }

    pub(crate) fn locate_utf16(&self, offset: usize) -> (usize, usize) {
        if self.leaf_count == 0 {
            return (0, 0);
        }
        locate_node(&self.root, offset.min(self.total().utf16), 0, |summary| {
            summary.utf16
        })
    }

    pub(crate) fn locate_lines(&self, offset: usize) -> (usize, usize) {
        if self.leaf_count == 0 {
            return (0, 0);
        }
        locate_node(&self.root, offset.min(self.total().lines), 0, |summary| {
            summary.lines
        })
    }

    pub(crate) fn byte_start(&self, leaf: usize) -> usize {
        self.summary_before_leaf(leaf).bytes
    }
}

impl<I: OffsetItem> SumTree<I> {
    pub(crate) fn apply_edit(&mut self, old_range: (usize, usize), new_len: usize) {
        let old_start = old_range.0;
        let old_end = old_range.1.max(old_start);
        let byte_delta = new_len as isize - old_end.saturating_sub(old_start) as isize;
        self.root = edit_node(&self.root, old_start, old_end, byte_delta);
    }

    pub(crate) fn iter_intersecting(&self, range: (usize, usize)) -> Vec<I> {
        let mut result = Vec::new();
        collect_intersecting(&self.root, 0, range, &mut result);
        result
    }

    /// 展开叶子下标 `[start, end)` 内的 item（DM-400/402：dirty-range 高亮替换的
    /// 局部读取）。与 `collect_intersecting` 一样将节点级 lazy delta 物化进返回 item，
    /// 因此返回的是**绝对坐标**，可直接用于替换合并。
    pub(crate) fn leaf_span_items(&self, start: usize, end: usize) -> Vec<I> {
        let start = start.min(self.leaf_count);
        let end = end.max(start).min(self.leaf_count);
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        collect_leaf_span(&self.root, 0, start, end, 0, &mut out);
        out
    }
}

impl<I: SumTreeItem> Node<I> {
    fn total(&self) -> &I::Summary {
        match self {
            Self::Leaf { total, .. } | Self::Branch { total, .. } => total,
        }
    }

    fn lazy(&self) -> isize {
        match self {
            Self::Leaf { lazy, .. } | Self::Branch { lazy, .. } => *lazy,
        }
    }

    fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf { items, .. } => items.len(),
            Self::Branch { leaf_count, .. } => *leaf_count,
        }
    }
}

fn build_node<I: SumTreeItem>(items: &[I]) -> Arc<Node<I>> {
    if items.len() <= FANOUT {
        return make_leaf(items);
    }
    let children: Vec<_> = items.chunks(FANOUT).map(build_node).collect();
    make_branch(children)
}

fn make_leaf<I: SumTreeItem>(items: &[I]) -> Arc<Node<I>> {
    let total = items
        .iter()
        .map(SumTreeItem::summary)
        .fold(I::Summary::default(), |acc, item| acc.add(&item));
    Arc::new(Node::Leaf {
        items: Arc::new(items.to_vec()),
        total,
        lazy: 0,
    })
}

fn make_branch<I: SumTreeItem>(children: Vec<Arc<Node<I>>>) -> Arc<Node<I>> {
    debug_assert!(!children.is_empty());
    let total = children
        .iter()
        .map(|child| child.total().clone())
        .fold(I::Summary::default(), |acc, item| acc.add(&item));
    let leaf_count = children.iter().map(|child| child.leaf_count()).sum();
    Arc::new(Node::Branch {
        children: Arc::new(children),
        total,
        leaf_count,
        lazy: 0,
    })
}

fn pack_level<I: SumTreeItem>(nodes: Vec<Arc<Node<I>>>) -> Vec<Arc<Node<I>>> {
    if nodes.len() <= FANOUT {
        return nodes;
    }
    nodes
        .chunks(FANOUT)
        .map(|chunk| make_branch(chunk.to_vec()))
        .collect()
}

fn pack_root<I: SumTreeItem>(mut nodes: Vec<Arc<Node<I>>>) -> Arc<Node<I>> {
    if nodes.is_empty() {
        return make_leaf(&[]);
    }
    while nodes.len() > FANOUT {
        nodes = pack_level(nodes);
    }
    if nodes.len() == 1 {
        nodes.pop().unwrap()
    } else {
        make_branch(nodes)
    }
}

fn replace_node<I: SumTreeItem>(
    node: &Arc<Node<I>>,
    start: usize,
    end: usize,
    inserted: &[I],
) -> Vec<Arc<Node<I>>> {
    match node.as_ref() {
        Node::Leaf { items, .. } => {
            let start = start.min(items.len());
            let end = end.max(start).min(items.len());
            let mut next = Vec::with_capacity(
                items
                    .len()
                    .saturating_sub(end - start)
                    .saturating_add(inserted.len()),
            );
            next.extend_from_slice(&items[..start]);
            next.extend_from_slice(inserted);
            next.extend_from_slice(&items[end..]);
            if next.is_empty() {
                return Vec::new();
            }
            next.chunks(FANOUT).map(make_leaf).collect()
        }
        Node::Branch { children, .. } => {
            let total_leaves = node.leaf_count();
            let start = start.min(total_leaves);
            let end = end.max(start).min(total_leaves);
            let insertion_owner = if start == total_leaves {
                children.len().saturating_sub(1)
            } else {
                let mut base = 0;
                children
                    .iter()
                    .position(|child| {
                        base += child.leaf_count();
                        start < base
                    })
                    .unwrap_or(0)
            };
            let mut out = Vec::with_capacity(children.len());
            let mut base = 0;
            for (index, child) in children.iter().enumerate() {
                let count = child.leaf_count();
                let child_start = base;
                let child_end = base + count;
                let overlaps = start < child_end && end > child_start;
                let owns_insertion = index == insertion_owner;
                if !overlaps && !owns_insertion {
                    out.push(child.clone());
                } else {
                    let local_start = start.saturating_sub(child_start).min(count);
                    let local_end = end.saturating_sub(child_start).min(count);
                    out.extend(replace_node(
                        child,
                        local_start,
                        local_end.max(local_start),
                        if owns_insertion { inserted } else { &[] },
                    ));
                }
                base = child_end;
            }
            pack_level(out)
        }
    }
}

fn locate_node<I: SumTreeItem>(
    node: &Node<I>,
    offset: usize,
    base_leaf: usize,
    measure: fn(&I::Summary) -> usize,
) -> (usize, usize) {
    match node {
        Node::Leaf { items, .. } => {
            let mut consumed: usize = 0;
            for (index, item) in items.iter().enumerate() {
                let size = measure(&item.summary());
                let next = consumed.saturating_add(size);
                if offset < next || index + 1 == items.len() {
                    return (base_leaf + index, offset.saturating_sub(consumed));
                }
                consumed = next;
            }
            (base_leaf, 0)
        }
        Node::Branch { children, .. } => {
            let mut consumed: usize = 0;
            let mut child_index = children.len().saturating_sub(1);
            for (index, child) in children.iter().enumerate() {
                let size = measure(child.total());
                if offset < consumed.saturating_add(size) || index + 1 == children.len() {
                    child_index = index;
                    break;
                }
                consumed = consumed.saturating_add(size);
            }
            let child_base = children[..child_index]
                .iter()
                .map(|child| child.leaf_count())
                .sum::<usize>();
            locate_node(
                &children[child_index],
                offset.saturating_sub(consumed),
                base_leaf + child_base,
                measure,
            )
        }
    }
}

fn summary_before_node<I: SumTreeItem>(node: &Node<I>, leaf: usize) -> I::Summary {
    match node {
        Node::Leaf { items, .. } => items
            .iter()
            .take(leaf)
            .map(SumTreeItem::summary)
            .fold(I::Summary::default(), |acc, item| acc.add(&item)),
        Node::Branch { children, .. } => {
            if leaf == 0 {
                return I::Summary::default();
            }
            let mut remaining = leaf;
            let mut result = I::Summary::default();
            for child in children.iter() {
                let count = child.leaf_count();
                if remaining < count {
                    return result.add(&summary_before_node(child, remaining));
                }
                remaining -= count;
                result = result.add(child.total());
                if remaining == 0 {
                    return result;
                }
            }
            result
        }
    }
}

fn shift_node<I: OffsetItem>(node: &Arc<Node<I>>, delta: isize) -> Arc<Node<I>> {
    if delta == 0 {
        return node.clone();
    }
    let mut shifted = node.as_ref().clone();
    match &mut shifted {
        Node::Leaf { total, lazy, .. } | Node::Branch { total, lazy, .. } => {
            *total = I::shift_summary(total, delta);
            *lazy = lazy.saturating_add(delta);
        }
    }
    Arc::new(shifted)
}

fn push_lazy<I: OffsetItem>(node: &mut Node<I>) {
    let delta = node.lazy();
    if delta == 0 {
        return;
    }
    match node {
        Node::Leaf { items, .. } => {
            for item in Arc::make_mut(items) {
                item.apply_delta(delta);
            }
        }
        Node::Branch { children, .. } => {
            for child in Arc::make_mut(children) {
                *child = shift_node(child, delta);
            }
        }
    }
    match node {
        Node::Leaf { lazy, .. } | Node::Branch { lazy, .. } => *lazy = 0,
    }
}

fn recalc<I: OffsetItem>(node: &mut Node<I>) {
    let total = match node {
        Node::Leaf { items, .. } => items
            .iter()
            .map(SumTreeItem::summary)
            .fold(I::Summary::default(), |acc, item| acc.add(&item)),
        Node::Branch { children, .. } => children
            .iter()
            .map(|child| child.total().clone())
            .fold(I::Summary::default(), |acc, item| acc.add(&item)),
    };
    match node {
        Node::Leaf { total: current, .. } | Node::Branch { total: current, .. } => *current = total,
    }
}

fn edit_node<I: OffsetItem>(
    node: &Arc<Node<I>>,
    old_start: usize,
    old_end: usize,
    byte_delta: isize,
) -> Arc<Node<I>> {
    let Some((min_start, max_end)) = I::range(node.total()) else {
        return node.clone();
    };
    if max_end <= old_start {
        return node.clone();
    }
    let boundary = if old_start == old_end {
        old_start
    } else {
        old_end
    };
    if min_start >= boundary {
        return shift_node(node, byte_delta);
    }

    let mut edited = node.as_ref().clone();
    push_lazy(&mut edited);
    match &mut edited {
        Node::Leaf { items, .. } => {
            let mut next = Vec::with_capacity(items.len());
            for mut item in Arc::make_mut(items).drain(..) {
                if item.apply_edit(old_start, old_end, byte_delta) {
                    next.push(item);
                }
            }
            *items = Arc::new(next);
        }
        Node::Branch { children, .. } => {
            for child in Arc::make_mut(children) {
                *child = edit_node(child, old_start, old_end, byte_delta);
            }
        }
    }
    recalc(&mut edited);
    Arc::new(edited)
}

fn collect_intersecting<I: OffsetItem>(
    node: &Arc<Node<I>>,
    inherited_delta: isize,
    range: (usize, usize),
    result: &mut Vec<I>,
) {
    let summary = I::shift_summary(node.total(), inherited_delta);
    let Some((min_start, max_end)) = I::range(&summary) else {
        return;
    };
    if max_end <= range.0 || min_start >= range.1 {
        return;
    }
    let child_delta = inherited_delta.saturating_add(node.lazy());
    match node.as_ref() {
        Node::Leaf { items, .. } => {
            for item in items.iter() {
                let mut item = item.clone();
                item.apply_delta(child_delta);
                if let Some((start, end)) = I::range(&item.summary()) {
                    if end > range.0 && start < range.1 {
                        result.push(item);
                    }
                }
            }
        }
        Node::Branch { children, .. } => {
            for child in children.iter() {
                collect_intersecting(child, child_delta, range, result);
            }
        }
    }
}

/// 收集叶子下标 `[start, end)` 的 item（DM-400/402）。按叶子序裁剪子树，把节点级
/// lazy delta 物化进 item 后再收集，保证返回绝对坐标。
fn collect_leaf_span<I: OffsetItem>(
    node: &Arc<Node<I>>,
    base_leaf: usize,
    start: usize,
    end: usize,
    inherited: isize,
    out: &mut Vec<I>,
) {
    if base_leaf >= end || base_leaf.saturating_add(node.leaf_count()) <= start {
        return;
    }
    let child_delta = inherited.saturating_add(node.lazy());
    match node.as_ref() {
        Node::Leaf { items, .. } => {
            let from = start.saturating_sub(base_leaf);
            let to = end.saturating_sub(base_leaf).min(items.len());
            for item in items[from..to].iter() {
                let mut item = item.clone();
                item.apply_delta(child_delta);
                out.push(item);
            }
        }
        Node::Branch { children, .. } => {
            let mut base = base_leaf;
            for child in children.iter() {
                collect_leaf_span(child, base, start, end, child_delta, out);
                base = base.saturating_add(child.leaf_count());
            }
        }
    }
}

/// 按叶子序收集每个叶子 item 容器的 `Arc` 指针（DM-202 尾共享检测用）。
#[cfg(test)]
fn leaf_ptrs<I: SumTreeItem>(node: &Node<I>) -> Vec<*const ()> {
    match node {
        Node::Leaf { items, .. } => vec![Arc::as_ptr(items) as *const ()],
        Node::Branch { children, .. } => {
            let mut out = Vec::new();
            for child in children.iter() {
                out.extend(leaf_ptrs(child));
            }
            out
        }
    }
}

/// 按序收集叶子 item（DM-202 FoldSet 迁移/遍历用）。
fn collect_leaves<I: SumTreeItem>(node: &Node<I>, out: &mut Vec<I>) {
    match node {
        Node::Leaf { items, .. } => out.extend(items.iter().cloned()),
        Node::Branch { children, .. } => {
            for child in children.iter() {
                collect_leaves(child, out);
            }
        }
    }
}

/// 读取第 `index` 个叶子 item。
fn get_leaf_item<I: SumTreeItem>(node: &Node<I>, index: usize) -> Option<I> {
    match node {
        Node::Leaf { items, .. } => items.get(index).cloned(),
        Node::Branch { children, .. } => {
            let mut base = 0;
            for child in children.iter() {
                let count = child.leaf_count();
                if index < base + count {
                    return get_leaf_item(child, index - base);
                }
                base += count;
            }
            None
        }
    }
}

fn get_leaf_ref<I: SumTreeItem>(node: &Node<I>, index: usize) -> Option<&I> {
    match node {
        Node::Leaf { items, .. } => items.get(index),
        Node::Branch { children, .. } => {
            let mut base = 0;
            for child in children.iter() {
                let count = child.leaf_count();
                if index < base + count {
                    return get_leaf_ref(child, index - base);
                }
                base += count;
            }
            None
        }
    }
}

/// 第一个 `start >= target` 的叶子下标（叶子按 start 升序）。
fn lower_bound_start_node<I: SumTreeItem>(node: &Node<I>, target: usize) -> usize
where
    I::Summary: IntervalSummary,
{
    match node {
        Node::Leaf { items, .. } => items
            .iter()
            .position(|item| item.summary().start() >= target)
            .unwrap_or(items.len()),
        Node::Branch { children, .. } => {
            let mut base = 0;
            for child in children.iter() {
                // 子树 start 包络：若整体 < target，全在其后，跳过整棵。
                if child.total().end() < target {
                    base += child.leaf_count();
                    continue;
                }
                let within = lower_bound_start_node(child, target);
                if within < child.leaf_count() {
                    return base + within;
                }
                base += child.leaf_count();
            }
            base
        }
    }
}

/// 与 `[qs,qe)` 相交（含相触）的叶子下标范围 `(first, last_exclusive)`。
/// 在 Branch 层据子树 `[start,end)` 包络剪枝：子树整体在查询前（end < qs）或后
/// （start > qe）直接跳过，只下钻相交的路径，保证 O(log n + 相交项)。
fn intersecting_span_node<I: SumTreeItem>(
    node: &Node<I>,
    qs: usize,
    qe: usize,
    base_leaf: usize,
    first: &mut usize,
    last: &mut usize,
) where
    I::Summary: IntervalSummary,
{
    match node {
        Node::Leaf { items, .. } => {
            let mut idx = 0;
            for item in items.iter() {
                let (s, e) = (item.summary().start(), item.summary().end());
                if e >= qs && s <= qe {
                    *first = (*first).min(base_leaf + idx);
                    *last = (*last).max(base_leaf + idx);
                }
                idx += 1;
            }
        }
        Node::Branch { children, .. } => {
            let mut base = base_leaf;
            for child in children.iter() {
                let (cs, ce) = (child.total().start(), child.total().end());
                if ce < qs || cs > qe {
                    base += child.leaf_count();
                    continue;
                }
                intersecting_span_node(child, qs, qe, base, first, last);
                base += child.leaf_count();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(bytes: usize, lines: usize, utf16: usize) -> TextSummary {
        TextSummary {
            bytes,
            lines,
            utf16,
        }
    }

    #[test]
    fn locates_across_branch_boundaries() {
        let input: Vec<_> = (0..100)
            .map(|index| summary(index + 1, index % 2, index + 1))
            .collect();
        let tree = SumTree::<TextSummary>::from_summaries(&input);
        assert_eq!(tree.leaf_count(), 100);
        assert_eq!(
            tree.total(),
            input
                .iter()
                .fold(TextSummary::default(), |acc, item| acc.add(item))
        );
        let mut start = 0;
        for (index, item) in input.iter().enumerate() {
            assert_eq!(tree.byte_start(index), start);
            assert_eq!(tree.locate_byte(start), (index, 0));
            start += item.bytes;
        }
    }

    #[test]
    fn locates_utf16_and_lines_across_branch_boundaries() {
        let input: Vec<_> = (0..100)
            .map(|index| summary(index + 1, (index % 3) + 1, (index % 5) + 1))
            .collect();
        let tree = SumTree::<TextSummary>::from_summaries(&input);
        let mut utf16 = 0;
        let mut lines = 0;
        for (index, item) in input.iter().enumerate() {
            assert_eq!(tree.locate_utf16(utf16), (index, 0));
            assert_eq!(tree.locate_lines(lines), (index, 0));
            utf16 += item.utf16;
            lines += item.lines;
        }
    }

    /// 层 transform 使用的二维摘要：输入长度 + 输出长度。证明通用 SumTree 能
    /// 直接承载 input/output 多维摘要，无需新建第二套平衡树（DM-102）。
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct TransformSummary {
        input: usize,
        output: usize,
    }

    impl Summary for TransformSummary {
        fn add(&self, other: &Self) -> Self {
            Self {
                input: self.input + other.input,
                output: self.output + other.output,
            }
        }
    }

    #[derive(Clone, Debug)]
    struct TransformItem {
        input: usize,
        output: usize,
    }

    impl SumTreeItem for TransformItem {
        type Summary = TransformSummary;

        fn summary(&self) -> Self::Summary {
            TransformSummary {
                input: self.input,
                output: self.output,
            }
        }
    }

    #[test]
    fn generic_tree_supports_input_output_dimensional_summary() {
        // 模拟 FoldMap transform 流：每项把若干输入字节折叠为一段占位符。
        let items: Vec<TransformItem> = (0..100)
            .map(|index| TransformItem {
                input: index + 1,
                output: 1,
            })
            .collect();
        let tree = SumTree::from_items(&items);
        assert_eq!(tree.leaf_count(), 100);

        // 总摘要同时携带 input/output 维度。
        let total = tree.total();
        let expected_input: usize = items.iter().map(|i| i.input).sum();
        assert_eq!(total.input, expected_input);
        assert_eq!(total.output, 100);

        // 局部替换只复制 dirty 路径，且新摘要正确。
        let replacement = [
            TransformItem {
                input: 5,
                output: 9,
            },
            TransformItem {
                input: 6,
                output: 2,
            },
        ];
        let next = tree.replace_leaves(40, 43, &replacement);
        let mut expected: Vec<TransformItem> = items.clone();
        expected.splice(40..43, replacement);
        assert_eq!(next.leaf_count(), expected.len());
        let t = next.total();
        assert_eq!(t.input, expected.iter().map(|i| i.input).sum::<usize>());
        assert_eq!(t.output, expected.iter().map(|i| i.output).sum::<usize>());
    }

    #[test]
    fn empty_tree_is_safe() {
        let tree = SumTree::<TextSummary>::default();
        assert_eq!(tree.total(), TextSummary::default());
        assert_eq!(tree.locate_byte(10), (0, 0));
        assert_eq!(tree.byte_start(10), 0);
    }

    /// DM-202：区间型摘要的相交叶子下标范围定位（含相触 + 无命中 None + lower_bound）。
    #[test]
    fn intersecting_leaf_span_locates() {
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        struct S {
            start: usize,
            end: usize,
        }
        impl Summary for S {
            fn add(&self, o: &Self) -> Self {
                Self {
                    start: self.start.min(o.start),
                    end: self.end.max(o.end),
                }
            }
        }
        impl IntervalSummary for S {
            fn start(&self) -> usize {
                self.start
            }
            fn end(&self) -> usize {
                self.end
            }
        }
        #[derive(Clone, Debug)]
        struct I {
            s: usize,
            e: usize,
        }
        impl SumTreeItem for I {
            type Summary = S;
            fn summary(&self) -> Self::Summary {
                S {
                    start: self.s,
                    end: self.e,
                }
            }
        }
        let tree =
            SumTree::from_items(&[I { s: 6, e: 12 }, I { s: 18, e: 24 }, I { s: 30, e: 36 }]);
        // 查询 [12,18)：与 [6,12)（e=12>=12）及 [18,24)（s=18<=18）相触 → 命中叶子 0..2。
        assert_eq!(tree.intersecting_leaf_span((12, 18)), Some((0, 2)));
        // 折叠区间为闭区间 [start,end]：查询 [36,42) 与叶子 [30,36]（e=36>=36）在 36 相触
        // → 命中（与 fold 插入相邻区间应合并的语义一致）。
        assert_eq!(tree.intersecting_leaf_span((36, 42)), Some((2, 3)));
        // 真正落在折叠间隙（[36,42) 触到 [30,36]；[37,42) 完全在 36 之后）→ None，
        // 此时用 lower_bound_start 定插入点。
        assert_eq!(tree.intersecting_leaf_span((37, 42)), None);
        // lower_bound：首个 start>=target 的叶子下标。
        assert_eq!(tree.lower_bound_start(18), 1);
        assert_eq!(tree.lower_bound_start(6), 0);
        assert_eq!(tree.lower_bound_start(100), 3);
    }

    #[test]
    fn persistent_replace_shares_untouched_suffix() {
        let input: Vec<_> = (0..100)
            .map(|index| summary(index + 1, index % 3, index + 2))
            .collect();
        let tree = SumTree::<TextSummary>::from_summaries(&input);
        let old_suffix = match tree.root.as_ref() {
            Node::Branch { children, .. } => children[3].clone(),
            Node::Leaf { .. } => panic!("test input must create a branch"),
        };
        let replacement = [summary(7, 1, 8), summary(11, 0, 12)];
        let next = tree.replace_leaves(40, 43, &replacement);
        let new_suffix = match next.root.as_ref() {
            Node::Branch { children, .. } => children[3].clone(),
            Node::Leaf { .. } => panic!("replacement should keep a branch"),
        };
        assert!(Arc::ptr_eq(&old_suffix, &new_suffix));
    }

    #[test]
    fn persistent_replace_matches_flat_sequence() {
        let mut expected: Vec<_> = (0..200)
            .map(|index| summary((index % 9) + 1, index % 2, (index % 7) + 1))
            .collect();
        let mut tree = SumTree::<TextSummary>::from_summaries(&expected);
        for step in 0..80 {
            let start = (step * 17) % (expected.len() + 1);
            let delete = (step * 5) % 4;
            let end = (start + delete).min(expected.len());
            let inserted = [summary(step + 1, step % 3, step + 2)];
            expected.splice(start..end, inserted);
            tree = tree.replace_leaves(start, end, &inserted);
            assert_eq!(tree.leaf_count(), expected.len());
            assert_eq!(
                tree.total(),
                expected
                    .iter()
                    .fold(TextSummary::default(), |acc, item| acc.add(item))
            );
        }
    }
}
