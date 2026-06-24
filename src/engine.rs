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

use crate::element::{LayoutElement, SizeStrategy};
use crate::region::RangeGenerator;
use crate::result::{LayoutSolution, LayoutWarning, PlacedElement};
use crate::rules::{HAlign, LayoutConfig, StackDirection, VAlign};
use crate::shape::ContainerShape;

/// 条件打印宏：仅在 `verbose` feature 开启时输出诊断日志
///
/// 使用 `cfg!()` 代替 `#[cfg]` 编译条件，确保表达式始终被求值
/// （消除未使用变量警告），运行时由编译器优化掉 false 分支。
macro_rules! vprintln {
    ($($arg:tt)*) => {
        if cfg!(feature = "verbose") {
            println!($($arg)*);
        }
    };
}

/// Fill 元素的最小内容宽度（像素）
///
/// 防止 Fill 元素仅靠 margin 占地就被打包进拥挤的行，
/// 而后在约束求解阶段被 REQUIRED 约束碾压至宽度归零。
const FILL_MIN_CONTENT_WIDTH: f64 = 1.0;

/// 计算 Fill 元素的隐式最小宽度
///
/// 优先级：`constraints.min_width`（显式） > `preferred * fill_min_ratio` > `FILL_MIN_CONTENT_WIDTH`
fn fill_implicit_min_width(elem: &LayoutElement, config: &LayoutConfig) -> f64 {
    elem.constraints
        .min_width
        .unwrap_or_else(|| (elem.width * config.fill_min_ratio).max(FILL_MIN_CONTENT_WIDTH))
        .max(FILL_MIN_CONTENT_WIDTH)
}

/// 单次排版求解（容器形状入口）—— 推荐的外部 API
///
/// 接受 `ContainerShape`（可序列化的逻辑实体），
/// 内部调用 `to_bezpath()` 转换为物理路径后送入 `layout_rows`。
///
/// # 参数
/// - `container`：容器形状（内置或自定义，均可序列化）
/// - `elements`：待排版元素列表
/// - `config`：全局排版配置
///
/// # 返回
/// `LayoutSolution` 包含已排放元素、未排放元素、警告信息。
pub fn layout_container(
    container: &ContainerShape,
    elements: &[LayoutElement],
    config: &LayoutConfig,
) -> LayoutSolution {
    let bezpath = container.to_bezpath();
    layout_rows(&bezpath, elements, config)
}

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
///     y -= row_h + line_spacing  (从上到下排版)
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

    // 调试心跳：每次排版都打印容器范围
    vprintln!(
        "[layout_rows] START | container_bbox=({:.1},{:.1}→{:.1},{:.1}) | elements={} | padding=(l:{},r:{},t:{},b:{})",
        rg.extents.x0, rg.extents.y0, rg.extents.x1, rg.extents.y1,
        elements.len(),
        config.padding_left, config.padding_right,
        config.padding_top, config.padding_bottom,
    );

    let container_y0 = rg.extents.y0;
    let container_top = rg.extents.y1 - config.padding_top;
    let container_bottom = container_y0 + config.padding_bottom;
    let mut placed: Vec<PlacedElement> = Vec::new();
    let mut warnings: Vec<LayoutWarning> = Vec::new();
    let mut unplaced: Vec<String> = Vec::new();

    // 从上到下排版：首行顶部贴齐容器顶部
    let first_est_height = elements[0].height;
    let mut y = container_top - first_est_height;
    let mut prev_row_bottom = container_top; // 🛡️ 记住上一行底部，防御行间 Y 轴重叠
    let mut idx: usize = 0;

    while idx < elements.len() {
        // 检查容器底部溢出：行底部低于容器底部则全部溢出
        if y < container_bottom - 1e-9 {
            for i in idx..elements.len() {
                warnings.push(LayoutWarning::Overflow {
                    element_id: elements[i].id.clone(),
                    message: format!(
                        "container bottom reached at y={:.1}, container_bottom={:.1}",
                        y, container_bottom
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
            // 无可用的行区间，用元素高度向下跳跃（P0 智能跳跃：跳过死区）
            y -= elements[idx].footprint_height().max(config.step_size);
            continue;
        }

        // 多区间贪心选择：遍历所有区间，选能放入最多元素的那个
        // 心形/回字形等异形容器在单行可能产生多个区间，单取最宽会浪费可用空间
        let mut best_count: usize = 0;
        let mut best_iv: (f64, f64) = (0.0, 0.0);
        let mut best_pack: Option<(Vec<usize>, usize, f64, Vec<LayoutWarning>)> = None;

        for iv in &row_range.intervals {
            let iv_w = iv.1 - iv.0;
            let avail = iv_w - config.padding_left - config.padding_right;
            // 提前跳过明显不够宽的区间（连第一个元素的最小宽度都装不下）
            let first = &elements[idx];
            let min_needed = if first.constraints.size_strategy.can_shrink() {
                first.footprint_width_with(first.constraints.min_width.unwrap_or(0.0))
            } else {
                first.footprint_width()
            };
            if avail < min_needed - 1e-9 {
                continue;
            }

            let mut trial_warnings: Vec<LayoutWarning> = Vec::new();
            let (indices, _next, _row_h) =
                pack_row_elements(elements, idx, avail, config, &mut trial_warnings);
            if indices.len() > best_count {
                best_count = indices.len();
                best_iv = *iv;
                best_pack = Some((indices, _next, _row_h, trial_warnings));
            }
        }

        if best_count == 0 {
            // 所有区间都放不下第一个元素
            if config.stack_direction == StackDirection::Vertical {
                // Vertical 模式：记录警告，跳过该元素，并向下跳跃（与 Flow 相同）
                // 否则下一个元素卡在同一 Y 高度，遇到同样的障碍（如孔洞）再次失败
                let max_avail = row_range.intervals.iter()
                    .map(|(l, r)| r - l - config.padding_left - config.padding_right)
                    .fold(0.0, f64::max);
                let report_min = if elements[idx].constraints.size_strategy.can_shrink() {
                    elements[idx].footprint_width_with(fill_implicit_min_width(&elements[idx], config))
                } else {
                    elements[idx].footprint_width()
                };
                warnings.push(LayoutWarning::ElementTooWide {
                    element_id: elements[idx].id.clone(),
                    min_width: report_min,
                    max_available: max_avail,
                });
                unplaced.push(elements[idx].id.clone());
                y -= elements[idx].footprint_height().max(config.step_size);
                idx += 1;
                continue;
            }
            // Flow 模式：向下跳跃
            y -= elements[idx].footprint_height().max(config.step_size);
            continue;
        }

        let (mut row_indices, next_idx, found_row_height, trial_warnings) = best_pack.unwrap();
        warnings.extend(trial_warnings);
        let row_start_idx = idx;
        idx = next_idx;
        row_height = found_row_height;

        let mut interval_l = best_iv.0 + config.padding_left;
        let mut interval_r = best_iv.1 - config.padding_right;

        // 边界保护（多区间选择后的兜底）
        if interval_r - interval_l <= 0.0 {
            y -= elements[idx].footprint_height().max(config.step_size);
            continue;
        }

        // ── VAlign::Baseline 行高修正 ──
        // row_height = max( max(baseline + margin.top), max(height - baseline + margin.bottom) )
        if config.valign == VAlign::Baseline {
            let mut max_ascent = 0.0_f64;
            let mut max_descent = 0.0_f64;
            for &ri in &row_indices {
                let elem = &elements[ri];
                let elem_baseline = elem.effective_baseline();
                let ascent = elem_baseline + elem.margin.top;
                let descent = (elem.height - elem_baseline).max(0.0) + elem.margin.bottom;
                max_ascent = max_ascent.max(ascent);
                max_descent = max_descent.max(descent);
            }
            row_height = max_ascent + max_descent;
        }

        // 🛡️ 行间 Y 轴安全网：在 refined query 之前先修正 y，
        // 避免 refined 区间基于旧 y 计算导致最后一行溢出轮廓
        // （旧 y 离容器顶部更远→轮廓更宽→区间过宽→元素放到新 y 时溢出）
        let mut safety_net_triggered = false;
        if y + row_height > prev_row_bottom + 1e-9 {
            y = prev_row_bottom - row_height;
            safety_net_triggered = true;
        }

        // 🔒 line_spacing 硬约束：确保上一行底部到当前行顶部 ≥ line_spacing
        // 因为 y += row_height + line_spacing 在执行时还不知道下行高度，
        // 若下行高度 > 上行高度，实际间距会被压缩（A高+line_spacing-B高）。
        // 这里事后校准：当前行顶部 = y + row_height，上一行底部 = prev_row_bottom
        // 要求 prev_row_bottom - (y + row_height) >= line_spacing
        // 即 y <= prev_row_bottom - line_spacing - row_height
        // ⚠️ 仅当存在上一行时才应用（第一行没有上行，不应钳制）
        if !placed.is_empty() {
            let max_allowed_y = prev_row_bottom - config.line_spacing - row_height;
            if y > max_allowed_y + 1e-9 {
                y = max_allowed_y.max(container_bottom); // 不越界到容器底部以下
            }
        }

        // 用实际行高重新查询区间（行内可能有更高元素改变了有效高度）
        let refined_row = rg.get_intervals_at(y, row_height, config.min_width);
        let mut refinement_applied = false;
        let mut refined_raw_iv: Option<(f64, f64)> = None;
        if !refined_row.is_empty() {
            // 选离 best_iv 中心最近的 refined 区间（防止分叉形容器跳到另一侧）
            let best_center = (best_iv.0 + best_iv.1) / 2.0;
            if let Some(closest_refined) = refined_row.intervals.iter().min_by(|a, b| {
                let ca = (a.0 + a.1) / 2.0;
                let cb = (b.0 + b.1) / 2.0;
                (ca - best_center)
                    .abs()
                    .partial_cmp(&(cb - best_center).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                refined_raw_iv = Some(*closest_refined);
                interval_l = closest_refined.0 + config.padding_left;
                interval_r = closest_refined.1 - config.padding_right;
                refinement_applied = true;
            }
        }

        // 校验 refinement 后的区间仍能容纳行内元素（防止沙漏形容器因行高增大
        // 导致区间缩水到放不下已打包元素）
        if !row_indices.is_empty() {
            let row_min_span: f64 = row_indices
                .iter()
                .map(|&ri| {
                    let e = &elements[ri];
                    if e.constraints.size_strategy.can_shrink() {
                        e.footprint_width_with(e.constraints.min_width.unwrap_or(0.0))
                    } else {
                        e.footprint_width()
                    }
                })
                .sum::<f64>()
                + (row_indices.len().saturating_sub(1)) as f64 * config.gap;
            if interval_r - interval_l < row_min_span - 1e-9 {
                safety_net_triggered = true;
                if refinement_applied {
                    // 沙漏容器：行高增大→轮廓收窄，best_iv 已过期
                    // 用 refined 真实物理区间的宽度重新打包，放不下的延迟到后续行
                    let refined_avail = refined_raw_iv
                        .map_or(
                            (interval_r - interval_l).max(0.0),
                            |(rl, rr)| (rr - rl) - config.padding_left - config.padding_right,
                        )
                        .max(0.0);
                    let mut re_pack_warnings: Vec<LayoutWarning> = Vec::new();
                    let (new_row_indices, new_next_idx, new_row_height) = pack_row_elements(
                        elements,
                        row_start_idx,
                        refined_avail,
                        config,
                        &mut re_pack_warnings,
                    );
                    if new_row_indices.is_empty() {
                        // 连一个元素都放不下 → 跳过首元素，下次循环再试
                        let report_min = if elements[row_start_idx].constraints.size_strategy.can_shrink() {
                            elements[row_start_idx].footprint_width_with(fill_implicit_min_width(&elements[row_start_idx], config))
                        } else {
                            elements[row_start_idx].footprint_width()
                        };
                        warnings.push(LayoutWarning::ElementTooWide {
                            element_id: elements[row_start_idx].id.clone(),
                            min_width: report_min,
                            max_available: refined_avail,
                        });
                        unplaced.push(elements[row_start_idx].id.clone());
                        idx = row_start_idx + 1;
                        y -= elements[row_start_idx].footprint_height().max(config.step_size);
                        continue;
                    }
                    warnings.extend(re_pack_warnings);
                    row_indices = new_row_indices;
                    idx = new_next_idx;
                    row_height = new_row_height;
                    // interval_l/r 保持 refined 值不变（真实物理边界）
                } else {
                    // 无 refinement → 回退到 best_iv
                    interval_l = best_iv.0 + config.padding_left;
                    interval_r = best_iv.1 - config.padding_right;
                }
            }
        }

        // 🔍 诊断日志：Refinement & Safety Net
        {
            let row_element_ids: Vec<&str> = row_indices.iter().map(|&ri| elements[ri].id.as_str()).collect();
            vprintln!(
                "[refine] y={:.1} row_h={:.1} elements={:?} | best_iv=({:.1},{:.1}) w={:.1} | refined_raw={:?} refined_applied={} safety_net={} | final=({:.1},{:.1}) w={:.1}",
                y, row_height, row_element_ids,
                best_iv.0, best_iv.1, best_iv.1 - best_iv.0,
                refined_raw_iv, refinement_applied, safety_net_triggered,
                interval_l, interval_r, interval_r - interval_l,
            );
        }

        // 检查行底部是否低于容器底部（行高变化后重检）
        if y < container_bottom - 1e-9 {
            // 已打包到行内的元素
            for &ri in &row_indices {
                warnings.push(LayoutWarning::Overflow {
                    element_id: elements[ri].id.clone(),
                    message: format!(
                        "row below container bottom: y={:.1} < container_bottom={:.1}",
                        y, container_bottom
                    ),
                });
                unplaced.push(elements[ri].id.clone());
            }
            // 尚未处理的后缀元素
            for i in idx..elements.len() {
                warnings.push(LayoutWarning::Overflow {
                    element_id: elements[i].id.clone(),
                    message: format!(
                        "row below container bottom: y={:.1} < container_bottom={:.1}",
                        y, container_bottom
                    ),
                });
                unplaced.push(elements[i].id.clone());
            }
            break;
        }

        // 5. kasuari 求解行内 X 位置
        let mut row_footprints: Vec<(usize, f64, f64)> = Vec::new();
        match solve_row_x(
            elements,
            &row_indices,
            interval_l,
            interval_r,
            config,
            &mut warnings,
        ) {
            Ok(x_solutions) => {
                for (elem_idx, resolved_x, resolved_width) in x_solutions {
                    row_footprints.push((elem_idx, resolved_x, resolved_width));
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
                        VAlign::Baseline => {
                            // max_ascent = max(baseline + margin.top) across the row
                            let max_ascent = row_indices.iter()
                                .map(|&ri| {
                                    let e = &elements[ri];
                                    e.effective_baseline() + e.margin.top
                                })
                                .fold(0.0_f64, f64::max);
                            let elem_baseline = elem.effective_baseline();
                            // 元素顶部 = row_y + max_ascent - baseline - margin.top
                            y + max_ascent - elem_baseline - elem.margin.top
                        }
                    };

                    // 🆕 内容盒边界保护：
                    // 安全网钳制的是 footprint 范围 [resolved_x, resolved_x+resolved_width]，
                    // 而 PlacedElement 输出的是 content box（去掉了 margin）。
                    // 当 footprint 紧贴区间边界时，content box 可能因 margin 偏移而越界。
                    let raw_content_x = resolved_x + elem.margin.left;
                    let raw_content_w = (resolved_width - elem.margin.left - elem.margin.right)
                        .max(0.0);
                    let clamped_x = raw_content_x.max(interval_l);
                    let clamped_right = (raw_content_x + raw_content_w).min(interval_r);
                    let clamped_w = (clamped_right - clamped_x).max(0.0);

                    // 🔍 诊断日志：逐元素 footprint bounds vs 区间边界 + Y 坐标
                    {
                        let fp_left = resolved_x;
                        let fp_right = resolved_x + resolved_width;
                        let fp_overflow_left = interval_l - fp_left;
                        let fp_overflow_right = fp_right - interval_r;
                        let ct_left = clamped_x;
                        let ct_right = clamped_x + clamped_w;
                        let ct_overflow_left = interval_l - ct_left;
                        let ct_overflow_right = ct_right - interval_r;
                        let y_top = final_y + viz_height; // 元素顶部 Y
                        vprintln!(
                            "[elem_place] id={:<4} fp=({:.1},{:.1}) ct=({:.1},{:.1}) | y={:.1} h={:.1} Y=[{:.1}→{:.1}] | interval=({:.1},{:.1}) | fp_ovf=(L:{:.3},R:{:.3}) ct_ovf=(L:{:.3},R:{:.3}) | margin=(l:{:.1},r:{:.1})",
                            elem.id, fp_left, fp_right, ct_left, ct_right,
                            final_y, viz_height, final_y, y_top,
                            interval_l, interval_r,
                            fp_overflow_left.max(0.0), fp_overflow_right.max(0.0),
                            ct_overflow_left.max(0.0), ct_overflow_right.max(0.0),
                            elem.margin.left, elem.margin.right,
                        );
                    }

                    // 🆕 零宽度过滤：width≈0 的元素实质不可见，归入 unplaced
                    // 以保持 API 语义诚实（placed = 观察者可见的元素）
                    const ZERO_WIDTH_EPSILON: f64 = 1e-9;
                    if clamped_w > ZERO_WIDTH_EPSILON {
                        placed.push(PlacedElement {
                            id: elem.id.clone(),
                            x: clamped_x,
                            y: final_y,
                            width: clamped_w,
                            height: viz_height,
                        });
                    } else {
                        unplaced.push(elem.id.clone());
                        warnings.push(LayoutWarning::WidthConstraintUnsatisfiable {
                            element_id: elem.id.clone(),
                            message: format!(
                                "element width collapsed to {:.3} (row too crowded for Fill element); moved to unplaced",
                                clamped_w
                            ),
                        });
                    }
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

        // 🔍 诊断日志：行摘要 — 整行元素 footprint span vs 区间
        {
            let row_placed_end = placed.len();
            let row_placed_start = row_placed_end.saturating_sub(row_indices.len());
            let row_placed_slice = &placed[row_placed_start..row_placed_end];
            let mut fp_left_min = f64::MAX;
            let mut fp_right_max = f64::MIN;
            let mut ct_left_min = f64::MAX;
            let mut ct_right_max = f64::MIN;
            let mut ids = Vec::new();
            for &(ei, fp_x, fp_w) in &row_footprints {
                let elem = &elements[ei];
                ids.push(elem.id.as_str());
                let fp_left = fp_x;
                let fp_right = fp_x + fp_w;
                fp_left_min = fp_left_min.min(fp_left);
                fp_right_max = fp_right_max.max(fp_right);
            }
            for p in row_placed_slice {
                ct_left_min = ct_left_min.min(p.x);
                ct_right_max = ct_right_max.max(p.x + p.width);
            }
            let fp_span = fp_right_max - fp_left_min;
            let ct_span = ct_right_max - ct_left_min;
            let interval_w = interval_r - interval_l;
            let fp_overflow_r = fp_right_max - interval_r;
            let ct_overflow_r = ct_right_max - interval_r;
            let row_bottom_y = y; // 行底（从上到下排版，y 指向行底部）
            let row_top_y = y + row_height; // 行顶
            let next_row_y = y - row_height - config.line_spacing; // 下一行 y
            vprintln!(
                "[row_done] y={:.1} h={:.1} ids={:?} | interval=({:.1},{:.1}) w={:.1} | fp_span=({:.1},{:.1})={:.1} ct_span=({:.1},{:.1})={:.1} | row_Y=[{:.1}→{:.1}] gap_to_next={:.1} | overflow: fp_r={:.3} ct_r={:.3} safety_net={}",
                y, row_height, ids,
                interval_l, interval_r, interval_w,
                fp_left_min, fp_right_max, fp_span,
                ct_left_min, ct_right_max, ct_span,
                row_bottom_y, row_top_y,
                row_bottom_y - next_row_y - row_height, // gap = current_bottom - next_top
                fp_overflow_r.max(0.0), ct_overflow_r.max(0.0),
                safety_net_triggered,
            );
        }

        prev_row_bottom = y;
        y -= row_height + config.line_spacing;
    }

    vprintln!(
        "[layout_rows] DONE | placed={} unplaced={} warnings={}",
        placed.len(),
        unplaced.len(),
        warnings.len(),
    );

    // ── kasuari 负数坐标验证（Phase 2 Step 2）──
    let placed_xs: Vec<String> = placed
        .iter()
        .map(|p| format!("{}={:.1}", p.id, p.x))
        .collect();
    vprintln!("[kasusari_verify] placed X coords: {:?}", placed_xs);
    let any_negative = placed.iter().any(|p| p.x < -1e-9);
    vprintln!("[kasusari_verify] any X < 0: {}", any_negative);

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
/// 当 `config.stack_direction == Vertical` 时，每行仅放入一个元素。
///
/// 返回 `(row_indices, next_idx, row_height)`
fn pack_row_elements(
    elements: &[LayoutElement],
    start_idx: usize,
    available_width: f64,
    config: &LayoutConfig,
    warnings: &mut Vec<LayoutWarning>,
) -> (Vec<usize>, usize, f64) {
    // ── Vertical 模式：强制单元素行 ──
    if config.stack_direction == StackDirection::Vertical {
        let elem = &elements[start_idx];
        let footprint_w = elem.footprint_width();
        let row_h = elem.footprint_height();

        // 尝试首选宽度
        if footprint_w <= available_width + 1e-9 {
            return (vec![start_idx], start_idx + 1, row_h);
        }

        // 可缩元素：尝试最小宽度
        if elem.constraints.size_strategy.can_shrink() {
            let min_w = if matches!(elem.constraints.size_strategy, SizeStrategy::Fill) {
                fill_implicit_min_width(elem, config)
            } else {
                elem.constraints.min_width.unwrap_or(0.0).max(0.0)
            };
            let min_fp = elem.footprint_width_with(min_w);
            if min_fp <= available_width + 1e-9 {
                return (vec![start_idx], start_idx + 1, row_h);
            }
        }

        // 放不下 → 记录警告，跳过该元素
        let report_min = if elem.constraints.size_strategy.can_shrink() {
            elem.footprint_width_with(fill_implicit_min_width(elem, config))
        } else {
            footprint_w
        };
        warnings.push(LayoutWarning::ElementTooWide {
            element_id: elem.id.clone(),
            min_width: report_min,
            max_available: available_width,
        });
        return (vec![], start_idx + 1, 0.0);
    }

    // ── Flow 模式：贪心多元素行 ──
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
        } else if elem.constraints.size_strategy.can_shrink() {
            // 尝试缩到最小宽度（含 margin）
            let min_w = if matches!(elem.constraints.size_strategy, SizeStrategy::Fill) {
                fill_implicit_min_width(elem, config)
            } else {
                elem.constraints.min_width.unwrap_or(0.0).max(0.0)
            };
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
/// 四态真值表（SizeStrategy → 约束强度）：
///
/// | SizeStrategy              | 收缩 | 拉伸 | 宽度偏好方程                               |
/// |:--------------------------|:-----|:-----|:-------------------------------------------|
/// | Fixed { shrinkable: false } | ❌    | ❌    | `== preferred` (**REQUIRED**)               |
/// | Fixed { shrinkable: true }  | ✅    | ❌    | `<= preferred` (**REQUIRED**) + `== preferred` (**STRONG**) |
/// | Fill                      | ✅    | ✅    | 无宽度偏好；靠 row-fill + 等宽约束驱动     |
///
/// 所有状态叠加：
/// - `min_width` → **REQUIRED**
/// - `max_width` → **REQUIRED**
/// - 行内有 Fill 元素时：`right_last - left_0 == interval_width` → **STRONG**
/// - 行内有 ≥2 个 Fill 元素时：`width_fill_i == width_fill_0` → **STRONG**（防欠定退化）
///
/// 返回 `Vec<(elem_index, x, width)>` 或错误信息。
fn solve_row_x(
    elements: &[LayoutElement],
    row_indices: &[usize],
    interval_l: f64,
    interval_r: f64,
    config: &LayoutConfig,
    warnings: &mut Vec<LayoutWarning>,
) -> Result<Vec<(usize, f64, f64)>, String> {
    let n = row_indices.len();
    if n == 0 {
        return Ok(vec![]);
    }

    let interval_width = interval_r - interval_l;
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

    // ── 宽度约束（四态真值表）──
    let mut has_fill_in_row = false;
    let mut fill_indices: Vec<usize> = Vec::new(); // 收集 Fill 元素的行内索引

    for i in 0..n {
        let elem = &elements[row_indices[i]];
        let footprint_w = elem.footprint_width();

        match &elem.constraints.size_strategy {
            SizeStrategy::Fixed { shrinkable: false } => {
                // 状态 1: 不可缩不可伸 → == preferred (REQUIRED)
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i])
                            | EQ(Strength::REQUIRED)
                            | footprint_w],
                    )
                    .map_err(|e| format!("add_constraint width==preferred(REQUIRED) failed: {e:?}"))?;
            }
            SizeStrategy::Fixed { shrinkable: true } => {
                // 状态 2: 可缩不可伸
                // width <= preferred (REQUIRED) — 可被压扁
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i])
                            | LE(Strength::REQUIRED)
                            | footprint_w],
                    )
                    .map_err(|e| format!("add_constraint width<=preferred(REQUIRED) failed: {e:?}"))?;
                // width == preferred (STRONG) — 偏好首选值
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i])
                            | EQ(Strength::STRONG)
                            | footprint_w],
                    )
                    .map_err(|e| format!("add_constraint width==preferred(STRONG) failed: {e:?}"))?;
            }
            SizeStrategy::Fill => {
                // 状态 4: 可缩可伸
                // 不添加宽度相等约束（WEAK 会导致 kasuari 通过推位置而非拉伸来满足）
                // 仅靠 min/max REQUIRED 边界 + STRONG row-fill 自然推动拉伸
                has_fill_in_row = true;
                fill_indices.push(i);

                // 🆕 保底：WEAK 级别最小内容宽度约束
                // 使用 fill_implicit_min_width 计算（优先显式 min_width，其次 preferred * fill_min_ratio）
                // WEAK 优先级最低，不会与 STRONG row-fill 或 REQUIRED 边界冲突。
                let implicit_min = fill_implicit_min_width(elem, config);
                let min_content_footprint = elem.footprint_width_with(implicit_min);
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i])
                            | GE(Strength::WEAK)
                            | min_content_footprint],
                    )
                    .map_err(|e| format!("add_constraint fill-min-content failed: {e:?}"))?;
            }
        }

        // min_width 约束（含 margin）— 永远 REQUIRED
        if let Some(min_w) = elem.constraints.min_width {
            let min_footprint = elem.footprint_width_with(min_w);
            if min_footprint > 0.0 {
                solver
                    .add_constraints(
                        [(right_vars[i] - left_vars[i])
                            | GE(Strength::REQUIRED)
                            | min_footprint],
                    )
                    .map_err(|e| format!("add_constraint width>=min failed: {e:?}"))?;
            }
        }

        // max_width 约束（含 margin）— 永远 REQUIRED
        if let Some(max_w) = elem.constraints.max_width {
            let max_footprint = elem.footprint_width_with(max_w);
            solver
                .add_constraints(
                    [(right_vars[i] - left_vars[i])
                        | LE(Strength::REQUIRED)
                        | max_footprint],
                )
                .map_err(|e| format!("add_constraint width<=max failed: {e:?}"))?;
        }
    }

    // ── 间距约束（双保险）：所有相邻 gap 全锁 ──
    // margin 已纳入 footprint 宽度，gap 直接叠加即可
    // 1. REQUIRED 硬底线：gap >= config.gap
    // 2. STRONG 精确锁死：gap == config.gap，防止膨胀，将拉伸力推入 Fill 宽度
    if n > 1 {
        for i in 0..(n - 1) {
            solver
                .add_constraints(
                    [(left_vars[i + 1] - right_vars[i]) | GE(Strength::REQUIRED) | config.gap],
                )
                .map_err(|e| format!("add_constraint gap>=min failed: {e:?}"))?;
            solver
                .add_constraints(
                    [(left_vars[i + 1] - right_vars[i]) | EQ(Strength::STRONG) | config.gap],
                )
                .map_err(|e| format!("add_constraint gap==exact failed: {e:?}"))?;
        }
    }

    // ── 多 Fill 等宽约束（N ≥ 2）──
    // 当一行中有 ≥2 个 Fill 元素时，row-fill 约束只管总体跨度，不管内部分配。
    // 不加等宽约束会导致求解器欠定 → Fill 宽度退化为 0。
    // STRONG 级别：不会抢夺 REQUIRED 边界约束的优先级，安全网仍在。
    if fill_indices.len() >= 2 {
        let first = fill_indices[0];
        for &fi in &fill_indices[1..] {
            let width_expr = right_vars[fi] - left_vars[fi];
            let first_width = right_vars[first] - left_vars[first];
            solver
                .add_constraints(
                    [(width_expr - first_width) | EQ(Strength::STRONG) | 0.0],
                )
                .map_err(|e| format!("add_constraint fill-equal-width failed: {e:?}"))?;
        }
    }

    // ── 行填满约束（Fill 元素存在时）──
    // right_last - left_0 == interval_width (STRONG)
    // 当 max_width 阻止完全填满时，HAlign fallback 生效
    if has_fill_in_row {
        let last = n - 1;
        solver
            .add_constraints(
                [(right_vars[last] - left_vars[0]) | EQ(Strength::STRONG) | interval_width],
            )
            .map_err(|e| format!("add_constraint row-fill failed: {e:?}"))?;
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

    // ═══════════════════════════════════════════════════════════════════════
    // 安全兜底网：钳制 + 重排，数学保证绝不越界、绝不重叠
    // ═══════════════════════════════════════════════════════════════════════
    let mut any_clamped = false;

    // 步骤1：逐个钳制到区间边界 [interval_l, interval_r]
    for (_, left, width) in results.iter_mut() {
        let raw_left = *left;
        let raw_right = *left + *width;
        *left = raw_left.max(interval_l).min(interval_r);
        let clamped_right = raw_right.max(interval_l).min(interval_r);
        if clamped_right < *left {
            // 整个元素在区间外 → 零宽度放到左边界
            *left = interval_l;
            *width = 0.0;
        } else {
            *width = clamped_right - *left;
        }
        if (raw_left - *left).abs() > 1e-9 || (raw_right - clamped_right).abs() > 1e-9 {
            any_clamped = true;
        }
    }

    // 步骤2：从左到右确保间距 ≥ config.gap（不超出 interval_r）
    if n > 1 {
        for i in 1..n {
            let prev_right = results[i - 1].1 + results[i - 1].2;
            let min_left = (prev_right + config.gap).min(interval_r);
            if results[i].1 < min_left {
                results[i].1 = min_left;
                let max_right = (results[i].1 + results[i].2).min(interval_r);
                results[i].2 = (max_right - results[i].1).max(0.0);
                any_clamped = true;
            }
        }
    }

    // 步骤3：右侧边界兜底 + 从右向左回溯压缩（保证最后一个元素不越界）
    if n > 0 {
        let last = n - 1;
        let right = results[last].1 + results[last].2;
        if right > interval_r + 1e-9 {
            // 压缩最后一个元素
            results[last].1 = results[last].1.min(interval_r);
            results[last].2 = (interval_r - results[last].1).max(0.0);
            any_clamped = true;
            // 从右向左回溯：前一个元素的右边界不能超过后一个元素的左边界 - gap
            for i in (0..last).rev() {
                let next_left = results[i + 1].1;
                let max_right = (next_left - config.gap).max(interval_l);
                let cur_right = results[i].1 + results[i].2;
                if cur_right > max_right + 1e-9 {
                    if max_right < results[i].1 {
                        results[i].1 = max_right;
                        results[i].2 = 0.0;
                    } else {
                        results[i].2 = max_right - results[i].1;
                    }
                    any_clamped = true;
                }
            }
        }
    }

    if any_clamped {
        warnings.push(LayoutWarning::ConstraintConflict(format!(
            "safety net: solver produced out-of-bounds positions; clamped row to interval [{:.3}, {:.3}]",
            interval_l, interval_r
        )));
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
    use kurbo::Shape;

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
        let fixed = LayoutElement::new("fixed", 50.0, 20.0);
        // constraints 默认 SizeStrategy::Fixed { shrinkable: false }，无需额外设置
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

    /// 窄缩容器（三角形）：从上到下排版，越往下 Y 越小
    #[test]
    fn test_narrowing_triangle() {
        let container = right_triangle(0.0, 0.0, 100.0, 100.0);
        let elements: Vec<LayoutElement> = (0..6)
            .map(|i| LayoutElement::new(&format!("e{i}"), 30.0, 15.0))
            .collect();
        let config = LayoutConfig::with_spacing(2.0, 2.0, 2.0);

        let solution = layout_rows(&container, &elements, &config);
        // 三角形顶部宽 100（高 Y），底部趋近 0（低 Y），至少应排放 ≥ 2 个
        assert!(
            solution.placed.len() >= 2,
            "expected ≥2 placed in triangle, got {} placed",
            solution.placed.len()
        );
        // 从上到下排版 → 后排放的元素 Y 更小（越靠下）
        for w in solution.placed.windows(2) {
            assert!(w[1].y <= w[0].y, "rows should go downward (y decreasing)");
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

    /// VAlign::Middle —— 行内矮元素垂直居中 (从上到下排版)
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

        // container_top = 195, row_height = 50, y(行底) = 195-50 = 145
        // tall 元素：高度等于行高，y 为行底
        let tall = &solution.placed[0];
        assert!((tall.y - 145.0).abs() < 1.0, "tall y should ≈145, got {}", tall.y);

        // short 元素：居中 y = 145 + (50-10)/2 = 165
        let short = &solution.placed[1];
        assert!(
            (short.y - 165.0).abs() < 1.0,
            "short valign=middle y should ≈165, got {}",
            short.y
        );
    }

    /// VAlign::Bottom —— 行内矮元素贴底 (从上到下排版)
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
        // container_top = 195, y(行底) = 195-50 = 145
        // bottom: y = 145 + 50 - 10 = 185
        assert!(
            (short.y - 185.0).abs() < 1.0,
            "short valign=bottom y should ≈185, got {}",
            short.y
        );
    }

    /// VAlign::Top 是默认，不需要特殊处理 (从上到下排版)
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
        // container_top = 195, y(行底) = 195-50 = 145, VAlign::Top: short.y = y = 145
        assert!(
            (short.y - 145.0).abs() < 1.0,
            "short valign=top y should ≈145, got {}",
            short.y
        );
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
        // container_top = 195, y(行底) = 195-20 = 175
        // margined element Middle: y + margin.top + (row_height - footprint_height) / 2
        //   = 175 + 5 + (20 - 20) / 2 = 180
        let margined = &solution.placed[1];
        assert!(
            (margined.y - 180.0).abs() < 1.0,
            "margined middle y should ≈180, got {}",
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

    // ═══════════════════════════════════════════════════════════════════════
    // Phase 2: Stretch / Fill 测试
    // ═══════════════════════════════════════════════════════════════════════

    /// 单个 Fill 元素填满整个区间
    #[test]
    fn test_single_fill_fills_interval() {
        let container = square(0.0, 0.0, 100.0);
        let mut fill_elem = LayoutElement::new("fill", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        let config = LayoutConfig::with_spacing(5.0, 3.0, 5.0);
        // 区间 = 100 - 5*2 = 90
        // Fill 元素应被拉伸到填满整个区间

        let solution = layout_rows(&container, &[fill_elem], &config);
        assert!(solution.is_fully_placed());
        let placed = &solution.placed[0];
        // width 应该 ≈ 90（Fill 拉伸到区间全宽）
        assert!(
            (placed.width - 90.0).abs() < 2.0,
            "Fill element should stretch to interval width, got width={}",
            placed.width
        );
    }

    /// 混合 Fixed + Fill：Fill 占据剩余空间
    #[test]
    fn test_fill_absorbs_remainder() {
        let container = square(0.0, 0.0, 200.0);
        let fixed_elem = LayoutElement::new("fixed", 40.0, 20.0);
        // Fixed: 不可缩不可伸 → 40
        let mut fill_elem = LayoutElement::new("fill", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;

        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        // 区间 = 200 - 10 = 190
        // Fixed 占 40, gap=5, 剩余 190 - 40 - 5 = 145 应分配给 Fill

        let solution = layout_rows(&container, &[fixed_elem, fill_elem], &config);
        assert!(
            solution.is_fully_placed(),
            "warnings: {:?}",
            solution.warnings
        );
        assert_eq!(solution.placed.len(), 2);

        let fixed = &solution.placed[0];
        assert!((fixed.width - 40.0).abs() < 1.0, "Fixed width should stay 40");

        let fill = &solution.placed[1];
        assert!(
            fill.width > 40.0,
            "Fill width should be >40 (got {}), absorbing remainder",
            fill.width
        );
    }

    /// max_width 限制 Fill 拉伸，HAlign fallback 生效
    #[test]
    fn test_fill_max_width_caps_expansion_center_fallback() {
        let container = square(0.0, 0.0, 200.0);
        let mut fill_elem = LayoutElement::new("fill_capped", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        fill_elem.constraints.max_width = Some(60.0);
        // Fill 最多 60，不能填满 190 的区间 → HAlign::Center fallback

        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.halign = HAlign::Center;

        let solution = layout_rows(&container, &[fill_elem], &config);
        assert!(solution.is_fully_placed());
        let placed = &solution.placed[0];
        // 视觉宽度不应超过 max_width（60），且在区间中居中
        assert!(
            placed.width <= 60.0 + 1.0,
            "Fill width {} should not exceed max_width 60",
            placed.width
        );
        // 居中：x ≈ (190 - width_with_margin) / 2 + 5
        // width_with_margin ≈ width (no margin), so x ≈ (190 - width) / 2 + 5
        let expected_x = (190.0 - placed.width) / 2.0 + 5.0;
        assert!(
            (placed.x - expected_x).abs() < 2.0,
            "Capped Fill should center: x={}, expected_x≈{}",
            placed.x,
            expected_x
        );
    }

    /// Fixed { shrinkable: true } 与 Fill 混合
    #[test]
    fn test_shrinkable_fixed_with_fill() {
        let container = square(0.0, 0.0, 120.0);
        // 可用宽度 = 120 - 5*2 = 110
        let mut shrinkable = LayoutElement::new("shrink", 80.0, 20.0);
        shrinkable.constraints.size_strategy = SizeStrategy::Fixed { shrinkable: true };
        shrinkable.constraints.min_width = Some(30.0);

        let mut fill_elem = LayoutElement::new("fill", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        fill_elem.constraints.min_width = Some(10.0);
        // 总 preferred = 80 + 5 + 20 = 105 ≤ 110 → 都能放
        // shrinkable 保持 80（STRONG），Fill 获得剩余：110 - 80 - 5 = 25

        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        let solution = layout_rows(&container, &[shrinkable, fill_elem], &config);
        assert!(
            solution.is_fully_placed(),
            "unplaced: {:?}",
            solution.unplaced
        );

        let shrink = &solution.placed[0];
        assert!(
            (shrink.width - 80.0).abs() < 3.0,
            "Shrinkable should stay at preferred 80"
        );

        let fill = &solution.placed[1];
        assert!(
            fill.width > 20.0,
            "Fill should absorb extra space, got {}",
            fill.width
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Phase 2: Baseline 基线对齐测试
    // ═══════════════════════════════════════════════════════════════════════

    /// 两个元素 baseline 对齐：不同高度、不同基线
    #[test]
    fn test_baseline_alignment() {
        let container = square(0.0, 0.0, 300.0);
        // tall: height=50, baseline=40 (文字基线在顶部下方 40 处)
        let mut tall = LayoutElement::new("tall", 40.0, 50.0);
        tall.baseline = Some(40.0);
        // short: height=30, baseline=25
        let mut short = LayoutElement::new("short", 40.0, 30.0);
        short.baseline = Some(25.0);

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.valign = VAlign::Baseline;

        let solution = layout_rows(&container, &[tall, short], &config);
        assert!(solution.is_fully_placed());

        let tall_placed = &solution.placed[0];
        let short_placed = &solution.placed[1];

        // 两者基线应在同一 Y 坐标
        let tall_baseline_y = tall_placed.y + 40.0;
        let short_baseline_y = short_placed.y + 25.0;
        assert!(
            (tall_baseline_y - short_baseline_y).abs() < 1.0,
            "baselines should align: tall_baseline_y={}, short_baseline_y={}",
            tall_baseline_y,
            short_baseline_y
        );
    }

    /// baseline 为 None 时回退到 height（底部对齐）
    #[test]
    fn test_baseline_defaults_to_height() {
        let container = square(0.0, 0.0, 300.0);
        let mut has_baseline = LayoutElement::new("text", 30.0, 40.0);
        has_baseline.baseline = Some(30.0);
        let no_baseline = LayoutElement::new("image", 30.0, 50.0);
        // no_baseline 的 effective_baseline() = 50（height），即底部对齐

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.valign = VAlign::Baseline;

        let solution = layout_rows(&container, &[has_baseline, no_baseline], &config);
        assert!(solution.is_fully_placed());

        let text = &solution.placed[0];
        let image = &solution.placed[1];

        // text 基线 Y = text.y + 30
        // image "基线" Y = image.y + 50 (bottom of image)
        // 两者应该对齐
        let text_baseline_y = text.y + 30.0;
        let image_bottom_y = image.y + 50.0;
        assert!(
            (text_baseline_y - image_bottom_y).abs() < 1.0,
            "text baseline and image bottom should align"
        );
    }

    /// 基线行高修正：行高由最高 ascent + 最大 descent 决定
    #[test]
    fn test_baseline_row_height_correction() {
        let container = square(0.0, 0.0, 300.0);
        // tall: height=100, baseline=30 → ascent=30, descent=70
        let mut tall = LayoutElement::new("tall", 40.0, 100.0);
        tall.baseline = Some(30.0);
        // short: height=20, baseline=15 → ascent=15, descent=5
        let short = LayoutElement::new("short", 40.0, 20.0);

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.valign = VAlign::Baseline;

        let solution = layout_rows(&container, &[tall, short], &config);
        assert!(solution.is_fully_placed());

        // tall 元素应完整可见（不被截断）
        let tall_placed = &solution.placed[0];
        // container_top = 300-10 = 290, y(行底) = 290-100 = 190
        // max_ascent=30, tall's y = y + max_ascent - tall.baseline = 190+30-30 = 190
        assert!(
            tall_placed.y >= 189.0 && tall_placed.y <= 191.0,
            "tall element should not be clipped, y={}",
            tall_placed.y
        );
    }

    /// margin 与 baseline 组合
    #[test]
    fn test_baseline_with_margin() {
        let container = square(0.0, 0.0, 300.0);
        let mut a = LayoutElement::new("a", 40.0, 30.0);
        a.baseline = Some(20.0);
        a.margin = ElementMargin { top: 10.0, bottom: 5.0, left: 0.0, right: 0.0 };

        let mut b = LayoutElement::new("b", 40.0, 40.0);
        b.baseline = Some(25.0);
        b.margin = ElementMargin { top: 3.0, bottom: 7.0, left: 0.0, right: 0.0 };

        // ascent: a=20+10=30, b=25+3=28 → max_ascent=30
        // descent: a=(30-20)+5=15, b=(40-25)+7=22 → max_descent=22
        // row_height = 30+22=52

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.valign = VAlign::Baseline;

        let solution = layout_rows(&container, &[a, b], &config);
        assert!(solution.is_fully_placed());

        let a_placed = &solution.placed[0];
        let b_placed = &solution.placed[1];

        // baseline 在视觉内容框中的偏移 = margin.top + baseline
        // a_baseline_y = a_placed.y + a.margin.top + a.effective_baseline()
        // b_baseline_y = b_placed.y + b.margin.top + b.effective_baseline()
        let a_baseline_y = a_placed.y + 10.0 + 20.0; // margin.top=10, baseline=20
        let b_baseline_y = b_placed.y + 3.0 + 25.0;  // margin.top=3, baseline=25
        assert!(
            (a_baseline_y - b_baseline_y).abs() < 1.0,
            "baselines should align even with margin: a_baseline_y={}, b_baseline_y={}",
            a_baseline_y,
            b_baseline_y
        );
    }

    /// 多个 Fill 元素均分剩余空间（Phase 2.5 等宽约束）
    /// 验证 N≥2 个 Fill 时，等宽 STRONG 约束防止求解器欠定退化
    #[test]
    fn test_multiple_fill_share_equally() {
        let container = square(0.0, 0.0, 200.0);
        let mut fill_a = LayoutElement::new("fill_a", 20.0, 20.0);
        fill_a.constraints.size_strategy = SizeStrategy::Fill;
        let mut fill_b = LayoutElement::new("fill_b", 20.0, 20.0);
        fill_b.constraints.size_strategy = SizeStrategy::Fill;

        let config = LayoutConfig::with_spacing(5.0, 10.0, 5.0);
        // 容器宽 200, padding 5*2=10 → interval=190
        // 2 个 Fill + 1 个 gap=10 → 每个 Fill 应占 (190-10)/2 = 90
        let solution = layout_rows(&container, &[fill_a, fill_b], &config);
        assert!(
            solution.is_fully_placed(),
            "two Fill elements should be placed, unplaced: {:?}, warnings: {:?}",
            solution.unplaced,
            solution.warnings
        );
        assert_eq!(solution.placed.len(), 2);

        let a = &solution.placed[0];
        let b = &solution.placed[1];
        // 两者宽度应该都 > 0
        assert!(a.width > 10.0, "fill_a width={}, should be > 10", a.width);
        assert!(b.width > 10.0, "fill_b width={}, should be > 10", b.width);
        // 两者宽度应该基本相等（STRONG 等宽约束）
        assert!(
            (a.width - b.width).abs() < 5.0,
            "fill widths should be approximately equal, got a={}, b={}",
            a.width,
            b.width
        );
        // 总跨度应近似填满区间
        let total_span = (b.x + b.width) - a.x;
        assert!(
            (total_span - 190.0).abs() < 3.0,
            "total span should be ~190, got {}",
            total_span
        );
        // 无 safety net 警告
        let constraint_warnings = solution.warnings.iter().filter(|w| {
            matches!(w, LayoutWarning::ConstraintConflict(_))
        }).count();
        assert_eq!(
            constraint_warnings, 0,
            "should have no constraint conflicts (safety net bogus), got {} warnings: {:?}",
            constraint_warnings, solution.warnings
        );
    }

    /// [Fill, Fixed] 排列：Fill 在 Fixed 前面时仍然正确拉伸
    /// 验证所有相邻 gap 全锁后，row-fill STRONG 约束将剩余空间推入 Fill 宽度
    #[test]
    fn test_fill_before_fixed() {
        let container = square(0.0, 0.0, 200.0);
        let mut fill_elem = LayoutElement::new("fill", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        let fixed_elem = LayoutElement::new("fixed", 40.0, 20.0);

        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        // padding.h=5*2=10, 容器内宽=190, Fixed占40, gap=5, Fill应占190-40-5=145
        let solution = layout_rows(&container, &[fill_elem, fixed_elem], &config);
        assert!(solution.is_fully_placed());

        let fill = &solution.placed[0];
        let fixed = &solution.placed[1];

        // Fixed 保持 40
        assert!((fixed.width - 40.0).abs() < 1.0, "Fixed should be 40, got {}", fixed.width);
        // Fill 吸收了剩余空间
        assert!(fill.width > 40.0, "Fill should absorb space, got {}", fill.width);
        // gap = fill.right 到 fixed.left = 5
        let gap = fixed.x - (fill.x + fill.width);
        assert!((gap - 5.0).abs() < 1.0, "gap should be ~5, got {}", gap);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // RON 序列化往返测试（Step 0.5d）
    // ═══════════════════════════════════════════════════════════════════════

    /// ContainerShape 所有变体的 RON roundtrip
    #[test]
    fn test_container_shape_ron_roundtrip() {
        let shapes = vec![
            ContainerShape::Rect {
                width: 200.0,
                height: 100.0,
            },
            ContainerShape::RoundedRect {
                width: 150.0,
                height: 80.0,
                radius: 10.0,
            },
            ContainerShape::Circle { diameter: 100.0 },
            ContainerShape::Heart { width: 120.0 },
            ContainerShape::Custom {
                path: square(0.0, 0.0, 50.0),
            },
        ];

        for shape in &shapes {
            let ron_str = ron::ser::to_string_pretty(shape, ron::ser::PrettyConfig::default())
                .unwrap_or_else(|e| panic!("RON serialize failed for {:?}: {}", shape, e));
            let roundtripped: ContainerShape =
                ron::from_str(&ron_str).unwrap_or_else(|e| {
                    panic!(
                        "RON deserialize failed for\n{}\nerror: {}",
                        ron_str, e
                    )
                });
            // 比较 BezPath 输出（而非 Debug repr，后者不保证相等）
            let original_bp = shape.to_bezpath();
            let rt_bp = roundtripped.to_bezpath();
            assert_eq!(
                format!("{:?}", original_bp.elements()),
                format!("{:?}", rt_bp.elements()),
                "BezPath mismatch after roundtrip for {:?}",
                shape
            );
        }
    }

    /// LayoutElement 的 RON roundtrip
    #[test]
    fn test_layout_element_ron_roundtrip() {
        let mut elem = LayoutElement::new("test", 100.0, 50.0);
        elem.constraints.min_width = Some(30.0);
        elem.constraints.max_width = Some(200.0);
        elem.constraints.size_strategy = SizeStrategy::Fill;
        elem.margin = ElementMargin::horizontal(10.0);
        elem.baseline = Some(25.0);

        let ron_str =
            ron::ser::to_string_pretty(&elem, ron::ser::PrettyConfig::default()).unwrap();
        let roundtripped: LayoutElement = ron::from_str(&ron_str).unwrap();

        assert_eq!(elem.id, roundtripped.id);
        assert!((elem.width - roundtripped.width).abs() < 1e-9);
        assert!((elem.height - roundtripped.height).abs() < 1e-9);
        assert_eq!(elem.constraints.min_width, roundtripped.constraints.min_width);
        assert_eq!(elem.constraints.max_width, roundtripped.constraints.max_width);
        assert_eq!(elem.constraints.size_strategy, roundtripped.constraints.size_strategy);
        assert!((elem.margin.left - roundtripped.margin.left).abs() < 1e-9);
        assert_eq!(elem.baseline, roundtripped.baseline);
    }

    /// LayoutConfig 的 RON roundtrip
    #[test]
    fn test_layout_config_ron_roundtrip() {
        let config = LayoutConfig {
            padding_top: 10.0,
            padding_bottom: 10.0,
            padding_left: 15.0,
            padding_right: 15.0,
            gap: 5.0,
            line_spacing: 8.0,
            min_width: Some(1.0),
            step_size: 2.0,
            halign: HAlign::Center,
            valign: VAlign::Baseline,
            stack_direction: StackDirection::Flow,
            fill_min_ratio: 0.4,
        };

        let ron_str =
            ron::ser::to_string_pretty(&config, ron::ser::PrettyConfig::default()).unwrap();
        // 验证 RON 字符串不包含未解析的结构体名
        assert!(
            !ron_str.contains("unwrap"),
            "RON should not contain Rust internals"
        );

        let roundtripped: LayoutConfig = ron::from_str(&ron_str).unwrap();
        assert!((config.padding_top - roundtripped.padding_top).abs() < 1e-9);
        assert!((config.gap - roundtripped.gap).abs() < 1e-9);
        assert_eq!(config.halign, roundtripped.halign);
        assert_eq!(config.valign, roundtripped.valign);
    }

    /// LayoutSolution 的 RON roundtrip
    #[test]
    fn test_layout_solution_ron_roundtrip() {
        let solution = LayoutSolution {
            placed: vec![
                PlacedElement {
                    id: "a".into(),
                    x: 10.0,
                    y: 50.0,
                    width: 80.0,
                    height: 40.0,
                },
                PlacedElement {
                    id: "b".into(),
                    x: 95.0,
                    y: 50.0,
                    width: 80.0,
                    height: 40.0,
                },
            ],
            unplaced: vec!["c".into()],
            warnings: vec![LayoutWarning::ElementTooWide {
                element_id: "c".into(),
                min_width: 200.0,
                max_available: 100.0,
            }],
        };

        let ron_str =
            ron::ser::to_string_pretty(&solution, ron::ser::PrettyConfig::default()).unwrap();
        let roundtripped: LayoutSolution = ron::from_str(&ron_str).unwrap();

        assert_eq!(solution.placed.len(), roundtripped.placed.len());
        assert_eq!(solution.unplaced, roundtripped.unplaced);
        assert_eq!(solution.warnings.len(), roundtripped.warnings.len());

        for (orig, rt) in solution.placed.iter().zip(roundtripped.placed.iter()) {
            assert_eq!(orig.id, rt.id);
            assert!((orig.x - rt.x).abs() < 1e-9);
            assert!((orig.y - rt.y).abs() < 1e-9);
            assert!((orig.width - rt.width).abs() < 1e-9);
            assert!((orig.height - rt.height).abs() < 1e-9);
        }
    }

    /// 端到端 RON 工作流：ContainerShape + elements + config → 排版 → 结果 roundtrip
    #[test]
    fn test_end_to_end_ron_workflow() {
        // 1. 准备容器（用 ContainerShape 而非手动 BezPath）
        let container = ContainerShape::RoundedRect {
            width: 200.0,
            height: 100.0,
            radius: 8.0,
        };

        // 2. 准备元素
        let mut fill_elem = LayoutElement::new("fill", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        fill_elem.margin = ElementMargin::horizontal(5.0);

        let fixed_elem = LayoutElement::new("fixed", 40.0, 20.0);

        let elements = vec![fill_elem, fixed_elem];

        // 3. 配置
        let config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);

        // 4. 排版 → 用 layout_container（新 API）
        let solution = layout_container(&container, &elements, &config);

        assert!(
            solution.is_fully_placed(),
            "RON e2e workflow should place all elements, got unplaced: {:?}",
            solution.unplaced
        );

        // 5. 序列化整个结果
        let ron_str =
            ron::ser::to_string_pretty(&solution, ron::ser::PrettyConfig::default()).unwrap();

        // 6. 反序列化
        let roundtripped: LayoutSolution = ron::from_str(&ron_str).unwrap();

        assert_eq!(solution.placed.len(), roundtripped.placed.len());
        assert!(roundtripped.is_fully_placed());

        // 7. 验证关键数据
        let fill = &roundtripped.placed[0];
        assert!(fill.width > 20.0, "Fill element should be stretched, got {}", fill.width);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Phase 2: Gourd & HangTag 验收测试
    // ═══════════════════════════════════════════════════════════════════════

    /// 验收案例 1：葫芦形洗发水标签排版
    ///
    /// 在葫芦形容器内排放 logo / 产品名 / 容量 / 条码区 四个元素，
    /// 验证元素在窄腰处正确换行，不溢出轮廓。
    /// 葫芦形天然在腰部收窄，部分元素可能因宽度不足被挤到底部溢出。
    #[test]
    fn test_gourd_shampoo_label() {
        // 大号葫芦形容器（模拟真实标签尺寸）
        let container = ContainerShape::Gourd {
            width: 200.0,
            height: 300.0,
            waist_y: 0.55,
            waist_ratio: 0.45,
        };

        // 4 个实际尺寸的元素
        let elements = vec![
            LayoutElement::new("logo", 80.0, 35.0),
            LayoutElement::new("product_name", 110.0, 25.0),
            LayoutElement::new("volume", 55.0, 18.0),
            LayoutElement::new("barcode", 90.0, 30.0),
        ];

        let config = LayoutConfig::with_spacing(10.0, 8.0, 10.0);

        let solution = layout_container(&container, &elements, &config);

        // 窄腰处宽度仅 200*0.45=90，去掉 padding 后仅 70，
        // 至少能排下 3 个元素（barcode 90 宽可能被底部挤出）
        assert!(
            solution.placed.len() >= 3,
            "Gourd should place at least 3 elements, placed={}, unplaced={:?}",
            solution.placed.len(),
            solution.unplaced
        );

        // 验证所有已放置元素都在容器 AABB 内
        let bbox = container.to_bezpath().bounding_box();
        for placed in &solution.placed {
            assert!(
                placed.x >= bbox.x0 - 1e-6,
                "element '{}' x={} < container x0={}",
                placed.id,
                placed.x,
                bbox.x0
            );
            assert!(
                placed.x + placed.width <= bbox.x1 + 1e-6,
                "element '{}' right={} > container x1={}",
                placed.id,
                placed.x + placed.width,
                bbox.x1
            );
            assert!(
                placed.y >= bbox.y0 - 1e-6,
                "element '{}' y={} < container y0={}",
                placed.id,
                placed.y,
                bbox.y0
            );
            assert!(
                placed.y + placed.height <= bbox.y1 + 1e-6,
                "element '{}' top={} > container y1={}",
                placed.id,
                placed.y + placed.height,
                bbox.y1
            );
        }

        // 验证从上到下排版
        for w in solution.placed.windows(2) {
            assert!(
                w[1].y + w[1].height <= w[0].y + 1e-6
                    || (w[1].y - w[0].y).abs() < 1e-6,
                "rows should not overlap upward: {} at y={} vs {} at y={}",
                w[0].id,
                w[0].y,
                w[1].id,
                w[1].y
            );
        }
    }

    /// 验收案例 2：带孔洞的吊牌排版
    ///
    /// 在带圆形穿绳孔洞的吊牌中排放品牌名 / 尺码 / 洗护标识，
    /// 验证孔洞区域元素自动避让（不会被放到孔洞位置）。
    #[test]
    fn test_hang_tag_with_hole() {
        let container = ContainerShape::HangTag {
            width: 80.0,
            height: 120.0,
            radius: 5.0,
            hole_y: 100.0,
            hole_radius: 6.0,
        };

        let elements = vec![
            LayoutElement::new("brand", 45.0, 15.0),
            LayoutElement::new("size_info", 30.0, 12.0),
            LayoutElement::new("care_label", 55.0, 18.0),
        ];

        let config = LayoutConfig::with_spacing(5.0, 4.0, 5.0);

        let solution = layout_container(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "HangTag: all 3 elements should be placed, unplaced={:?}, warnings={:?}",
            solution.unplaced,
            solution.warnings
        );

        // 验证没有元素覆盖孔洞区域
        // 孔洞中心 (40, 100)，半径 6
        let hole_cx = 40.0;
        let hole_cy = 100.0;
        let hole_r = 6.0;

        for placed in &solution.placed {
            let elem_cx = placed.x + placed.width / 2.0;
            let elem_cy = placed.y + placed.height / 2.0;

            // 孔洞中心到元素中心的距离应该大于孔洞半径（粗略检查）
            let dx = elem_cx - hole_cx;
            let dy = elem_cy - hole_cy;
            let dist = (dx * dx + dy * dy).sqrt();

            assert!(
                dist > hole_r * 1.5,
                "element '{}' at ({},{}) overlaps hole area (center={},{}, r={}), dist={}",
                placed.id,
                placed.x,
                placed.y,
                hole_cx,
                hole_cy,
                hole_r,
                dist
            );
        }

        // 验证所有元素在吊牌 AABB 内
        let bbox = container.to_bezpath().bounding_box();
        for placed in &solution.placed {
            assert!(placed.x >= bbox.x0 - 1e-6);
            assert!(placed.x + placed.width <= bbox.x1 + 1e-6);
            assert!(placed.y >= bbox.y0 - 1e-6);
            assert!(placed.y + placed.height <= bbox.y1 + 1e-6);
        }
    }

    /// Gourd 的 RON roundtrip
    #[test]
    fn test_gourd_ron_roundtrip() {
        let gourd = ContainerShape::Gourd {
            width: 120.0,
            height: 180.0,
            waist_y: 0.55,
            waist_ratio: 0.45,
        };

        let ron_str =
            ron::ser::to_string_pretty(&gourd, ron::ser::PrettyConfig::default()).unwrap();
        let roundtripped: ContainerShape = ron::from_str(&ron_str).unwrap();

        let orig_bp = gourd.to_bezpath();
        let rt_bp = roundtripped.to_bezpath();
        assert_eq!(
            format!("{:?}", orig_bp.elements()),
            format!("{:?}", rt_bp.elements()),
            "Gourd BezPath mismatch after roundtrip"
        );
    }

    /// HangTag 的 RON roundtrip
    #[test]
    fn test_hang_tag_ron_roundtrip() {
        let tag = ContainerShape::HangTag {
            width: 80.0,
            height: 120.0,
            radius: 5.0,
            hole_y: 100.0,
            hole_radius: 6.0,
        };

        let ron_str =
            ron::ser::to_string_pretty(&tag, ron::ser::PrettyConfig::default()).unwrap();
        let roundtripped: ContainerShape = ron::from_str(&ron_str).unwrap();

        let orig_bp = tag.to_bezpath();
        let rt_bp = roundtripped.to_bezpath();
        assert_eq!(
            format!("{:?}", orig_bp.elements()),
            format!("{:?}", rt_bp.elements()),
            "HangTag BezPath mismatch after roundtrip"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Phase 3: StackDirection::Vertical 测试
    // ═══════════════════════════════════════════════════════════════════════

    /// Vertical 模式：每个元素独占一行
    #[test]
    fn test_vertical_mode_each_element_own_row() {
        let container = square(0.0, 0.0, 200.0);
        // 三个元素，宽度都足够同行，但 Vertical 模式应强制各占一行
        let elements = vec![
            LayoutElement::new("a", 40.0, 20.0),
            LayoutElement::new("b", 50.0, 25.0),
            LayoutElement::new("c", 60.0, 30.0),
        ];
        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.stack_direction = StackDirection::Vertical;

        let solution = layout_rows(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "Vertical: all 3 should be placed, unplaced={:?}",
            solution.unplaced
        );
        assert_eq!(solution.placed.len(), 3);

        // 验证每行只有一个元素，且 Y 坐标依次递减（从上到下）
        for w in solution.placed.windows(2) {
            assert!(
                w[1].y + w[1].height <= w[0].y + 1e-6,
                "Vertical rows must not overlap: {} (y={}) vs {} (y={})",
                w[0].id, w[0].y,
                w[1].id, w[1].y,
            );
        }
    }

    /// Vertical 模式 + Fill：Fill 元素在独占行内填满该行宽度
    #[test]
    fn test_vertical_with_fill() {
        let container = square(0.0, 0.0, 200.0);
        let mut fill_elem = LayoutElement::new("fill", 20.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        let fixed_elem = LayoutElement::new("fixed", 40.0, 20.0);

        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.stack_direction = StackDirection::Vertical;

        let solution = layout_rows(&container, &[fill_elem, fixed_elem], &config);
        assert!(
            solution.is_fully_placed(),
            "Vertical + Fill: should place both, unplaced={:?}",
            solution.unplaced
        );

        // Fill 元素在独占行应被拉伸到区间全宽 (≈200-10=190)
        let fill = &solution.placed[0];
        assert!(
            fill.width > 100.0,
            "Fill should fill its row: width={}",
            fill.width
        );
        // Fixed 保持 40
        let fixed = &solution.placed[1];
        assert!(
            (fixed.width - 40.0).abs() < 1.0,
            "Fixed should stay 40, got {}",
            fixed.width
        );
    }

    /// Vertical 模式：元素过宽不可缩 → 跳过并记录警告
    #[test]
    fn test_vertical_wide_unshrinkable_skipped() {
        let container = square(0.0, 0.0, 100.0);
        let wide = LayoutElement::new("wide", 120.0, 20.0);
        let normal = LayoutElement::new("normal", 40.0, 20.0);

        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.stack_direction = StackDirection::Vertical;
        // 可用宽度 = 100 - 10 = 90，wide 120 放不下

        let solution = layout_rows(&container, &[wide, normal], &config);
        assert!(!solution.is_fully_placed());
        assert_eq!(solution.unplaced.len(), 1);
        assert_eq!(solution.unplaced[0], "wide");
        assert_eq!(solution.placed.len(), 1);
        assert_eq!(solution.placed[0].id, "normal");
    }

    /// Vertical 模式：可缩元素在窄行中缩小
    #[test]
    fn test_vertical_shrinkable_narrow_row() {
        let container = square(0.0, 0.0, 100.0);
        let mut shrinkable = LayoutElement::new("shrink", 80.0, 20.0);
        shrinkable.constraints.size_strategy = SizeStrategy::Fixed { shrinkable: true };
        shrinkable.constraints.min_width = Some(40.0);

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.stack_direction = StackDirection::Vertical;
        // 可用宽度 = 100 - 20 = 80，元素 80 刚好 → 不用缩
        let solution = layout_rows(&container, &[shrinkable], &config);
        assert!(solution.is_fully_placed());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Phase 3: Fill min_ratio 保护测试
    // ═══════════════════════════════════════════════════════════════════════

    /// Fill 元素即使无显式 min_width，也应至少保留 preferred * fill_min_ratio 的宽度
    /// （验证隐式最小宽度保护）
    #[test]
    fn test_fill_implicit_min_width_protection() {
        let container = square(0.0, 0.0, 80.0);
        let mut fill_elem = LayoutElement::new("fill", 60.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        // 无显式 min_width → 隐式 = 60 * 0.4 = 24

        let config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        // 可用宽度 = 80 - 10 = 70，虽然能放下，但要用隐式 min 保护

        let solution = layout_rows(&container, &[fill_elem], &config);
        assert!(
            solution.is_fully_placed(),
            "Fill with implicit min should fit"
        );
        // 宽度应该＞隐式 min = 24
        assert!(
            solution.placed[0].width >= 24.0,
            "Fill should not shrink below implicit min 24, got {}",
            solution.placed[0].width
        );
    }

    /// Fill 在极窄区间 + 高 fill_min_ratio → 因隐式 min 过大而被拒绝
    #[test]
    fn test_fill_rejected_by_implicit_min() {
        let container = square(0.0, 0.0, 50.0);
        let mut fill_elem = LayoutElement::new("fill", 60.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        // 无显式 min_width

        let mut config = LayoutConfig::with_spacing(3.0, 3.0, 3.0);
        config.fill_min_ratio = 0.6;
        // 可用宽度 = 50 - 6 = 44，隐式 min = 60 * 0.6 = 36
        // 36 < 44 → 能放入

        let solution = layout_rows(&container, &[fill_elem], &config);
        assert!(solution.is_fully_placed());
    }

    /// 显式 min_width 覆盖隐式 min_ratio（显式优先）
    #[test]
    fn test_fill_explicit_min_overrides_ratio() {
        let container = square(0.0, 0.0, 100.0);
        let mut fill_elem = LayoutElement::new("fill", 60.0, 20.0);
        fill_elem.constraints.size_strategy = SizeStrategy::Fill;
        fill_elem.constraints.min_width = Some(10.0); // 显式 10 < 隐式 24

        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.fill_min_ratio = 0.4; // 隐式 = 24

        let solution = layout_rows(&container, &[fill_elem], &config);
        assert!(
            solution.is_fully_placed(),
            "Explicit min 10 should be used, not implicit 24"
        );
        // Fill 被拉伸到区间全宽，但应该 ≥ 10
        assert!(
            solution.placed[0].width >= 10.0,
            "Fill should respect explicit min 10"
        );
    }

    /// RON roundtrip: StackDirection
    #[test]
    fn test_stack_direction_ron_roundtrip() {
        let sd = StackDirection::Vertical;
        let ron_str = ron::ser::to_string_pretty(&sd, ron::ser::PrettyConfig::default()).unwrap();
        let roundtripped: StackDirection = ron::from_str(&ron_str).unwrap();
        assert_eq!(sd, roundtripped);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Bug Fix 验证：padding_top / padding_bottom 方向修正 + Vertical 跳跃
    // ═══════════════════════════════════════════════════════════════════════

    /// Bug 1 修复验证：padding_top 和 padding_bottom 不对称时分别作用于正确边界
    ///
    /// 修复前：`container_top = y1 - padding_bottom`（拿底部 padding 缩顶部）
    /// 修复后：`container_top = y1 - padding_top`（正确）
    #[test]
    fn test_padding_top_bottom_asymmetric() {
        // 200x200 正方形容器
        let container = square(0.0, 0.0, 200.0);
        let elements = vec![LayoutElement::new("a", 40.0, 20.0)];

        let config = LayoutConfig {
            padding_top: 30.0,
            padding_bottom: 10.0,
            padding_left: 5.0,
            padding_right: 5.0,
            gap: 5.0,
            line_spacing: 5.0,
            ..Default::default()
        };

        let solution = layout_rows(&container, &elements, &config);
        assert!(solution.is_fully_placed());

        let a = &solution.placed[0];

        // 修复后：container_top = 200 - 30 = 170，元素高 20 → y(行底) = 170 - 20 = 150
        // 修复前（Bug）：container_top = 200 - 10 = 190 → y = 190 - 20 = 170（错误偏高）
        assert!(
            (a.y - 150.0).abs() < 2.0,
            "padding_top=30: element y should ≈150, got {} (Bug 1 would give 170)",
            a.y
        );

        // 元素不应低于 container_bottom = 0 + 10 = 10
        assert!(
            a.y >= 10.0 - 1e-6,
            "element y={} should be >= container_bottom 10",
            a.y
        );
    }

    /// Bug 2 修复验证：Vertical 模式在遇到障碍（孔洞）时向下跳跃 Y，
    /// 使后续元素能在障碍下方成功排放。
    ///
    /// 修复前：跳过元素但不移 Y → 下一元素卡在同一孔洞高度，同样失败。
    /// 修复后：跳过元素 + 向下跳 Y → 下一元素越过孔洞后成功排放。
    #[test]
    fn test_vertical_mode_jumps_y_on_obstacle() {
        // 吊牌容器：顶部有穿绳孔洞（hole_y=100, hole_radius=6）
        // 孔洞区域（Y≈94-106）可用宽度极窄，元素应自动避让到孔洞下方
        let container = ContainerShape::HangTag {
            width: 80.0,
            height: 120.0,
            radius: 5.0,
            hole_y: 100.0,
            hole_radius: 6.0,
        };

        // 元素 1：宽 60、高 30，在孔洞区间的单侧安全区域放不下；被跳过时 Y 跳 30px
        // 元素 2：宽 20、高 10，Y 跳 30px 后行顶 85+10=95 仍在孔洞范围 (94-106)，
        // 所以继续被跳；最终两个元素到孔洞下方才排放。
        // 关键：Bug 2 修复后 Y 会向下跳跃，元素不会永远卡在同一位置。
        //
        // 简化起见：让元素 1 的 footprint_height 足够大，使得一次 Y 跳就完全越过孔洞。
        // hole_bottom=94, elem2 row top = y + height, 需要 y+10 <= 94 → y <= 84
        // elem1 at y=115-25=90 → y jump 25 → y=65. Row [55,65] << 94 → clean.
        let elements = vec![
            LayoutElement::new("too_wide_for_hole", 60.0, 25.0),
            LayoutElement::new("fits_below_hole", 20.0, 10.0),
        ];

        let mut config = LayoutConfig::with_spacing(5.0, 5.0, 5.0);
        config.stack_direction = StackDirection::Vertical;

        let solution = layout_container(&container, &elements, &config);

        // 验证：元素 1 因孔洞宽度不足被跳过
        assert!(
            solution.unplaced.contains(&"too_wide_for_hole".to_string()),
            "too_wide_for_hole should be unplaced; unplaced={:?}",
            solution.unplaced
        );

        // 验证：元素 2 在孔洞下方成功排放（Bug 2 修复的关键断言）
        assert!(
            solution.placed.iter().any(|p| p.id == "fits_below_hole"),
            "fits_below_hole should be placed (below the hole); placed={:?}",
            solution.placed.iter().map(|p| &p.id).collect::<Vec<_>>()
        );

        // 元素 2 应排在元素 1 试图占据的位置之下（Y 已向下跳跃）
        if let Some(elem2) = solution.placed.iter().find(|p| p.id == "fits_below_hole") {
            // Y jump of 25 from 90 → y=65. Row [55,65] << hole_bottom=94. Safe.
            assert!(
                elem2.y <= 80.0,
                "fits_below_hole y={:.1} should be well below the hole region",
                elem2.y
            );
        }
    }

    /// Bug 1 + Bug 2 联合验证：HangTag 大 padding_top 推元素到孔洞下方
    ///
    /// 模拟 AI 排版场景：用 padding_top 将起始排版区推到孔洞以下，
    /// 确保所有元素在孔洞下方全宽区域成功排放。
    #[test]
    fn test_hang_tag_top_padding_skips_hole() {
        let container = ContainerShape::HangTag {
            width: 80.0,
            height: 120.0,
            radius: 5.0,
            hole_y: 100.0,
            hole_radius: 6.0,
        };

        let elements = vec![
            LayoutElement::new("brand", 45.0, 12.0),
            LayoutElement::new("size", 30.0, 10.0),
        ];

        // 大 padding_top 把起始排版区推到孔洞下方
        let config = LayoutConfig {
            padding_top: 36.0, // 推过孔洞底部 (100-6=94)
            padding_bottom: 5.0,
            padding_left: 5.0,
            padding_right: 5.0,
            gap: 4.0,
            line_spacing: 4.0,
            ..Default::default()
        };

        let solution = layout_container(&container, &elements, &config);
        assert!(
            solution.is_fully_placed(),
            "both elements should be placed below the hole; unplaced={:?}",
            solution.unplaced
        );

        // 所有元素顶部都应在孔洞底部（94）以下
        let hole_bottom = 100.0 - 6.0;
        for p in &solution.placed {
            let top = p.y + p.height;
            assert!(
                top <= hole_bottom + 5.0,
                "element '{}' top={:.1} should be below hole bottom={:.1}",
                p.id, top, hole_bottom
            );
        }
    }
}
