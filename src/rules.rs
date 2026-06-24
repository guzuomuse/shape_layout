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

/// 排版方向：控制元素在容器内的堆叠方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StackDirection {
    /// 默认流式：从上到下贪心分行，同行内尽可能多地放置元素
    #[default]
    Flow,
    /// 强制竖排：每个元素独占一行，同行内不放第二个元素
    ///
    /// 用于名片、标签竖排、海报等需要每个元素单独成行的场景。
    /// Fill 元素在独占行内仍然会拉伸到该行可用宽度。
    Vertical,
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
    /// 垂直对齐
    #[serde(default)]
    pub valign: VAlign,
    /// 排版方向
    #[serde(default)]
    pub stack_direction: StackDirection,
    /// Fill 元素的隐式最小宽度比例
    ///
    /// 当 Fill 元素未显式设置 `constraints.min_width` 时，
    /// 以此比例 × `preferred_width` 作为隐式下限，
    /// 防止 Fill 在狭窄区域被压缩至无法辨认的尺寸。
    /// 默认 0.4（即至少保留首选宽度的 40%），
    /// 当计算值 < 1.0 时回退到 1.0（`FILL_MIN_CONTENT_WIDTH`）。
    #[serde(default = "default_fill_min_ratio")]
    pub fill_min_ratio: f64,
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
            stack_direction: StackDirection::Flow,
            fill_min_ratio: default_fill_min_ratio(),
        }
    }
}

fn default_step_size() -> f64 {
    0.5
}

fn default_fill_min_ratio() -> f64 {
    0.4
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
