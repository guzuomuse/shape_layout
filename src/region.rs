//! 扫描线几何引擎 — RangeGenerator
//!
//! 把复杂的 2D 异形多边形在 Y 轴高度上降维切片成 1D 的线段区间 [L, R]。

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::i_float::float::compatible::FloatPointCompatible;
use kurbo::{BezPath, PathEl, Rect};
use serde::Serialize;

// ═══════════════════════════════════════════════════════════════════════════
// SPoint — ioverlay 兼容的点类型
// ═══════════════════════════════════════════════════════════════════════════

/// 内部使用的 2D 点，实现 `FloatPointCompatible` 以对接 ioverlay
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SPoint {
    pub x: f64,
    pub y: f64,
}

impl SPoint {
    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl FloatPointCompatible for SPoint {
    type Scalar = f64;

    fn from_xy(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn x(&self) -> f64 {
        self.x
    }

    fn y(&self) -> f64 {
        self.y
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BezPath ⇄ ioverlay 轮廓组 转换
// ═══════════════════════════════════════════════════════════════════════════

/// 将 `BezPath` 离散化为轮廓组 `Vec<Vec<SPoint>>`
///
/// 每条子路径（MoveTo … ClosePath）成为一个独立轮廓。
/// 曲线段被 `tolerance` 控制精度做 flatten。
pub fn bezpath_to_contours(path: &BezPath, tolerance: f64) -> Vec<Vec<SPoint>> {
    let mut contours = Vec::new();
    let mut current = Vec::new();

    kurbo::flatten(path, tolerance, |el| match el {
        PathEl::MoveTo(p) => {
            if !current.is_empty() {
                contours.push(std::mem::take(&mut current));
            }
            current.push(SPoint::new(p.x, p.y));
        }
        PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
            current.push(SPoint::new(p.x, p.y));
        }
        PathEl::ClosePath => {
            if !current.is_empty() {
                // 闭合：首尾重复点去重
                if current.len() > 1 && current[0] == *current.last().unwrap() {
                    current.pop();
                }
                contours.push(std::mem::take(&mut current));
            }
        }
    });

    if !current.is_empty() {
        if current.len() > 1 && current[0] == *current.last().unwrap() {
            current.pop();
        }
        contours.push(current);
    }

    contours
}

/// 计算轮廓组的整体 AABB
fn contours_aabb(contours: &[Vec<SPoint>]) -> Option<Rect> {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut has_data = false;

    for contour in contours {
        for pt in contour {
            min_x = min_x.min(pt.x);
            min_y = min_y.min(pt.y);
            max_x = max_x.max(pt.x);
            max_y = max_y.max(pt.y);
            has_data = true;
        }
    }

    if has_data {
        Some(Rect::new(min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

/// 计算单轮廓的 AABB
fn contour_aabb(contour: &[SPoint]) -> Rect {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for pt in contour {
        min_x = min_x.min(pt.x);
        min_y = min_y.min(pt.y);
        max_x = max_x.max(pt.x);
        max_y = max_y.max(pt.y);
    }
    Rect::new(min_x, min_y, max_x, max_y)
}

/// 计算多边形有向面积（CCW > 0, CW < 0）
fn signed_area(contour: &[SPoint]) -> f64 {
    let n = contour.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += contour[i].x * contour[j].y - contour[j].x * contour[i].y;
    }
    area * 0.5
}

/// 构造一个矩形轮廓
fn rect_contour(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<SPoint> {
    vec![
        SPoint::new(x0, y0),
        SPoint::new(x1, y0),
        SPoint::new(x1, y1),
        SPoint::new(x0, y1),
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// RowRange — 单行区间查询结果
// ═══════════════════════════════════════════════════════════════════════════

/// 单一行的绝对安全区间
///
/// `intervals` 中的每个 `(l, r)` 表示该行内 X 轴上一段连续的可用空间。
#[derive(Debug, Clone, Serialize)]
pub struct RowRange {
    /// 本行起始 Y 坐标
    pub y_start: f64,
    /// 本行高度
    pub height: f64,
    /// 安全区间列表，按 X 升序排列，互不重叠
    pub intervals: Vec<(f64, f64)>,
}

impl RowRange {
    /// 本行结束 Y 坐标
    #[inline]
    pub fn y_end(&self) -> f64 {
        self.y_start + self.height
    }

    /// 本行是否没有任何可用区间
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// 本行所有可用区间的总宽度
    pub fn total_width(&self) -> f64 {
        self.intervals.iter().map(|(l, r)| r - l).sum()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RangeGenerator — 扫描线引擎
// ═══════════════════════════════════════════════════════════════════════════

/// 异形容器扫描线引擎
///
/// 初始化时对容器形状做一次离散化和分类（外轮廓 / 孔洞），
/// 之后可以无限次查询任意 Y 行上的安全区间。
///
/// # 使用示例
///
/// ```rust,ignore
/// use kurbo::BezPath;
/// use shape_layout::RangeGenerator;
///
/// let container = BezPath::from_vec(vec![
///     kurbo::PathEl::MoveTo((0.0, 0.0).into()),
///     kurbo::PathEl::LineTo((100.0, 0.0).into()),
///     kurbo::PathEl::LineTo((100.0, 100.0).into()),
///     kurbo::PathEl::LineTo((0.0, 100.0).into()),
///     kurbo::PathEl::ClosePath,
/// ]);
///
/// let gen = RangeGenerator::new(&container).unwrap();
/// let row = gen.get_intervals_at(10.0, 20.0, Some(5.0));
/// assert!(!row.is_empty());
/// ```
pub struct RangeGenerator {
    /// 外轮廓组（CCW）
    outer_contours: Vec<Vec<SPoint>>,
    /// 孔洞轮廓组（CW）
    hole_contours: Vec<Vec<SPoint>>,
    /// 容器整体 AABB
    pub extents: Rect,
    /// 贝塞尔曲线离散化精度
    #[allow(dead_code)]
    tolerance: f64,
}

impl RangeGenerator {
    const EPSILON: f64 = 1e-6;

    /// 从 `BezPath` 创建扫描线引擎
    ///
    /// 自动根据有向面积识别：
    /// - 面积 > 0 → 外轮廓（CCW）
    /// - 面积 < 0 → 孔洞（CW，内部自动翻转为 CW 存储）
    ///
    /// 返回 `None` 如果形状没有有效的外轮廓。
    pub fn new(shape: &BezPath) -> Option<Self> {
        let tolerance = 0.1;
        let contours = bezpath_to_contours(shape, tolerance);

        let mut outer = Vec::new();
        let mut holes = Vec::new();

        for contour in contours {
            if contour.len() < 3 {
                continue;
            }
            let area = signed_area(&contour);
            if area > 0.0 {
                // 外轮廓，保持 CCW
                outer.push(contour);
            } else {
                // 孔洞，翻转为 CW 存储，以便 ioverlay 识别
                let mut hole = contour;
                hole.reverse();
                holes.push(hole);
            }
        }

        if outer.is_empty() {
            return None;
        }

        let extents = contours_aabb(&outer)?;

        Some(Self {
            outer_contours: outer,
            hole_contours: holes,
            extents,
            tolerance,
        })
    }

    /// 查询指定 Y 行上的安全区间
    ///
    /// # 参数
    ///
    /// - `y_start`：行顶部 Y 坐标
    /// - `height`：行高度
    /// - `min_width_opt`：最小区间宽度过滤（`None` = 不过滤）
    ///
    /// # 返回
    ///
    /// `RowRange` 包含所有宽度 ≥ min_width 的安全区间，按 X 升序排列。
    pub fn get_intervals_at(
        &self,
        y_start: f64,
        height: f64,
        min_width_opt: Option<f64>,
    ) -> RowRange {
        let jitter = 1e-4;
        let adj_y_start = y_start + jitter;
        let adj_y_end = y_start + height - jitter;

        // 1. 构造切片矩形（略宽于容器，确保覆盖全部）
        let row_rect = rect_contour(
            self.extents.x0 - 1.0,
            adj_y_start,
            self.extents.x1 + 1.0,
            adj_y_end,
        );
        let row_rect_wrapped = vec![row_rect];

        // 2. 外轮廓 AND 切片矩形 → 基础安全区间
        let mut final_intervals: Vec<(f64, f64)> = Vec::new();

        for outer in &self.outer_contours {
            let outer_wrapped = vec![outer.clone()];
            let result =
                outer_wrapped.overlay(&row_rect_wrapped, OverlayRule::Intersect, FillRule::NonZero);

            for shape in &result {
                for contour in shape {
                    let ext = contour_aabb(contour);
                    // 只保留高度达标的碎块
                    if ext.height() >= (height - 3.0 * jitter) {
                        if let Some((l, r)) = Self::find_bottleneck(contour) {
                            final_intervals.push((l, r));
                        }
                    }
                }
            }
        }

        // 3. 孔洞裁剪：对每个孔洞做 AND，得到禁区，1D 裁剪所有区间
        for hole in &self.hole_contours {
            let hole_wrapped = vec![hole.clone()];
            let result =
                hole_wrapped.overlay(&row_rect_wrapped, OverlayRule::Intersect, FillRule::NonZero);

            for shape in &result {
                for contour in shape {
                    let ext = contour_aabb(contour);
                    let forbidden = (ext.x0, ext.x1);
                    final_intervals =
                        Self::subtract_1d_interval(final_intervals, forbidden);
                }
            }
        }

        // 4. 宽度过滤 + 排序
        let min_width = min_width_opt.unwrap_or(0.0);
        final_intervals.retain(|(l, r)| (r - l) >= min_width);
        final_intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // 调试日志：记录靠近心形右尖的行级别区间
        const HEART_RIGHT_TIP_X: f64 = 200.0;
        if final_intervals.iter().any(|(_, r)| *r > HEART_RIGHT_TIP_X) {
            println!(
                "[row_interval] y_start={:.3} height={:.3} intervals={:?}",
                y_start, height, final_intervals,
            );
        }

        RowRange {
            y_start,
            height,
            intervals: final_intervals,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 内部算法
    // ═══════════════════════════════════════════════════════════════════════

    /// 从扁平化的 AND 结果多边形中找出 X 轴瓶颈区间 `[safe_left, safe_right]`
    ///
    /// 使用边缘方向分类法：对 CCW 多边形，
    /// - 向下走的边（dy < 0）是左墙，取 `max_x` 收紧左边界
    /// - 向上走的边（dy > 0）是右墙，取 `min_x` 收紧右边界
    ///
    /// 对凸/凹多边形均数学上正确，不依赖采样密度。
    fn find_bottleneck(contour: &[SPoint]) -> Option<(f64, f64)> {
        if contour.len() < 3 {
            return None;
        }

        // 确保 CCW（边缘方向分类法的前提）
        let area = signed_area(contour);
        let working = if area < 0.0 {
            let mut reversed = contour.to_vec();
            reversed.reverse();
            reversed
        } else {
            contour.to_vec()
        };
        let n = working.len();

        let mut safe_left = f64::NEG_INFINITY;
        let mut safe_right = f64::INFINITY;

        for i in 0..n {
            let v1 = &working[i];
            let v2 = &working[(i + 1) % n];

            let dy = v2.y - v1.y;
            if dy.abs() < Self::EPSILON {
                continue; // 水平边不参与分类
            }

            // 等效于 cavalier_contours::seg_bounding_box(v1, v2)
            // ioverlay 产出的全部是直线段，所以直接用端点 X 范围即可
            let seg_min_x = v1.x.min(v2.x);
            let seg_max_x = v1.x.max(v2.x);

            if dy < 0.0 {
                // 左墙：向下走 → 取该段最右点作为安全左边界
                safe_left = safe_left.max(seg_max_x);
            } else {
                // 右墙：向上走 → 取该段最左点作为安全右边界
                safe_right = safe_right.min(seg_min_x);
            }
        }

        if safe_right > safe_left + Self::EPSILON {
            // 调试日志：记录靠近心形右尖的瓶颈区间
            const HEART_RIGHT_TIP_X: f64 = 200.0;
            if safe_right > HEART_RIGHT_TIP_X {
                let aabb = contour_aabb(&working);
                println!(
                    "[bottleneck] method=edge_dir safe=({:.3}, {:.3}) width={:.3} | contour_bbox=({:.3}, {:.3}, {:.3}, {:.3}) vertices={}",
                    safe_left, safe_right, safe_right - safe_left,
                    aabb.x0, aabb.y0, aabb.x1, aabb.y1,
                    working.len(),
                );
            }
            Some((safe_left, safe_right))
        } else {
            None
        }
    }

    /// 一维线段裁剪：从 current 中减去 forbidden 区间
    fn subtract_1d_interval(
        current: Vec<(f64, f64)>,
        forbidden: (f64, f64),
    ) -> Vec<(f64, f64)> {
        let mut next = Vec::new();
        let (f_min, f_max) = forbidden;

        for (s, e) in current {
            if e <= f_min || s >= f_max {
                // 完全无重叠，保留
                next.push((s, e));
            } else {
                // 发生重叠，切分
                if s < f_min {
                    next.push((s, f_min));
                }
                if e > f_max {
                    next.push((f_max, e));
                }
            }
        }
        next
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个简单的正方形 BezPath
    fn square(x: f64, y: f64, size: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x, y));
        p.line_to((x + size, y));
        p.line_to((x + size, y + size));
        p.line_to((x, y + size));
        p.close_path();
        p
    }

    /// 构造一个带正方形孔洞的外框
    fn square_with_hole(outer_size: f64, hole_size: f64) -> BezPath {
        let margin = (outer_size - hole_size) / 2.0;
        let mut p = BezPath::new();
        // 外轮廓 CCW
        p.move_to((0.0, 0.0));
        p.line_to((outer_size, 0.0));
        p.line_to((outer_size, outer_size));
        p.line_to((0.0, outer_size));
        p.close_path();
        // 孔洞 CW
        p.move_to((margin, margin));
        p.line_to((margin, margin + hole_size));
        p.line_to((margin + hole_size, margin + hole_size));
        p.line_to((margin + hole_size, margin));
        p.close_path();
        p
    }

    #[test]
    fn test_simple_square() {
        let shape = square(0.0, 0.0, 100.0);
        let rg = RangeGenerator::new(&shape).unwrap();

        let row = rg.get_intervals_at(10.0, 20.0, None);
        assert_eq!(row.intervals.len(), 1);
        let (l, r) = row.intervals[0];
        assert!(l < 1.0);
        assert!(r > 99.0);
    }

    #[test]
    fn test_square_with_hole() {
        let shape = square_with_hole(100.0, 40.0);
        let rg = RangeGenerator::new(&shape).unwrap();

        let row = rg.get_intervals_at(30.0, 20.0, None);
        assert_eq!(row.intervals.len(), 2);

        let (l1, r1) = row.intervals[0];
        let (l2, r2) = row.intervals[1];

        assert!(l1 < 31.0);
        assert!(r1 < 32.0);
        assert!(l2 > 68.0);
        assert!(r2 > 99.0);
    }

    #[test]
    fn test_min_width_filter() {
        let shape = square(0.0, 0.0, 100.0);
        let rg = RangeGenerator::new(&shape).unwrap();

        let row = rg.get_intervals_at(10.0, 20.0, Some(200.0));
        assert!(row.is_empty());
    }

    #[test]
    fn test_row_y_end() {
        let shape = square(0.0, 0.0, 100.0);
        let rg = RangeGenerator::new(&shape).unwrap();

        let row = rg.get_intervals_at(10.0, 20.0, None);
        assert!((row.y_end() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn test_total_width() {
        let shape = square(0.0, 0.0, 100.0);
        let rg = RangeGenerator::new(&shape).unwrap();

        let row = rg.get_intervals_at(10.0, 20.0, None);
        assert!(row.total_width() > 98.0);
        assert!(row.total_width() < 102.0);
    }

    #[test]
    fn test_empty_shape() {
        let shape = BezPath::new();
        let rg = RangeGenerator::new(&shape);
        assert!(rg.is_none());
    }
}
