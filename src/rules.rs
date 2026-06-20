//! 排版规则：对齐、间距、全局配置

use serde::{Deserialize, Serialize};

/// 行内水平对齐
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HAlign {
    /// 左对齐：元素从左侧安全区间边界开始排列
    #[default]
    Left,
    /// 居中：元素组在行内居中
    Center,
    /// 右对齐：元素组靠右排列
    Right,
}

/// 垂直对齐
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VAlign {
    /// 顶部对齐
    #[default]
    Top,
    /// 垂直居中
    Middle,
    /// 底部对齐
    Bottom,
    /// 基线对齐：所有元素的基线对齐到同一 Y 坐标，
    /// 行高由"最高基线 + 最大下伸"决定而非 max(height)
    Baseline,
}

/// 全局排版配置——一次 `layout_rows()` 调用使用一套配置
///
/// 所有尺寸单位为世界坐标（与容器坐标系一致）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// 容器上边距
    pub padding_top: f64,
    /// 容器下边距
    pub padding_bottom: f64,
    /// 容器左边距
    pub padding_left: f64,
    /// 容器右边距
    pub padding_right: f64,
    /// 行内元素水平间距
    pub gap: f64,
    /// 行间垂直间距
    pub line_spacing: f64,
    /// 最小有效区间宽度（小于此宽度的区间被过滤）
    pub min_width: Option<f64>,
    /// Y 轴扫描步长（用于跳过不可用区域，默认 0.5）
    #[serde(default = "default_step_size")]
    pub step_size: f64,
    /// 行内水平对齐
    #[serde(default)]
    pub halign: HAlign,
    /// 垂直对齐（Phase 1 预留）
    #[serde(default)]
    pub valign: VAlign,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            gap: 0.0,
            line_spacing: 0.0,
            min_width: None,
            step_size: 0.5,
            halign: HAlign::Left,
            valign: VAlign::Top,
        }
    }
}

fn default_step_size() -> f64 {
    0.5
}

impl LayoutConfig {
    /// 创建一个紧凑的默认配置（零边距、无间距）
    pub fn compact() -> Self {
        Self::default()
    }

    /// 快捷构造：指定 padding / gap / line_spacing，其余默认
    pub fn with_spacing(padding: f64, gap: f64, line_spacing: f64) -> Self {
        Self {
            padding_top: padding,
            padding_bottom: padding,
            padding_left: padding,
            padding_right: padding,
            gap,
            line_spacing,
            ..Default::default()
        }
    }

    /// 快捷构造：指定 padding / gap / line_spacing + 对齐方式
    pub fn with_alignment(
        padding: f64,
        gap: f64,
        line_spacing: f64,
        halign: HAlign,
    ) -> Self {
        Self {
            padding_top: padding,
            padding_bottom: padding,
            padding_left: padding,
            padding_right: padding,
            gap,
            line_spacing,
            halign,
            ..Default::default()
        }
    }
}
