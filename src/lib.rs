//! # shape_layout — 异形容器内的扫描线排版引擎
//!
//! ## 三层架构
//!
//! ```text
//! Layer 1 (眼)  RangeGenerator    — 扫描线切割异形容器 → 1D 区间 [L, R]
//! Layer 2 (管)  RowScheduler      — 逐行推进 Y，贪心分行
//! Layer 3 (脑)  kasuari Solver    — 行内 X 轴线性约束求解
//! ```
//!
//! ## 快速开始
//!
//! ```rust,ignore
//! use shape_layout::{layout_rows, LayoutElement, LayoutConfig};
//! use kurbo::BezPath;
//!
//! let container = BezPath::from_vec(/* ... */);
//! let elements = vec![
//!     LayoutElement::new("logo", 80.0, 40.0),
//!     LayoutElement::new("title", 200.0, 30.0),
//! ];
//! let config = LayoutConfig::with_spacing(10.0, 8.0, 12.0);
//!
//! let solution = layout_rows(&container, &elements, &config);
//! for placed in &solution.placed {
//!     println!("{}: x={}, y={}, w={}, h={}",
//!         placed.id, placed.x, placed.y, placed.width, placed.height);
//! }
//! ```
//!
//! ## AI 友好
//!
//! - 全声明式 API
//! - 所有数据结构 `Serialize + Deserialize`，支持 RON/JSON
//! - 错误信息对 AI 可自愈（`LayoutWarning::ElementTooWide` 等）
//!
//! ## 不依赖 Bevy
//!
//! 仅依赖 `kurbo` + `i_overlay` + `kasuari`，纯计算库。

// ── 模块声明 ──
pub mod element;
pub mod engine;
pub mod region;
pub mod result;
pub mod rules;
pub mod shape;

// ── 重导出（方便外部使用） ──
pub use element::ElementConstraints;
pub use element::ElementMargin;
pub use element::LayoutElement;
pub use element::SizeStrategy;
pub use engine::layout_container;
pub use engine::layout_rows;
pub use region::RangeGenerator;
pub use region::RowRange;
pub use result::LayoutSolution;
pub use result::LayoutWarning;
pub use result::PlacedElement;
pub use rules::HAlign;
pub use rules::LayoutConfig;
pub use rules::VAlign;
pub use shape::ContainerShape;
