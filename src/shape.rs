//! 容器形状 —— 可序列化的逻辑实体
//!
//! `ContainerShape` 枚举定义容器形状，AI 优先使用内置变体（极少 Token），
//! `Custom` 变体用于任意路径的兜底。
//!
//! 所有变体均实现 `Serialize + Deserialize`，可直接通过 RON 格式传输。
//!
//! # 示例
//!
//! ```ron
//! // 矩形
//! Rect(width: 200.0, height: 100.0)
//!
//! // 圆角矩形
//! RoundedRect(width: 150.0, height: 80.0, radius: 10.0)
//!
//! // 圆形
//! Circle(diameter: 100.0)
//!
//! // 心形
//! Heart(width: 120.0)
//! ```

use kurbo::{BezPath, Circle, Rect, Shape};
use serde::{Deserialize, Serialize};

/// 容器形状定义
///
/// 调用 `to_bezpath()` 将逻辑实体转换为物理贝塞尔路径，
/// 供 `layout_rows()` / `layout_container()` 使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContainerShape {
    /// 矩形（无需手动构造 BezPath）
    Rect {
        /// 宽度
        width: f64,
        /// 高度
        height: f64,
    },
    /// 圆角矩形
    RoundedRect {
        /// 宽度
        width: f64,
        /// 高度
        height: f64,
        /// 圆角半径
        radius: f64,
    },
    /// 正圆形
    Circle {
        /// 直径
        diameter: f64,
    },
    /// 心形（AI 经常用到的异形容器）
    Heart {
        /// 心形宽度（高度自动按比例 0.9 计算）
        width: f64,
    },
    /// 任意自定义路径（兜底方案）
    Custom {
        /// 已支持 serde 的 BezPath（来自 workspace kurbo with serde feature）
        path: BezPath,
    },
}

impl ContainerShape {
    /// 逻辑实体 → 物理贝塞尔路径
    ///
    /// 使用 kurbo 内置形状的 `to_path()` 方法生成高质量贝塞尔近似，
    /// 而非手动拼接路径段。
    pub fn to_bezpath(&self) -> BezPath {
        match self {
            Self::Rect { width, height } => {
                let rect = Rect::new(0.0, 0.0, *width, *height);
                rect.to_path(0.1)
            }
            Self::RoundedRect {
                width,
                height,
                radius,
            } => {
                let rect = Rect::new(0.0, 0.0, *width, *height);
                let r_rect = kurbo::RoundedRect::from_rect(rect, *radius);
                r_rect.to_path(0.1)
            }
            Self::Circle { diameter } => {
                let r = diameter / 2.0;
                let circle = Circle::new((r, r), r);
                circle.to_path(0.1)
            }
            Self::Heart { width } => {
                // 心形参数化公式（贝塞尔曲线近似）
                // 所有坐标 ≥ 0，底部中心为原点
                let w = *width;
                let h = w * 0.9;
                let mut p = BezPath::new();
                // 底部中心
                p.move_to((w / 2.0, 0.0));
                // 右侧向上到右鼓起
                p.curve_to(
                    (w * 0.85, h * 0.15),
                    (w, h * 0.3),
                    (w, h * 0.45),
                );
                // 右鼓起到顶部中心
                p.curve_to(
                    (w, h * 0.65),
                    (w * 0.65, h),
                    (w / 2.0, h),
                );
                // 顶部中心到左鼓起
                p.curve_to(
                    (w * 0.35, h),
                    (0.0, h * 0.65),
                    (0.0, h * 0.45),
                );
                // 左鼓起到左侧底部
                p.curve_to(
                    (0.0, h * 0.3),
                    (w * 0.15, h * 0.15),
                    (w / 2.0, 0.0),
                );
                p.close_path();
                p
            }
            Self::Custom { path } => path.clone(),
        }
    }
}
