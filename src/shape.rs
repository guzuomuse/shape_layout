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
//!
//! // 葫芦形
//! Gourd(width: 120.0, height: 180.0, waist_y: 0.55, waist_ratio: 0.45)
//!
//! // 吊牌（带孔洞）
//! HangTag(width: 80.0, height: 120.0, hole_y: 100.0, hole_radius: 6.0)
//! ```

use kurbo::{BezPath, Circle, Rect, RoundedRect, Shape};
use serde::{Deserialize, Serialize};

fn default_waist_y() -> f64 {
    0.5
}
fn default_waist_ratio() -> f64 {
    0.5
}
fn default_gourd_width() -> f64 {
    120.0
}
fn default_gourd_height() -> f64 {
    180.0
}
fn default_tag_radius() -> f64 {
    5.0
}

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
    /// 葫芦形（细腰双胞形状，适合瓶身标签等场景）
    ///
    /// ```ron
    /// Gourd(width: 120.0, height: 180.0, waist_y: 0.55, waist_ratio: 0.45)
    /// ```
    Gourd {
        /// 整体宽度（两个鼓包的最大宽度）
        #[serde(default = "default_gourd_width")]
        width: f64,
        /// 整体高度
        #[serde(default = "default_gourd_height")]
        height: f64,
        /// 腰部 Y 位置（0.0=底部，1.0=顶部，默认 0.5 居中）
        #[serde(default = "default_waist_y")]
        waist_y: f64,
        /// 腰部宽度比例（占整体宽度的比例，0.3~0.7，默认 0.5）
        #[serde(default = "default_waist_ratio")]
        waist_ratio: f64,
    },
    /// 吊牌（矩形主体 + 顶部圆形穿绳孔洞）
    ///
    /// ```ron
    /// HangTag(width: 80.0, height: 120.0, hole_y: 100.0, hole_radius: 6.0)
    /// ```
    HangTag {
        /// 宽度
        width: f64,
        /// 高度
        height: f64,
        /// 四角圆角半径（默认 5.0）
        #[serde(default = "default_tag_radius")]
        radius: f64,
        /// 孔洞中心 Y 坐标（距底部）
        hole_y: f64,
        /// 孔洞半径
        hole_radius: f64,
    },
    /// 任意自定义路径（兜底方案，AI 不应直接使用此变体）
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
    ///
    /// 对于带孔洞的变体（如 HangTag），孔洞以 CW 子路径追加，
    /// `RangeGenerator::new()` 会自动识别外轮廓/孔洞。
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
                let r_rect = RoundedRect::from_rect(rect, *radius);
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
            Self::Gourd {
                width,
                height,
                waist_y,
                waist_ratio,
            } => {
                // ── 葫芦形：上下两个鼓包由细腰连接 ──
                // 路径：底部中心 → 右鼓包 → 腰部右侧 → 上鼓包 → 顶部中心
                //        → 上鼓包左侧 → 腰部左侧 → 下鼓包左侧 → 闭合
                let w = *width;
                let h = *height;
                let wy = waist_y.clamp(0.1, 0.9);
                let wr = waist_ratio.clamp(0.25, 0.75);

                let waist_y_pos = h * wy;
                let waist_half = w * wr / 2.0;
                let bulge_x = w * 0.95; // 鼓包最大宽度（略小于总宽，留余地）
                let bot_bulge_y = waist_y_pos * 0.35;
                let top_bulge_y = waist_y_pos + (h - waist_y_pos) * 0.65;

                let mut p = BezPath::new();

                // 底部中心
                p.move_to((w / 2.0, 0.0));

                // — 右侧：底部 → 下鼓包 → 腰部 → 上鼓包 → 顶部 —
                p.curve_to(
                    (w * 0.72, waist_y_pos * 0.06),
                    (bulge_x * 0.98, waist_y_pos * 0.12),
                    (bulge_x, bot_bulge_y),
                );
                p.curve_to(
                    (bulge_x, waist_y_pos * 0.62),
                    (waist_half + (w - waist_half * 2.0) * 0.28, waist_y_pos * 0.88),
                    (waist_half, waist_y_pos),
                );
                p.curve_to(
                    (
                        waist_half + (w - waist_half * 2.0) * 0.28,
                        waist_y_pos + (h - waist_y_pos) * 0.12,
                    ),
                    (bulge_x, waist_y_pos + (h - waist_y_pos) * 0.38),
                    (bulge_x, top_bulge_y),
                );
                p.curve_to(
                    (bulge_x, h * 0.96),
                    (w * 0.72, h),
                    (w / 2.0, h),
                );

                // — 左侧：顶部 → 上鼓包 → 腰部 → 下鼓包 → 底部（镜像）—
                let mirror = |x: f64| w - x;
                p.curve_to(
                    (mirror(w * 0.72), h),
                    (mirror(bulge_x), h * 0.96),
                    (mirror(bulge_x), top_bulge_y),
                );
                p.curve_to(
                    (mirror(bulge_x), waist_y_pos + (h - waist_y_pos) * 0.38),
                    (
                        mirror(waist_half + (w - waist_half * 2.0) * 0.28),
                        waist_y_pos + (h - waist_y_pos) * 0.12,
                    ),
                    (mirror(waist_half), waist_y_pos),
                );
                p.curve_to(
                    (mirror(waist_half + (w - waist_half * 2.0) * 0.28), waist_y_pos * 0.88),
                    (mirror(bulge_x), waist_y_pos * 0.62),
                    (mirror(bulge_x), bot_bulge_y),
                );
                p.curve_to(
                    (mirror(bulge_x * 0.98), waist_y_pos * 0.12),
                    (mirror(w * 0.72), waist_y_pos * 0.06),
                    (w / 2.0, 0.0),
                );

                p.close_path();
                p
            }
            Self::HangTag {
                width,
                height,
                radius,
                hole_y,
                hole_radius,
            } => {
                // ── 吊牌：矩形主体 (CCW) + 顶部圆形孔洞 (CW) ──
                let w = *width;
                let h = *height;
                let r = *radius;

                // 外轮廓：圆角矩形 (CCW)
                let rect = Rect::new(0.0, 0.0, w, h);
                let r_rect = RoundedRect::from_rect(rect, r);
                let mut p = r_rect.to_path(0.1);

                // 孔洞：圆形 (CW，移至正确位置)
                let cx = w / 2.0;
                let cy = *hole_y;
                let hr = *hole_radius;
                // 贝塞尔圆近似常数
                const K: f64 = 0.551915024494;

                // CW 孔洞：从右侧开始顺时针 → signed_area < 0
                p.move_to((cx + hr, cy));
                p.curve_to(
                    (cx + hr, cy - hr * K),
                    (cx + hr * K, cy - hr),
                    (cx, cy - hr),
                );
                p.curve_to(
                    (cx - hr * K, cy - hr),
                    (cx - hr, cy - hr * K),
                    (cx - hr, cy),
                );
                p.curve_to(
                    (cx - hr, cy + hr * K),
                    (cx - hr * K, cy + hr),
                    (cx, cy + hr),
                );
                p.curve_to(
                    (cx + hr * K, cy + hr),
                    (cx + hr, cy + hr * K),
                    (cx + hr, cy),
                );
                p.close_path();

                p
            }
            Self::Custom { path } => path.clone(),
        }
    }
}
