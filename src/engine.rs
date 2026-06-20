//! 排版主循环 — kasuari 约束求解引擎
//!
//! 核心函数 `layout_rows()` 实现三层架构的调度逻辑：
//! 1. RangeGenerator 切割行区间（眼睛）
//! 2. 贪心分行调度（调度器）
//! 3. kasuari 行内约束求解（大脑）
//!
//! 约束求解只管 X 轴：left, width, gap, alignment。
//! 不碰 Y、不碰旋转、不碰异形。

use kasuari::WeightedRelation::*;
use kasuari::{Solver, Strength, Variable};
use kurbo::BezPath;

use crate::element::LayoutElement;
use crate::region::RangeGenerator;
use crate::result::{LayoutSolution, LayoutWarning, PlacedElement};
use crate::rules::{HAlign, LayoutConfig, VAlign};

/// 单次排版求解：在异形容器内按竖直流式排放矩形元素
///
/// # 参数
/// - `container`：容器形状（BezPath，支持孔洞）
/// - `elements`：待排版元素列表（按输入顺序尝试排放）
/// - `config`：全局排版配置（边距、间距、对齐）
///
/// # 返回
/// `LayoutSolution` 包含已排放元素、未排放元素、警告信息。
///
/// # 算法
///
/// ```text
/// y = container.y0 + padding_top
/// while 有剩余元素:
///     row_h = 下一批元素的预估行高
///     intervals = rg.get_intervals_at(y, row_h)
///     取最宽区间 (L, R)
///     贪心塞入元素直到放不下
///     kasuari 求解行内约束 → 得到每个元素的 X
///     记录 PlacedElement { x, y, width, height }
///     y += row_h + line_spacing
/// ```
pub fn layout_rows(
    container: &BezPath,
    elements: &[LayoutElement],
    config: &LayoutConfig,
) -> LayoutSolution {
    // 1. 创建扫描线引擎
    let rg = match RangeGenerator::new(container) {
        Some(rg) => rg,
        None => return LayoutSolution::invalid_container(elements),
    };

    let max_y = rg.extents.y1 - config.padding_bottom;
    let container_y0 = rg.extents.y0;
    let mut placed: Vec<PlacedElement> = Vec::new();
    let mut warnings: Vec<LayoutWarning> = Vec::new();
    let mut unplaced: Vec<String> = Vec::new();

    let mut y = container_y0 + config.padding_top;
    let mut idx: usize = 0;

    while idx < elements.len() {
        // 检查容器底部溢出（留 1e-9 容差，避免误杀刚好贴底的行）
        if y > max_y + 1e-9 {
            for i in idx..elements.len() {
                warnings.push(LayoutWarning::Overflow {
                    element_id: elements[i].id.clone(),
                    message: format!(
                        "container bottom reached at y={:.1}, max_y={:.1}",
                        y, max_y
                    ),
                });
                unplaced.push(elements[i].id.clone());
            }
            break;
        }

        // 2. 预估行高 = 当前元素的高度（后续会随加入元素而增大）
        let mut row_height = elements[idx].height;

        // 3. 查询这一行的可用区间
        let row_range = rg.get_intervals_at(y, row_height, config.min_width);

        if row_range.is_empty() {
            // 无可用的行区间，推进 Y 继续尝试
            y += config.step_size;
            continue;
        }

        // 取最宽区间
        let widest = row_range
            .intervals
            .iter()
            .max_by(|a, b| {
                (a.1 - a.0)
                    .partial_cmp(&(b.1 - b.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("row_range is non-empty");

        let mut interval_l = widest.0 + config.padding_left;
        let mut interval_r = widest.1 - config.padding_right;

        // 边界保护
        if interval_r - interval_l <= 0.0 {
            y += config.step_size;
            continue;
        }

        // 4. 贪心塞入元素
        let (row_indices, new_idx, final_row_height) =
            pack_row_elements(elements, idx, interval_r - interval_l, config, &mut warnings);

        if row_indices.is_empty() {
            // 连第一个元素都放不下 → 用行高推进 Y（而非微小步长）
            y += row_height.max(config.step_size);
            continue;
        }

        idx = new_idx;
        row_height = final_row_height;

        // 用实际行高重新查询区间（行内可能有更高元素改变了有效高度）
        let refined_row = rg.get_intervals_at(y, row_height, config.min_width);
        if !refined_row.is_empty() {
            if let Some(widest_refined) = refined_row.intervals.iter().max_by(|a, b| {
                (a.1 - a.0)
                    .partial_cmp(&(b.1 - b.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                interval_l = widest_refined.0 + config.padding_left;
                interval_r = widest_refined.1 - config.padding_right;
            }
        }

        // 校验 refinement 后的区间仍能容纳行内元素（防止沙漏形容器因行高增大
        // 导致区间缩水到放不下已打包元素，退回原始区间）
        if !row_indices.is_empty() {
            let row_min_span: f64 = row_indices
                .iter()
                .map(|&ri| {
                    let e = &elements[ri];
                    if e.constraints.shrinkable {
                        e.footprint_width_with(e.constraints.min_width.unwrap_or(0.0))
                    } else {
                        e.footprint_width()
                    }
                })
                .sum::<f64>()
                + (row_indices.len().saturating_sub(1)) as f64 * config.gap;
            if interval_r - interval_l < row_min_span - 1e-9 {
                // refinement 后区间太窄，回退到原始区间
                interval_l = widest.0 + config.padding_left;
                interval_r = widest.1 - config.padding_right;
            }
        }

        // Bug B fix: 检查整行（含已打包元素）是否在容器底部以内
        if y + row_height > max_y + 1e-9 {
            // 已打包到行内的元素
            for &ri in &row_indices {
                warnings.push(LayoutWarning::Overflow {
                    element_id: elements[ri].id.clone(),
                    message: format!(
                        "row exceeds container bottom: y={:.1} + row_height={:.1} > max_y={:.1}",
                        y, row_height, max_y
                    ),
                });
                unplaced.push(elements[ri].id.clone());
            }
            // 尚未处理的后缀元素
            for i in idx..elements.len() {
                warnings.push(LayoutWarning::Overflow {
                    element_id: elements[i].id.clone(),
                    message: format!(
                        "row exceeds container bottom: y={:.1} + row_height={:.1} > max_y={:.1}",
                        y, row_height, max_y
                    ),
                });
                unplaced.push(elements[i].id.clone());
            }
            break;
        }

        // 5. kasuari 求解行内 X 位置
        match solve_row_x(
            elements,
            &row_indices,
            interval_l,
            interval_r,
            config,
        ) {
            Ok(x_solutions) => {
                for (elem_idx, resolved_x, resolved_width) in x_solutions {
                    let elem = &elements[elem_idx];

                    // ── VAlign: 垂直对齐 + margin ──
                    let viz_height = elem.height;
                    let total_height = elem.footprint_height();
                    let final_y = match config.valign {
                        VAlign::Top => y + elem.margin.top,
                        VAlign::Middle => {
                            y + elem.margin.top + (row_height - total_height) / 2.0
                        }
                        VAlign::Bottom => {
                            y + row_height - elem.margin.bottom - viz_height
                        }
                    };

                    placed.push(PlacedElement {
                        id: elem.id.clone(),
                        x: resolved_x + elem.margin.left,
                        y: final_y,
                        width: resolved_width - elem.margin.left - elem.margin.right,
                        height: viz_height,
                    });
                }
            }
            Err(err_msg) => {
                // 约束求解失败 → 整行元素全部不可排入
                warnings.push(LayoutWarning::ConstraintConflict(err_msg.clone()));
                for &ei in &row_indices {
                    warnings.push(LayoutWarning::ConstraintConflict(format!(
                        "element '{}' unplaced due to row constraint failure",
                        elements[ei].id
                    )));
                    unplaced.push(elements[ei].id.clone());
                }
            }
        }

        y += row_height + config.line_spacing;
    }

    LayoutSolution {
        placed,
        unplaced,
        warnings,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 行打包逻辑
// ═══════════════════════════════════════════════════════════════════════════

/// 贪心打包一行元素
///
/// 返回 `(row_indices, next_idx, row_height)`
fn pack_row_elements(
    elements: &[LayoutElement],
    start_idx: usize,
    available_width: f64,
    config: &LayoutConfig,
    warnings: &mut Vec<LayoutWarning>,
) -> (Vec<usize>, usize, f64) {
    let mut row_indices: Vec<usize> = Vec::new();
    let mut used_width = 0.0;
    let mut row_height: f64 = 0.0;
    let mut idx = start_idx;

    while idx < elements.len() {
        let elem = &elements[idx];

        // 尝试首选宽度（含 margin 占地）
        let footprint_w = elem.footprint_width();
        let gap_needed = if row_indices.is_empty() {
            0.0
        } else {
            config.gap
        };

        let total_needed = used_width + gap_needed + footprint_w;

        if total_needed <= available_width + 1e-9 {
            // 放得下
            row_indices.push(idx);
            used_width = total_needed;
            row_height = row_height.max(elem.footprint_height());
            idx += 1;
        } else if elem.constraints.shrinkable {
            // 尝试缩到最小宽度（含 margin）
            let min_w = elem
                .constraints
                .min_width
                .unwrap_or(0.0)
                .max(0.0);
            let min_footprint = elem.footprint_width_with(min_w);
            let total_min = used_width + gap_needed + min_footprint;

            if total_min <= available_width + 1e-9 {
                row_indices.push(idx);
                used_width = total_min;
                row_height = row_height.max(elem.footprint_height());
                idx += 1;
            } else {
                // 哪怕最小宽度也放不下，这行到此为止
                break;
            }
        } else {
            // 不可缩且放不下 → 行到此为止，下一个元素开新行
            if row_indices.is_empty() {
                // 单元素也放不下 → 记录警告
                let occupied_span = elem.footprint_width();
                warnings.push(LayoutWarning::ElementTooWide {
                    element_id: elem.id.clone(),
                    min_width: occupied_span,
                    max_available: available_width,
                });
            }
            break;
        }
    }

    (row_indices, idx, row_height)
}

// ═══════════════════════════════════════════════════════════════════════════
// kasuari 行内约束求解
// ═══════════════════════════════════════════════════════════════════════════

/// 用 kasuari 求解一行的 X 轴布局
///
/// 返回 `Vec<(elem_index, x, width)>` 或错误信息。
fn solve_row_x(
    elements: &[LayoutElement],
    row_indices: &[usize],
    interval_l: f64,
    interval_r: f64,
    config: &LayoutConfig,
) -> Result<Vec<(usize, f64, f64)>, String> {
    let n = row_indices.len();
    if n == 0 {
        return Ok(vec![]);
    }

    let mut solver = Solver::new();

    // 为每个元素创建 left / right 变量
    let mut left_vars: Vec<Variable> = Vec::with_capacity(n);
    let mut right_vars: Vec<Variable> = Vec::with_capacity(n);

    for _ in 0..n {
        left_vars.push(Variable::new());
        right_vars.push(Variable::new());
    }

    // ── 边界约束 ──
    // left_i >= interval_l
    for i in 0..n {
        solver
            .add_constraints([left_vars[i] | GE(Strength::REQUIRED) | interval_l])
            .map_err(|e| format!("add_constraint left>=L failed: {e:?}"))?;
    }

    // right_i <= interval_r
    for i in 0..n {
        solver
            .add_constraints([right_vars[i] | LE(Strength::REQUIRED) | interval_r])
            .map_err(|e| format!("add_constraint right<=R failed: {e:?}"))?;
    }

    // ── 宽度约束（含 margin 占地）──
    for i in 0..n {
        let elem = &elements[row_indices[i]];
        let footprint_w = elem.footprint_width();

        // right_i - left_i == footprint_width
        // 不可缩元素用 REQUIRED（硬锁，宁愿求解失败也不允许被压扁）
        // 可缩元素用 STRONG（边界约束为 REQUIRED 时 kasuari 会自动缩短）
        let width_strength = if elem.constraints.shrinkable {
            Strength::STRONG
        } else {
            Strength::REQUIRED
        };
        solver
            .add_constraints(
                [(right_vars[i] - left_vars[i]) | EQ(width_strength) | footprint_w],
            )
            .map_err(|e| format!("add_constraint width==preferred failed: {e:?}"))?;

        // min_width 约束（含 margin）
        if let Some(min_w) = elem.constraints.min_width {
            let min_footprint = elem.footprint_width_with(min_w);
            if min_footprint > 0.0 {
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i]) | GE(Strength::REQUIRED) | min_footprint],
                    )
                    .map_err(|e| format!("add_constraint width>=min failed: {e:?}"))?;
            }
        }

        // max_width 约束（含 margin）
        if let Some(max_w) = elem.constraints.max_width {
            let max_footprint = elem.footprint_width_with(max_w);
            solver
                .add_constraints(
                    [(right_vars[i] - left_vars[i]) | LE(Strength::REQUIRED) | max_footprint],
                )
                .map_err(|e| format!("add_constraint width<=max failed: {e:?}"))?;
        }
    }

    // ── 间距约束：left_{i+1} >= right_i + gap ──
    // margin 已纳入 footprint 宽度，gap 直接叠加即可
    if n > 1 {
        for i in 0..(n - 1) {
            solver
                .add_constraints(
                    [(left_vars[i + 1] - right_vars[i]) | GE(Strength::REQUIRED) | config.gap],
                )
                .map_err(|e| format!("add_constraint gap failed: {e:?}"))?;
        }
    }

    // ── 对齐约束 ──
    match config.halign {
        HAlign::Left => {
            // left_0 == interval_l (STRONG)
            solver
                .add_constraints([left_vars[0] | EQ(Strength::STRONG) | interval_l])
                .map_err(|e| format!("add_constraint align-left failed: {e:?}"))?;
        }
        HAlign::Right => {
            // right_last == interval_r (STRONG)
            let last = n - 1;
            solver
                .add_constraints([right_vars[last] | EQ(Strength::STRONG) | interval_r])
                .map_err(|e| format!("add_constraint align-right failed: {e:?}"))?;
        }
        HAlign::Center => {
            // 真正的居中：元素组的几何中心 == 区间中心
            let last = n - 1;
            let total_width = right_vars[last] - left_vars[0];
            let center = (interval_l + interval_r) / 2.0;
            solver
                .add_constraints(
                    [(left_vars[0] + total_width / 2.0) | EQ(Strength::STRONG) | center],
                )
                .map_err(|e| format!("add_constraint align-center failed: {e:?}"))?;
        }
    }

    // 收割结果
    let mut results: Vec<(usize, f64, f64)> = Vec::with_capacity(n);
    for i in 0..n {
        let left = solver.get_value(left_vars[i]);
        let right = solver.get_value(right_vars[i]);
        let width = right - left;
        results.push((row_indices[i], left, width.max(0.0)));
    }

    Ok(results)
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::LayoutElement;
    use crate::ElementMargin;

    fn square(x: f64, y: f64, size: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x, y));
        p.line_to((x + size, y));
        p.line_to((x + size, y + size));
        p.line_to((x, y + size));
        p.close_path();
        p
    }

    #[test]
    fn test_simple_layout_left_align() {
        let container = square(0.0, 0.0, 100.0);
        let elements = vec![
            LayoutElement::new("a", 30.0, 20.0),
            LayoutElement::new("b", 40.0, 20.0),
            LayoutElement::new("c", 20.0, 20.0),
        ];
        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed(), "warnings: {solution:?}");
        assert_eq!(solution.placed.len(), 3);

        // 元素 a 左对齐，应该有 x ≈ 5.0
        let a = &solution.placed[0];
        assert!((a.x - 5.0).abs() < 1.0, "a.x={}", a.x);

        // b 应该在 a 右边
        let b = &solution.placed[1];
        assert!(b.x > a.x + a.width);
    }

    #[test]
    fn test_layout_multi_row() {
        let container = square(0.0, 0.0, 100.0);
        // 两个宽元素，一个放不下→换行
        let elements = vec![
            LayoutElement::new("wide1", 60.0, 20.0),
            LayoutElement::new("wide2", 70.0, 20.0),
            LayoutElement::new("small", 30.0, 20.0),
        ];
        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);

        let solution = layout_rows(&container, &elements, &config);
        // wide1 第一行，wide2 第二行，small 可能跟 wide2 同行也可能第三行
        assert!(solution.placed.len() >= 2);
        // 不同行的元素 y 应该不同
    }

    #[test]
    fn test_center_align() {
        let container = square(0.0, 0.0, 100.0);
        let elements = vec![LayoutElement::new("center_me", 40.0, 20.0)];
        let config = LayoutConfig::with_alignment(0.0, 0.0, 0.0, HAlign::Center);

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());
        let elem = &solution.placed[0];
        // 居中：(100 - 40) / 2 = 30
        assert!((elem.x - 30.0).abs() < 1.0, "expected x≈30, got {}", elem.x);
    }

    #[test]
    fn test_right_align() {
        let container = square(0.0, 0.0, 100.0);
        let elements = vec![LayoutElement::new("right_me", 40.0, 20.0)];
        let config = LayoutConfig::with_alignment(0.0, 0.0, 0.0, HAlign::Right);

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());
        let elem = &solution.placed[0];
        // 右对齐：100 - 40 = 60
        assert!((elem.x - 60.0).abs() < 1.0, "expected x≈60, got {}", elem.x);
    }

    #[test]
    fn test_element_too_wide() {
        let container = square(0.0, 0.0, 50.0);
        let elements = vec![LayoutElement::new("too_wide", 100.0, 20.0)];
        let config = LayoutConfig::default();

        let solution = layout_rows(&container, &elements, &config);
        assert!(!solution.is_fully_placed());
        assert_eq!(solution.unplaced.len(), 1);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 异形容器集成测试（对应 P0/P1 Bug 修复）
    // ═══════════════════════════════════════════════════════════════════════

    /// 直角三角形：顶部宽，底部窄
    fn right_triangle(x: f64, y: f64, base: f64, height: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x, y));
        p.line_to((x + base, y));
        p.line_to((x, y + height));
        p.close_path();
        p
    }

    /// Bug 1 修复验证 —— 容器不在原点时元素应相对于容器放置
    #[test]
    fn test_container_at_offset() {
        // 容器在 (50, 100)，100x100
        let container = square(50.0, 100.0, 100.0);
        let elements = vec![
            LayoutElement::new("a", 40.0, 20.0),
            LayoutElement::new("b", 40.0, 20.0),
        ];
        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);

        let solution = layout_rows(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "unplaced: {:?}, warnings: {:?}",
            solution.unplaced,
            solution.warnings
        );

        // 元素 Y 应该 ≥ container_y0 + padding_top = 105
        for placed in &solution.placed {
            assert!(
                placed.y >= 105.0,
                "element '{}' y={} < 105 (should be >= container_y0 + padding_top)",
                placed.id,
                placed.y
            );
            // X 应该 ≥ container_x0 + padding_left = 55
            assert!(
                placed.x >= 55.0 - 1e-6,
                "element '{}' x={} < 55 (should be >= container_x0 + padding_left)",
                placed.id,
                placed.x
            );
        }
    }

    /// Bug 4 修复验证 —— 元素高度恰好贴底时不应被误杀
    #[test]
    fn test_bottom_fit() {
        // 20 高的容器，20 高的元素，刚好贴底
        let mut container = BezPath::new();
        container.move_to((0.0, 0.0));
        container.line_to((100.0, 0.0));
        container.line_to((100.0, 20.0));
        container.line_to((0.0, 20.0));
        container.close_path();

        let elements = vec![LayoutElement::new("fits_exactly", 40.0, 20.0)];
        let config = LayoutConfig::default(); // no padding

        let solution = layout_rows(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "should fit at exact bottom, got unplaced: {:?}",
            solution.unplaced
        );
        assert_eq!(solution.placed[0].y, 0.0);
    }

    /// Bug 2 修复验证 —— 不可缩元素在可容纳时正常排放
    #[test]
    fn test_non_shrinkable_protection() {
        let container = square(0.0, 0.0, 60.0);
        let element = LayoutElement::new("fixed", 50.0, 20.0);
        // constraints 默认 shrinkable=false，无需额外设置
        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);

        let solution = layout_rows(&container, &[element], &config);
        // 可用宽度 = 60 - 5*2 = 50，元素正好 50 → 应放置
        assert!(
            solution.is_fully_placed(),
            "50-wide element in 60-wide container should fit"
        );
    }

    /// Bug 2 修复验证 —— 不可缩元素过宽时拒绝被压扁
    #[test]
    fn test_non_shrinkable_refused() {
        let container = square(0.0, 0.0, 40.0);
        let mut fixed = LayoutElement::new("fixed", 50.0, 20.0);
        fixed.constraints.shrinkable = false;
        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);

        let solution = layout_rows(&container, &[fixed], &config);
        // 可用宽度 = 40 - 10 = 30，元素 50 且不可缩 → unplaced
        assert!(
            !solution.is_fully_placed(),
            "non-shrinkable 50-wide element should not fit in 30-wide interval"
        );
        assert_eq!(solution.unplaced.len(), 1);
        assert_eq!(solution.unplaced[0], "fixed");
    }

    /// 窄缩容器（三角形）：顶部宽、底部窄，底部元素可能放不下
    #[test]
    fn test_narrowing_triangle() {
        let container = right_triangle(0.0, 0.0, 100.0, 100.0);
        let elements: Vec<LayoutElement> = (0..6)
            .map(|i| LayoutElement::new(&format!("e{i}"), 30.0, 15.0))
            .collect();
        let config = LayoutConfig::with_spacing(2.0, 2.0, 2.0);

        let solution = layout_rows(&container, &elements, &config);
        // 三角形顶部宽 100，底部趋近 0，至少应排放 ≥ 2 个
        assert!(
            solution.placed.len() >= 2,
            "expected ≥2 placed in triangle, got {} placed",
            solution.placed.len()
        );
        // 越靠下的元素 Y 越大
        for w in solution.placed.windows(2) {
            assert!(w[1].y >= w[0].y, "rows should go downward");
        }
    }

    /// Bug 5 修复验证 —— 自定义 step_size 替换魔数 0.5
    #[test]
    fn test_step_size_custom() {
        let container = square(0.0, 0.0, 100.0);
        let elements = vec![LayoutElement::new("a", 40.0, 20.0)];
        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.step_size = 0.5; // default
        let sol_default = layout_rows(&container, &elements, &config);

        config.step_size = 10.0;
        let sol_big = layout_rows(&container, &elements, &config);

        // 两种 step 都能正常排放（因为容器足够宽）
        assert!(sol_default.is_fully_placed());
        assert!(sol_big.is_fully_placed());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Phase 1.5 新功能测试
    // ═══════════════════════════════════════════════════════════════════════

    /// VAlign::Middle —— 行内矮元素垂直居中
    #[test]
    fn test_valign_middle() {
        let container = square(0.0, 0.0, 200.0);
        let elements = vec![
            LayoutElement::new("tall", 30.0, 50.0),  // 行内最高
            LayoutElement::new("short", 30.0, 10.0), // 应该垂直居中
        ];
        let mut config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);
        config.valign = VAlign::Middle;

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());

        // tall 元素：高度等于行高，y 不变
        let tall = &solution.placed[0];
        assert!((tall.y - 5.0).abs() < 1.0, "tall y should ≈5");

        // short 元素：居中 y = 5 + (50-10)/2 = 25
        let short = &solution.placed[1];
        assert!(
            (short.y - 25.0).abs() < 1.0,
            "short valign=middle y should ≈25, got {}",
            short.y
        );
    }

    /// VAlign::Bottom —— 行内矮元素贴底
    #[test]
    fn test_valign_bottom() {
        let container = square(0.0, 0.0, 200.0);
        let elements = vec![
            LayoutElement::new("tall", 30.0, 50.0),
            LayoutElement::new("short", 30.0, 10.0),
        ];
        let mut config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);
        config.valign = VAlign::Bottom;

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());

        let short = &solution.placed[1];
        // bottom: y = 5 + 50 - 10 = 45
        assert!(
            (short.y - 45.0).abs() < 1.0,
            "short valign=bottom y should ≈45, got {}",
            short.y
        );
    }

    /// VAlign::Top 是默认，不需要特殊处理
    #[test]
    fn test_valign_top_default() {
        let container = square(0.0, 0.0, 200.0);
        let elements = vec![
            LayoutElement::new("tall", 30.0, 50.0),
            LayoutElement::new("short", 30.0, 10.0),
        ];
        let config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());

        let short = &solution.placed[1];
        assert!((short.y - 5.0).abs() < 1.0, "short valign=top y should ≈5");
    }

    /// 元素 Margin —— 水平 margin 增加占地面积
    #[test]
    fn test_element_margin_horizontal() {
        let container = square(0.0, 0.0, 100.0);
        let elements = vec![
            LayoutElement::with_margin(
                "a", 30.0, 20.0,
                ElementMargin::horizontal(5.0),
            ),
            LayoutElement::new("b", 20.0, 20.0),
        ];
        // a 的占地 = 5 + 30 + 5 = 40
        // b 的占地 = 20
        // 可用宽度 = 100 - 5*2 = 90, gap = 2
        // 40 + 2 + 20 = 62 ≤ 90 → 同行
        let config = LayoutConfig::with_spacing(5.0, 2.0, 5.0);

        let solution = layout_rows(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "warnings: {:?}",
            solution.warnings
        );
        assert_eq!(solution.placed.len(), 2);

        // a 的视觉 x 应该 = padding_left + margin.left = 5 + 5 = 10
        let a = &solution.placed[0];
        assert!(
            (a.x - 10.0).abs() < 1.0,
            "a x should ≈10 (padding + margin_left), got {}",
            a.x
        );
        // a 的 visual width = 30 (不含 margin)
        assert!(
            (a.width - 30.0).abs() < 1.0,
            "a width should ≈30, got {}",
            a.width
        );
    }

    /// 元素 Margin —— 垂直 margin 影响行高和 VAlign
    #[test]
    fn test_element_margin_vertical_valign() {
        let container = square(0.0, 0.0, 200.0);
        let elements = vec![
            LayoutElement::new("plain", 30.0, 20.0),
            LayoutElement::with_margin(
                "margined",
                30.0,
                10.0,
                ElementMargin {
                    left: 0.0,
                    right: 0.0,
                    top: 5.0,
                    bottom: 5.0,
                },
            ),
        ];
        let mut config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);
        config.valign = VAlign::Middle;

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());

        // row_height = max(20, 5+10+5) = 20 (plain 更高)
        // margined element Middle: y + margin.top + (row_height - footprint_height) / 2
        //   = 5 + 5 + (20 - 20) / 2 = 10
        let margined = &solution.placed[1];
        assert!(
            (margined.y - 10.0).abs() < 1.0,
            "margined middle y should ≈10, got {}",
            margined.y
        );
    }

    /// Bug B 修复 —— 行高超过容器底部时整行溢出
    #[test]
    fn test_row_exceeds_container_bottom() {
        // 容器 200x30，padding 5/5 → 可用高度 20
        let mut container = BezPath::new();
        container.move_to((0.0, 0.0));
        container.line_to((200.0, 0.0));
        container.line_to((200.0, 30.0));
        container.line_to((0.0, 30.0));
        container.close_path();

        // 元素高 25，加上 padding 5*2 = 10，一行就超过容器底部
        let elements = vec![
            LayoutElement::new("too_tall", 30.0, 25.0),
            LayoutElement::new("next", 30.0, 10.0),
        ];
        let config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);
        // y=5, row_height=25 → y+row_height=30, max_y=25 → overflow
        // 两个元素都应 unplaced

        let solution = layout_rows(&container, &elements, &config);
        assert!(!solution.is_fully_placed());
        assert_eq!(solution.unplaced.len(), 2);
        // 应该有 Overflow 警告
        assert!(solution
            .warnings
            .iter()
            .any(|w| matches!(w, LayoutWarning::Overflow { .. })));
    }

    /// Bug B 修复 —— 刚好贴底的行可以排放
    #[test]
    fn test_row_exactly_fits_bottom() {
        // 容器 200x30，padding 5/5 → max_y=25
        let mut container = BezPath::new();
        container.move_to((0.0, 0.0));
        container.line_to((200.0, 0.0));
        container.line_to((200.0, 30.0));
        container.line_to((0.0, 30.0));
        container.close_path();

        let elements = vec![LayoutElement::new("fits", 30.0, 20.0)];
        let config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);
        // y=5, row_height=20 → y+row_height=25 = max_y → 恰好贴底，应排放

        let solution = layout_rows(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "should fit exactly at bottom"
        );
    }

    /// Margin 导致单元素过宽
    #[test]
    fn test_margin_causes_overflow() {
        let container = square(0.0, 0.0, 50.0);
        let elements = vec![LayoutElement::with_margin(
            "wide",
            40.0,
            20.0,
            ElementMargin::horizontal(10.0),
        )];
        // footprint = 10 + 40 + 10 = 60 > 50 - 2*5 = 40 → 放不下
        let config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);

        let solution = layout_rows(&container, &elements, &config);
        assert!(!solution.is_fully_placed());
    }
}
