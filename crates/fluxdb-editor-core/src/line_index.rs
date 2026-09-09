//! Persistent block treap for line starts.
//!
//! Each leaf stores a small block of line offsets. Range shifts are represented by
//! lazy deltas on treap subtrees, so inserting one character does not walk every
//! following line. Snapshots share the immutable root.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use crate::model::{Offset, Range};

const BLOCK_LINES: usize = 256;

type Link = Option<Arc<Node>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    priority: u64,
    starts: Arc<Vec<Offset>>,
    block_delta: isize,
    lazy: isize,
    left: Link,
    right: Link,
    size: usize,
    first: Offset,
    last: Offset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineIndex {
    root: Link,
}

fn priorities() -> &'static AtomicU64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new(0x9e37_79b9_7f4a_7c15))
}

fn next_priority() -> u64 {
    let mut x = priorities().fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    // splitmix64: deterministic, dependency-free priorities with good treap balance.
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn shift(value: Offset, delta: isize) -> Offset {
    if delta >= 0 {
        value.saturating_add(delta as usize)
    } else {
        value.saturating_sub((-delta) as usize)
    }
}

fn add_delta(a: isize, b: isize) -> isize {
    a.saturating_add(b)
}

fn node_size(node: &Link) -> usize {
    node.as_ref().map(|node| node.size).unwrap_or(0)
}

fn block_last(node: &Node) -> Offset {
    node.starts
        .last()
        .copied()
        .map(|offset| shift(offset, node.block_delta))
        .unwrap_or(0)
}

fn recalc(node: &mut Node) {
    node.size = node_size(&node.left) + node.starts.len() + node_size(&node.right);
    node.first = node
        .left
        .as_ref()
        .map(|left| left.first)
        .unwrap_or_else(|| shift(node.starts[0], node.block_delta));
    node.last = node
        .right
        .as_ref()
        .map(|right| right.last)
        .unwrap_or_else(|| block_last(node));
}

fn apply_delta(link: &mut Link, delta: isize) {
    if delta == 0 {
        return;
    }
    if let Some(node) = link {
        let node = Arc::make_mut(node);
        node.lazy = add_delta(node.lazy, delta);
        node.first = shift(node.first, delta);
        node.last = shift(node.last, delta);
    }
}

fn push(node: &mut Node) {
    let delta = node.lazy;
    if delta == 0 {
        return;
    }
    node.block_delta = add_delta(node.block_delta, delta);
    apply_delta(&mut node.left, delta);
    apply_delta(&mut node.right, delta);
    node.lazy = 0;
}

fn make_node(starts: Vec<Offset>, left: Link, right: Link) -> Arc<Node> {
    debug_assert!(!starts.is_empty());
    let first = starts.first().copied().unwrap_or(0);
    let mut node = Node {
        priority: next_priority(),
        last: starts.last().copied().unwrap_or(0),
        starts: Arc::new(starts),
        block_delta: 0,
        lazy: 0,
        size: 0,
        first,
        left,
        right,
    };
    recalc(&mut node);
    Arc::new(node)
}

fn merge(left: Link, right: Link) -> Link {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) => {
            if left.priority >= right.priority {
                let mut node = (*left).clone();
                push(&mut node);
                node.right = merge(node.right.take(), Some(right));
                recalc(&mut node);
                Some(Arc::new(node))
            } else {
                let mut node = (*right).clone();
                push(&mut node);
                node.left = merge(Some(left), node.left.take());
                recalc(&mut node);
                Some(Arc::new(node))
            }
        }
    }
}

fn split(root: Link, count: usize) -> (Link, Link) {
    let Some(root) = root else {
        return (None, None);
    };
    let mut node = (*root).clone();
    push(&mut node);
    let left_size = node_size(&node.left);
    let block_size = node.starts.len();
    if count < left_size {
        let (left, middle) = split(node.left.take(), count);
        node.left = middle;
        recalc(&mut node);
        (left, Some(Arc::new(node)))
    } else if count > left_size + block_size {
        let (middle, right) = split(node.right.take(), count - left_size - block_size);
        node.right = middle;
        recalc(&mut node);
        (Some(Arc::new(node)), right)
    } else if count == left_size {
        let left = node.left.take();
        recalc(&mut node);
        (left, Some(Arc::new(node)))
    } else if count == left_size + block_size {
        let right = node.right.take();
        recalc(&mut node);
        (Some(Arc::new(node)), right)
    } else {
        let split_at = count - left_size;
        let left_block = make_node(node.starts[..split_at].to_vec(), node.left.take(), None);
        let right_block = make_node(node.starts[split_at..].to_vec(), None, node.right.take());
        (Some(left_block), Some(right_block))
    }
}

impl LineIndex {
    pub(crate) fn from_offsets(offsets: &[Offset]) -> Self {
        let mut root = None;
        for block in offsets.chunks(BLOCK_LINES) {
            root = merge(root, Some(make_node(block.to_vec(), None, None)));
        }
        Self { root }
    }

    pub(crate) fn len(&self) -> usize {
        node_size(&self.root)
    }

    pub(crate) fn offset_at(&self, row: usize) -> Offset {
        fn get(node: &Node, row: usize, ancestor_delta: isize) -> Offset {
            let delta = add_delta(ancestor_delta, node.lazy);
            let left_size = node_size(&node.left);
            if row < left_size {
                return get(node.left.as_ref().expect("left row exists"), row, delta);
            }
            let block_end = left_size + node.starts.len();
            if row < block_end {
                return shift(
                    node.starts[row - left_size],
                    add_delta(delta, node.block_delta),
                );
            }
            get(
                node.right.as_ref().expect("right row exists"),
                row - block_end,
                delta,
            )
        }
        let row = row.min(self.len().saturating_sub(1));
        self.root
            .as_ref()
            .map(|root| get(root, row, 0))
            .unwrap_or(0)
    }

    pub(crate) fn row_for_offset(&self, offset: Offset) -> usize {
        self.count_leq(offset).saturating_sub(1)
    }

    fn count_leq(&self, offset: Offset) -> usize {
        fn count(node: &Node, target: Offset, ancestor_delta: isize) -> usize {
            let delta = add_delta(ancestor_delta, node.lazy);
            if target < shift(node.first, ancestor_delta) {
                return 0;
            }
            if target >= shift(node.last, ancestor_delta) {
                return node.size;
            }
            let left_count = node
                .left
                .as_ref()
                .map(|left| count(left, target, delta))
                .unwrap_or(0);
            let block_delta = add_delta(delta, node.block_delta);
            let mut lo = 0;
            let mut hi = node.starts.len();
            while lo < hi {
                let mid = (lo + hi) / 2;
                if shift(node.starts[mid], block_delta) <= target {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            left_count
                + lo
                + if lo == node.starts.len() {
                    node.right
                        .as_ref()
                        .map(|right| count(right, target, delta))
                        .unwrap_or(0)
                } else {
                    0
                }
        }
        self.root
            .as_ref()
            .map(|root| count(root, offset, 0))
            .unwrap_or(0)
    }

    pub(crate) fn splice(&mut self, old_range: Range, new_text: &str) {
        let old_len = old_range.len();
        let affected_line = self.row_for_offset(old_range.start);
        let suffix_index = self.count_leq(if old_len == 0 {
            old_range.start
        } else {
            old_range.end
        });
        let inserted: Vec<Offset> = new_text
            .bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(old_range.start + index + 1))
            .collect();
        let prefix_count = affected_line + 1;
        let (prefix, rest) = split(self.root.take(), prefix_count);
        let (_removed, suffix) = split(rest, suffix_index.saturating_sub(prefix_count));
        let delta = new_text.len() as isize - old_len as isize;
        let mut suffix = suffix;
        apply_delta(&mut suffix, delta);
        let inserted = inserted.into_iter().collect::<Vec<_>>();
        let middle = if inserted.is_empty() {
            None
        } else {
            Some(make_node(inserted, None, None))
        };
        self.root = merge(merge(prefix, middle), suffix);
    }

    pub(crate) fn collect(&self) -> Vec<Offset> {
        (0..self.len()).map(|row| self.offset_at(row)).collect()
    }
}
