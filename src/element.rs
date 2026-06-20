//! 排版元素定义

use serde::{Deserialize, Serialize};

/// 元素的尺寸策略 —— 决定元素如何响应空间变化
///
/// 对应 Figma / InDesign 的 Fixed / Fill 语义，
/// 与 CSS 的 `flex-shrink` / `flex-grow` 正交。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeStrategy {
    /// 固定尺寸：保持首选宽度不变
    ///
    /// `shrinkable` 控制空间不足时是否允许缩小到 [min_width, preferred] 之间。
    Fixed {
        /// 如果空间不够，是否允许缩小（不能低于 min_width）
        #[serde(default)]
        shrinkable: bool,
    },
    /// 弹性填充：可拉伸来填补剩余空间，亦可缩小当空间不足
    ///
    /// 等同于 CSS `flex: 1`（可伸可缩）。
    Fill,
}

impl Default for SizeStrategy {
    fn default() -> Self {
        SizeStrategy::Fixed { shrinkable: false }
    }
}

impl SizeStrategy {
    /// 当前策略是否允许缩小
    pub fn can_shrink(&self) -> bool {
        match self {
            SizeStrategy::Fixed { shrinkable } => *shrinkable,
            SizeStrategy::Fill => true,
        }
    }

    /// 当前策略是否允许拉伸（行内填满空间）
    pub fn can_stretch(&self) -> bool {
        matches!(self, SizeStrategy::Fill)
    }
}

/// 一个待排版元素
///
/// `width` / `height` 是元素的**首选尺寸**（偏好值），
/// 实际排出的尺寸受 `constraints` 和布局上下文影响。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutElement {
    /// 元素唯一标识（对 AI/用户友好）
    pub id: String,
    /// 首选宽度
    pub width: f64,
    /// 首选高度
    pub height: f64,
    /// 宽度约束
    #[serde(default)]
    pub constraints: ElementConstraints,
    /// 每元素独立外边距（margin）
    #[serde(default)]
    pub margin: ElementMargin,
    /// 基线位置：从元素顶部到基线的距离
    ///
    /// `None` 时回退为 `height`（底部对齐），
    /// 这是 CSS / InDesign 的惯例：非文本元素底部对齐文本基线。
    #[serde(default)]
    pub baseline: Option<f64>,
}

/// 元素级别的外边距（margin）
///
/// 所有字段默认为 0.0。margin 参与行内占地面积计算，
/// 但 `PlacedElement.width` / `height` 只包含内容尺寸（不含 margin）。
///
/// 相邻元素的 margin 之间以及 margin 与 gap 不会折叠——它们会叠加。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct ElementMargin {
    /// 左边距
    #[serde(default)]
    pub left: f64,
    /// 右边距
    #[serde(default)]
    pub right: f64,
    /// 上边距
    #[serde(default)]
    pub top: f64,
    /// 下边距
    #[serde(default)]
    pub bottom: f64,
}

impl ElementMargin {
    /// 创建四周均等的 margin
    pub fn uniform(v: f64) -> Self {
        Self {
            left: v,
            right: v,
            top: v,
            bottom: v,
        }
    }

    /// 创建仅水平 margin（左右均等）
    pub fn horizontal(v: f64) -> Self {
        Self {
            left: v,
            right: v,
            top: 0.0,
            bottom: 0.0,
        }
    }
}

/// 元素级别的宽度约束
///
/// 所有字段默认为无约束（`None` / `Fixed`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ElementConstraints {
    /// 最小宽度（低于此值不可缩）
    pub min_width: Option<f64>,
    /// 最大宽度（超过此值不可扩）
    pub max_width: Option<f64>,
    /// 尺寸策略（替代旧的 `shrinkable` / `stretchable` 双 bool）
    #[serde(default)]
    pub size_strategy: SizeStrategy,
}

impl LayoutElement {
    /// 创建一个最简单的固定尺寸元素
    pub fn new(id: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            id: id.into(),
            width,
            height,
            constraints: ElementConstraints::default(),
            margin: ElementMargin::default(),
            baseline: None,
        }
    }

    /// 创建一个带 margin 的固定尺寸元素
    pub fn with_margin(
        id: impl Into<String>,
        width: f64,
        height: f64,
        margin: ElementMargin,
    ) -> Self {
        Self {
            id: id.into(),
            width,
            height,
            constraints: ElementConstraints::default(),
            margin,
            baseline: None,
        }
    }

    /// 有效基线：`baseline` 指定值时用它，否则回退到 `height`（底部）
    pub fn effective_baseline(&self) -> f64 {
        self.baseline.unwrap_or(self.height)
    }

    /// 有效宽度：夹在 [min_width, max_width] 之间的首选宽度（不含 margin）
    pub fn effective_width(&self) -> f64 {
        let mut w = self.width;
        if let Some(min) = self.constraints.min_width {
            w = w.max(min);
        }
        if let Some(max) = self.constraints.max_width {
            w = w.min(max);
        }
        w
    }

    /// 行内占地面积：内容宽度 + 水平 margin
    pub fn footprint_width(&self) -> f64 {
        self.effective_width() + self.margin.left + self.margin.right
    }

    /// 行内占地面积（用给定宽度替代 effective_width）
    pub fn footprint_width_with(&self, w: f64) -> f64 {
        w + self.margin.left + self.margin.right
    }
    /// 垂直占地面积：内容高度 + 垂直 margin
    pub fn footprint_height(&self) -> f64 {
        self.height + self.margin.top + self.margin.bottom
    }
}
