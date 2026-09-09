//! Display 分层坐标空间。
//!
//! 显示映射把 buffer 文本逐层变换为 UI 逻辑坐标，禁止继续用裸 `usize`
//! 混用 byte offset、UTF-16 column、display column 和 visual row。本模块
//! 用 newtype 表达各层坐标，保证编译器区分不同坐标空间：
//!
//! ```text
//! BufferOffset(UTF-8 byte)
//!   -> BufferPoint (row + byte column)
//!   -> InlayPoint  -> FoldPoint -> TabPoint -> WrapPoint -> BlockPoint
//!   -> DisplayPoint (UI 最终逻辑坐标)
//! ```
//!
//! 每种 newtype 含义见 [`crate::display_map`] 设计；坐标转换必须经对应层
//! Snapshot 并显式传入 [`Bias`]，处理 fold placeholder、inlay 与零宽边界。

use crate::model::{Bias, Point};

/// UTF-8 byte offset，随编辑变化；稳定重定位依赖 [`crate::model::Anchor`]。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferOffset(pub usize);

impl BufferOffset {
    pub const fn new(offset: usize) -> Self {
        Self(offset)
    }

    pub fn to_usize(self) -> usize {
        self.0
    }
}

impl From<usize> for BufferOffset {
    fn from(offset: usize) -> Self {
        Self(offset)
    }
}

impl From<BufferOffset> for usize {
    fn from(offset: BufferOffset) -> Self {
        offset.0
    }
}

/// Buffer 坐标：`row` + 字节列。不含 inlay/fold/wrap。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferPoint {
    pub row: usize,
    pub column: usize,
}

impl BufferPoint {
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }

    pub fn is_zero(&self) -> bool {
        self.row == 0 && self.column == 0
    }
}

impl From<Point> for BufferPoint {
    fn from(point: Point) -> Self {
        Self {
            row: point.row,
            column: point.column,
        }
    }
}

impl From<BufferPoint> for Point {
    fn from(point: BufferPoint) -> Self {
        Point::new(point.row, point.column)
    }
}

/// 插入 inline element（inlay）后、折叠前的坐标空间。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InlayPoint {
    pub row: usize,
    /// 显示列；inlay 文本已计入，但不改变 buffer byte 语义。
    pub column: usize,
}

impl InlayPoint {
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }
}

/// 折叠替换后的坐标空间，包含 placeholder。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FoldPoint {
    pub row: usize,
    pub column: usize,
}

/// Tab 展开后的显示坐标。`\t` 不改变 buffer byte offset，只改变显示列。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabPoint {
    pub row: usize,
    pub column: usize,
}

/// soft-wrap 后的坐标；一个 buffer 行可映射为多个 display row。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WrapPoint {
    pub row: usize,
    pub column: usize,
}

/// block 插入后的显示坐标；参与滚动 y。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockPoint {
    pub row: usize,
    pub column: usize,
}

impl BlockPoint {
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }
}

/// UI 最终逻辑坐标；selection/caret/IME/layout/hit test 统一使用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayPoint {
    pub row: usize,
    pub column: usize,
}

/// 按 [`Bias`] 在共享点两侧之间取其一。[`Bias`] 语义（DM-101）：
///
/// - **零宽边界**：共享点两侧无内容（`left == right`），任一偏置结果相同；
/// - **fold placeholder**：`left` 指向占位符之前，`right` 指向占位符之后，
///   光标在折叠边界移动时应落到预期一侧；
/// - **inlay 边界**：inlay 不改变 buffer offset，只改变 display column，
///   `left` 落在 inlay 起始，`right` 落在 inlay 结束之后。
///
/// 该类型由各层 Snapshot 在输出时构造，避免把「两个候选点 + 一个枚举」
/// 散落为裸元组或裸 usize 分支。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Biased<T> {
    pub left: T,
    pub right: T,
}

impl<T: Copy + PartialEq> Biased<T> {
    pub fn get(self, bias: Bias) -> T {
        match bias {
            Bias::Left => self.left,
            Bias::Right => self.right,
        }
    }

    /// 两个候选相等（零宽边界）。
    pub fn is_identical(self) -> bool {
        self.left == self.right
    }
}

impl<T: Copy> From<T> for Biased<T> {
    fn from(value: T) -> Self {
        Self {
            left: value,
            right: value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 各坐标是独立类型：同数值的 BufferOffset 与 BufferPoint 不能互赋。
    #[test]
    fn coordinate_types_are_distinct() {
        let offset = BufferOffset::new(7);
        assert_eq!(offset.to_usize(), 7);
        let point = BufferPoint::new(1, 2);
        assert_eq!(point.row, 1);
        assert_eq!(point.column, 2);
        let _ = InlayPoint { row: 0, column: 0 };
        let _ = FoldPoint { row: 0, column: 0 };
        let _ = TabPoint { row: 0, column: 0 };
        let _ = WrapPoint { row: 0, column: 0 };
        let _ = BlockPoint { row: 0, column: 0 };
        let _ = DisplayPoint { row: 0, column: 0 };
    }

    #[test]
    fn buffer_point_converts_to_model_point_and_back() {
        let model = Point::new(3, 5);
        let buffer: BufferPoint = model.into();
        assert_eq!(buffer, BufferPoint::new(3, 5));
        assert_eq!(Point::from(buffer), model);
    }

    #[test]
    fn biased_selects_by_bias() {
        let b: Biased<usize> = Biased { left: 1, right: 2 };
        assert_eq!(b.get(Bias::Left), 1);
        assert_eq!(b.get(Bias::Right), 2);
        // 无障碍点两侧相同。
        let same: Biased<usize> = 9.into();
        assert_eq!(same.get(Bias::Left), 9);
        assert_eq!(same.get(Bias::Right), 9);
    }

    /// DM-101：零宽边界两侧候选相同，偏置无效。
    #[test]
    fn zero_width_boundary_is_identical_and_bias_independent() {
        let b: Biased<BufferPoint> = Biased {
            left: BufferPoint::new(1, 5),
            right: BufferPoint::new(1, 5),
        };
        assert!(b.is_identical());
        assert_eq!(b.get(Bias::Left), b.get(Bias::Right));
    }

    /// DM-101：fold placeholder 边界——left 落在占位符前，right 落在占位符后。
    #[test]
    fn fold_placeholder_boundary_bias() {
        // 折叠占位符把 byte6..=16 折叠，占位符起始列 6，占位后该行显示列并入右界。
        let b: Biased<BufferPoint> = Biased {
            left: BufferPoint::new(0, 6),
            right: BufferPoint::new(0, 8),
        };
        assert!(!b.is_identical());
        assert_eq!(b.get(Bias::Left), BufferPoint::new(0, 6));
        assert_eq!(b.get(Bias::Right), BufferPoint::new(0, 8));
    }

    /// DM-101：inlay 边界——bias 决定落在 inline hint 之前还是之后；inlay
    /// 不改变 buffer byte offset，只改变 display column。
    #[test]
    fn inlay_boundary_bias_affects_display_column_only() {
        // 同一 buffer offset，inlay 左侧列 4、右侧列 7（hint 宽 3 列）。
        let b: Biased<TabPoint> = Biased {
            left: TabPoint { row: 0, column: 4 },
            right: TabPoint { row: 0, column: 7 },
        };
        assert_eq!(b.get(Bias::Left).column, 4);
        assert_eq!(b.get(Bias::Right).column, 7);
        // 两候选共享同一 buffer row（坐标空间不因 inlay 跨行）。
        assert!(b.get(Bias::Left).row == b.get(Bias::Right).row);
    }
}
