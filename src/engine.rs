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
use crate::rules::{HAlign, LayoutConfig};

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
/// y = padding_top
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
    let mut placed: Vec<PlacedElement> = Vec::new();
    let mut warnings: Vec<LayoutWarning> = Vec::new();
    let mut unplaced: Vec<String> = Vec::new();

    let mut y = config.padding_top;
    let mut idx: usize = 0;

    while idx < elements.len() {
        // 检查容器底部溢出
        if y >= max_y {
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
            y += 0.5;
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
            y += 0.5;
            continue;
        }

        // 4. 贪心塞入元素
        let (row_indices, new_idx, final_row_height) =
            pack_row_elements(elements, idx, interval_r - interval_l, config, &mut warnings);

        if row_indices.is_empty() {
            // 连第一个元素都放不下
            let elem = &elements[idx];
            if elem.constraints.shrinkable {
                if let Some(min_w) = elem.constraints.min_width {
                    if min_w > interval_r - interval_l {
                        warnings.push(LayoutWarning::ElementTooWide {
                            element_id: elem.id.clone(),
                            min_width: min_w,
                            max_available: interval_r - interval_l,
                        });
                        unplaced.push(elem.id.clone());
                        idx += 1;
                        continue;
                    }
                }
            } else {
                // 不可缩且放不下 → 跳过
                warnings.push(LayoutWarning::ElementTooWide {
                    element_id: elem.id.clone(),
                    min_width: elem.width,
                    max_available: interval_r - interval_l,
                });
                unplaced.push(elem.id.clone());
                idx += 1;
                continue;
            }
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
                    placed.push(PlacedElement {
                        id: elem.id.clone(),
                        x: resolved_x,
                        y,
                        width: resolved_width,
                        height: elem.height,
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
    _warnings: &mut Vec<LayoutWarning>,
) -> (Vec<usize>, usize, f64) {
    let mut row_indices: Vec<usize> = Vec::new();
    let mut used_width = 0.0;
    let mut row_height: f64 = 0.0;
    let mut idx = start_idx;

    while idx < elements.len() {
        let elem = &elements[idx];

        // 尝试首选宽度
        let preferred_w = elem.effective_width();
        let gap_needed = if row_indices.is_empty() {
            0.0
        } else {
            config.gap
        };

        let total_needed = used_width + gap_needed + preferred_w;

        if total_needed <= available_width + 1e-9 {
            // 放得下
            row_indices.push(idx);
            used_width = total_needed;
            row_height = row_height.max(elem.height);
            idx += 1;
        } else if elem.constraints.shrinkable {
            // 尝试缩到最小宽度
            let min_w = elem
                .constraints
                .min_width
                .unwrap_or(0.0)
                .max(0.0);
            let total_min = used_width + gap_needed + min_w;

            if total_min <= available_width + 1e-9 {
                row_indices.push(idx);
                used_width = total_min;
                row_height = row_height.max(elem.height);
                idx += 1;
            } else {
                // 哪怕最小宽度也放不下，这行到此为止
                break;
            }
        } else {
            // 不可缩且放不下 → 行到此为止，下一个元素开新行
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

    // ── 宽度约束 ──
    for i in 0..n {
        let elem = &elements[row_indices[i]];
        let preferred_w = elem.effective_width();

        // right_i - left_i == preferred_width (STRONG)
        solver
            .add_constraints(
                [(right_vars[i] - left_vars[i]) | EQ(Strength::STRONG) | preferred_w],
            )
            .map_err(|e| format!("add_constraint width==preferred failed: {e:?}"))?;

        // min_width 约束
        if let Some(min_w) = elem.constraints.min_width {
            if min_w > 0.0 {
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i]) | GE(Strength::REQUIRED) | min_w],
                    )
                    .map_err(|e| format!("add_constraint width>=min failed: {e:?}"))?;
            }
        }

        // max_width 约束
        if let Some(max_w) = elem.constraints.max_width {
            solver
                .add_constraints(
                    [(right_vars[i] - left_vars[i]) | LE(Strength::REQUIRED) | max_w],
                )
                .map_err(|e| format!("add_constraint width<=max failed: {e:?}"))?;
        }
    }

    // ── 间距约束：left_{i+1} >= right_i + gap ──
    if n > 1 && config.gap > 0.0 {
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
            // left_0 + right_last == interval_l + interval_r (等边距)
            let last = n - 1;
            let target = interval_l + interval_r;
            solver
                .add_constraints(
                    [(left_vars[0] + right_vars[last]) | EQ(Strength::STRONG) | target],
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
}
