//! 排版元素定义

use serde::{Deserialize, Serialize};

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
    /// 首选高度（Phase 1 中高度固定不可变）
    pub height: f64,
    /// 宽度约束
    #[serde(default)]
    pub constraints: ElementConstraints,
}

/// 元素级别的宽度约束
///
/// 所有字段默认为无约束（`None` / `false`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ElementConstraints {
    /// 最小宽度（低于此值不可缩）
    pub min_width: Option<f64>,
    /// 最大宽度（超过此值不可扩）
    pub max_width: Option<f64>,
    /// 是否允许缩小来适应行宽
    #[serde(default)]
    pub shrinkable: bool,
    /// 是否允许拉伸来填充行宽
    #[serde(default)]
    pub stretchable: bool,
}

impl LayoutElement {
    /// 创建一个最简单的固定尺寸元素
    pub fn new(id: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            id: id.into(),
            width,
            height,
            constraints: ElementConstraints::default(),
        }
    }

    /// 有效宽度：夹在 [min_width, max_width] 之间的首选宽度
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
}
