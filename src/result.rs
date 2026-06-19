//! 排版结果

use serde::{Deserialize, Serialize};

/// 已排好的单个元素
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacedElement {
    /// 元素 ID（对应 `LayoutElement::id`）
    pub id: String,
    /// 放置后的左上角 X 坐标
    pub x: f64,
    /// 放置后的左上角 Y 坐标
    pub y: f64,
    /// 实际宽度（可能因约束被压缩或拉伸）
    pub width: f64,
    /// 实际高度（Phase 1 中恒等于输入高度）
    pub height: f64,
}

/// 单次排版求解的结果
///
/// - `placed`：成功排入容器的元素
/// - `unplaced`：无法排入的元素的 ID（容器空间不足）
/// - `warnings`：求解过程中产生的非致命警告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutSolution {
    /// 已排好的元素列表
    pub placed: Vec<PlacedElement>,
    /// 无法排入的元素 ID
    pub unplaced: Vec<String>,
    /// 警告信息（对 AI 可读）
    pub warnings: Vec<LayoutWarning>,
}

/// 排版警告（对 AI / 用户可自愈）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayoutWarning {
    /// 约束冲突：某个 kasuari 约束无法满足
    ConstraintConflict(String),
    /// 空间溢出：容器底部空间不足
    Overflow {
        element_id: String,
        message: String,
    },
    /// 元素过大：即使最小宽度也放不进任何行
    ElementTooWide {
        element_id: String,
        min_width: f64,
        max_available: f64,
    },
    /// 容器形状无效
    InvalidContainer,
    /// 某元素因无法满足的宽度约束被跳过
    WidthConstraintUnsatisfiable {
        element_id: String,
        message: String,
    },
}

impl LayoutSolution {
    /// 快速构造一个容器无效的结果
    pub fn invalid_container(elements: &[super::element::LayoutElement]) -> Self {
        Self {
            placed: vec![],
            unplaced: elements.iter().map(|e| e.id.clone()).collect(),
            warnings: vec![LayoutWarning::InvalidContainer],
        }
    }

    /// 所有元素都已成功排放
    pub fn is_fully_placed(&self) -> bool {
        self.unplaced.is_empty()
    }

    /// 已排放的元素数量
    pub fn placed_count(&self) -> usize {
        self.placed.len()
    }
}
